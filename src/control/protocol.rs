//! Wire-level control protocol — serde types for the JSON messages.
//! Message shapes: `hello`, `start_stream`, `stop_stream`, `update`, `ping`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::control::fields::{AppliedParams, StartStream, StopStream, UpdateStream};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        #[serde(default)]
        device_id: Option<String>,
    },
    StartStream(StartStream),
    StopStream(StopStream),
    Update(UpdateStream),
    Ping {
        t: Option<i64>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    HelloAck {
        server_version: String,
    },
    Ack {
        out: i32,
        #[serde(skip_serializing_if = "Option::is_none")]
        applied: Option<AppliedParams>,
    },
    Pong {
        t: Option<i64>,
    },
    Error(ErrorMsg),
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorMsg {
    pub code: String,
    pub message: String,
}

/// Free-form start/update payloads come in with arbitrary extra keys (future
/// fields, per-client experiments). `StartStream`/`UpdateStream` capture the
/// known fields but we also keep the raw JSON for cases that need it.
pub fn parse_client_msg(raw: &str) -> serde_json::Result<ClientMsg> {
    serde_json::from_str(raw)
}

pub fn serialize_server_msg(msg: &ServerMsg) -> String {
    serde_json::to_string(msg).unwrap_or_else(|e| {
        // Can't encode a JSON error payload as JSON? At this point the best
        // we can do is send a minimal inline error literal.
        format!(r#"{{"type":"error","code":"server_error","message":"{}"}}"#, e)
    })
}

/// Retained for future compatibility checks: `start_stream` responses echo
/// back the `applied` map, which surfaces server-resolved values (`hw=auto`
/// → concrete backend, etc.).
#[allow(dead_code)]
pub type RawMap = serde_json::Map<String, Value>;
