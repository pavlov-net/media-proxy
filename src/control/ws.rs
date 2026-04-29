//! axum WebSocket handler — wires a connection to the `Handler` dispatcher.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{
    ConnectInfo, State, WebSocketUpgrade,
    ws::{Message, Utf8Bytes, WebSocket},
};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use bytes::Bytes;
use futures::SinkExt;
use tokio::time::{MissedTickBehavior, interval, timeout};
use tracing::{debug, info};

use crate::api::AppState;
use crate::control::handler::Handler;
use crate::control::protocol::{self, ClientMsg};
use crate::control::session::Session;

/// Time the server waits for the initial `hello` before giving up.
const HELLO_TIMEOUT: Duration = Duration::from_secs(30);

/// How often the server sends a WebSocket-level PING. tungstenite/axum
/// will auto-respond to ours, and incoming traffic resets the activity
/// clock — so as long as the TCP path is up, the connection stays open.
const PING_INTERVAL: Duration = Duration::from_secs(15);

/// If no traffic at all (text, binary, ping, pong) arrives within this
/// window, the connection is considered dead. Generous enough to absorb
/// one missed ping/pong round-trip.
const IDLE_DEAD: Duration = Duration::from_secs(45);

pub async fn upgrade(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let host = crate::api::host_header(&headers).to_string();
    ws.on_upgrade(move |socket| handle_socket(socket, state, addr, host))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>, addr: SocketAddr, server_host: String) {
    let session = Session::new(addr.ip(), server_host);
    info!(client = %addr, session = %session.client_id, "ws upgraded");

    let handler = Handler {
        config: state.config.clone(),
        orch: state.orch.clone(),
    };

    // First message must be `hello`.
    match recv_with_timeout(&mut socket, HELLO_TIMEOUT).await {
        RecvOutcome::Message(Message::Text(raw)) => {
            let msg = match protocol::parse_client_msg(&raw) {
                Ok(m) => m,
                Err(e) => {
                    let _ = send_error(&mut socket, "proto", &format!("invalid hello: {e}")).await;
                    let _ = socket.close().await;
                    return;
                }
            };
            if !matches!(msg, ClientMsg::Hello { .. }) {
                let _ = send_error(&mut socket, "proto", "expect 'hello' first").await;
                let _ = socket.close().await;
                return;
            }
            let response = match handler.dispatch(&session, msg).await {
                Ok(r) => r,
                Err(e) => Handler::error_response(&e),
            };
            let _ = send_server_msg(&mut socket, &response).await;
        }
        RecvOutcome::Message(_) => {
            let _ = send_error(&mut socket, "proto", "expected text message").await;
            return;
        }
        RecvOutcome::Timeout => {
            debug!(client = %addr, "ws idle timeout before hello");
            return;
        }
        RecvOutcome::Closed => return,
    }

    // Main loop — server-driven heartbeat: send a PING every PING_INTERVAL,
    // give up if no traffic at all for IDLE_DEAD. The client doesn't have
    // to send anything (incoming PONGs reset `last_activity`).
    let mut last_activity = Instant::now();
    let mut ping_iv = interval(PING_INTERVAL);
    ping_iv.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ping_iv.tick().await; // skip the immediate first tick
    loop {
        tokio::select! {
            recv = socket.recv() => {
                last_activity = Instant::now();
                match recv {
                    Some(Ok(Message::Text(raw))) => {
                        debug!(client = %addr, "recv text");
                        let parsed = match protocol::parse_client_msg(&raw) {
                            Ok(m) => m,
                            Err(e) => {
                                let _ = send_error(&mut socket, "bad_request", &e.to_string()).await;
                                continue;
                            }
                        };
                        let response = match handler.dispatch(&session, parsed).await {
                            Ok(r) => r,
                            Err(e) => Handler::error_response(&e),
                        };
                        if send_server_msg(&mut socket, &response).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!(client = %addr, "ws close");
                        break;
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_))) => {
                        // tungstenite auto-responds to PING; PONG is just bookkeeping.
                    }
                    Some(Err(e)) => {
                        debug!(client = %addr, error = %e, "ws recv error");
                        break;
                    }
                    None => break,
                }
            }
            _ = ping_iv.tick() => {
                if last_activity.elapsed() > IDLE_DEAD {
                    debug!(client = %addr, "ws idle (no traffic)");
                    break;
                }
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    debug!(client = %addr, "ws ping send failed");
                    break;
                }
            }
        }
    }

    // Cancel any remaining streams for this session.
    let handles: Vec<_> = session.streams.lock().drain().map(|(_, h)| h).collect();
    for h in handles {
        h.cancel();
    }
    info!(client = %addr, session = %session.client_id, "ws closed");
}

enum RecvOutcome {
    Message(Message),
    Timeout,
    Closed,
}

async fn recv_with_timeout(socket: &mut WebSocket, deadline: Duration) -> RecvOutcome {
    match timeout(deadline, socket.recv()).await {
        Ok(Some(Ok(m))) => RecvOutcome::Message(m),
        Ok(Some(Err(e))) => {
            debug!(error = %e, "ws recv error");
            RecvOutcome::Closed
        }
        Ok(None) => RecvOutcome::Closed,
        Err(_) => RecvOutcome::Timeout,
    }
}

async fn send_server_msg(
    socket: &mut WebSocket,
    msg: &crate::control::protocol::ServerMsg,
) -> Result<(), axum::Error> {
    let text: Utf8Bytes = protocol::serialize_server_msg(msg).into();
    socket.send(Message::Text(text)).await
}

async fn send_error(socket: &mut WebSocket, code: &str, msg: &str) -> Result<(), axum::Error> {
    let err = crate::control::protocol::ServerMsg::Error(crate::control::protocol::ErrorMsg {
        code: code.into(),
        message: msg.into(),
    });
    send_server_msg(socket, &err).await
}

/// Classify benign disconnect errors — Windows `winerror 64/121` and
/// generic connection-reset — so we can log them at `info` instead of
/// `warn`. (Kept as a helper for future use; axum's error types don't
/// currently expose the underlying OS code.)
#[allow(dead_code)]
pub fn is_benign_disconnect(e: &axum::Error) -> bool {
    let s = e.to_string().to_ascii_lowercase();
    s.contains("reset") || s.contains("broken pipe") || s.contains("disconnected")
}
