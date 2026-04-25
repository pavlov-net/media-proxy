//! DDP collision registry — actor task owning `HashMap<DdpKey, StreamId>`.
//!
//! The invariant: at most one active DDP stream per `(dest_ip, output_id)`.
//! When a new `start_stream` arrives for an already-taken key, the prior
//! stream is cancelled (via its `cancel_tx`) before the new reservation
//! completes.

use std::collections::HashMap;
use std::net::IpAddr;

use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info};

use crate::error::OutputError;
use crate::output::sink::StreamId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DdpKey {
    pub dest: IpAddr,
    pub output_id: u8,
}

impl std::fmt::Display for DdpKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.dest, self.output_id)
    }
}

/// A reservation for a DDP key. Dropping it asks the registry to release
/// the key (the registry also treats a closed cancel channel as "gone").
pub struct DdpReservation {
    key: DdpKey,
    stream_id: StreamId,
    release_tx: mpsc::UnboundedSender<RegistryMsg>,
}

impl DdpReservation {
    pub fn key(&self) -> DdpKey {
        self.key
    }
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }
}

impl Drop for DdpReservation {
    fn drop(&mut self) {
        let _ = self.release_tx.send(RegistryMsg::Release {
            key: self.key,
            stream_id: self.stream_id,
        });
    }
}

enum RegistryMsg {
    Reserve {
        key: DdpKey,
        stream_id: StreamId,
        cancel_tx: oneshot::Sender<()>,
        reply: oneshot::Sender<()>,
    },
    Release {
        key: DdpKey,
        stream_id: StreamId,
    },
}

#[derive(Debug, Clone)]
pub struct DdpRegistry {
    tx: mpsc::UnboundedSender<RegistryMsg>,
}

impl DdpRegistry {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(rx));
        Self { tx }
    }

    /// Reserve a DDP key for the given stream. If another stream owns the key,
    /// it is cancelled before this reservation is granted.
    ///
    /// Returns `(reservation, cancel_rx)`. The stream task watches `cancel_rx`
    /// alongside its frame loop — it fires if a later reservation displaces
    /// this one.
    pub async fn reserve(
        &self,
        key: DdpKey,
        stream_id: StreamId,
    ) -> Result<(DdpReservation, oneshot::Receiver<()>), OutputError> {
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let (reply_tx, reply_rx) = oneshot::channel();

        self.tx
            .send(RegistryMsg::Reserve {
                key,
                stream_id,
                cancel_tx,
                reply: reply_tx,
            })
            .map_err(|_| OutputError::Sink("registry task died".into()))?;

        reply_rx
            .await
            .map_err(|_| OutputError::Sink("registry reply channel closed".into()))?;

        Ok((
            DdpReservation {
                key,
                stream_id,
                release_tx: self.tx.clone(),
            },
            cancel_rx,
        ))
    }
}

async fn run(mut rx: mpsc::UnboundedReceiver<RegistryMsg>) {
    let mut occupied: HashMap<DdpKey, (StreamId, oneshot::Sender<()>)> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        match msg {
            RegistryMsg::Reserve {
                key,
                stream_id,
                cancel_tx,
                reply,
            } => {
                if let Some((old_id, old_cancel)) = occupied.remove(&key) {
                    info!(%key, %old_id, %stream_id, "displacing conflicting DDP stream");
                    let _ = old_cancel.send(());
                }
                occupied.insert(key, (stream_id, cancel_tx));
                let _ = reply.send(());
                debug!(%key, %stream_id, "DDP reservation granted");
            }
            RegistryMsg::Release { key, stream_id } => {
                // Only release if the stream_id still matches — a displaced
                // stream may send Release after a new one has taken over.
                if matches!(occupied.get(&key), Some((sid, _)) if *sid == stream_id) {
                    occupied.remove(&key);
                    debug!(%key, %stream_id, "DDP reservation released");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ip: &str, out: u8) -> DdpKey {
        DdpKey {
            dest: ip.parse().unwrap(),
            output_id: out,
        }
    }

    #[tokio::test]
    async fn reserve_succeeds_for_new_key() {
        let reg = DdpRegistry::spawn();
        let k = key("127.0.0.1", 1);
        let (res, _cancel_rx) = reg.reserve(k, StreamId::new()).await.unwrap();
        assert_eq!(res.key(), k);
    }

    #[tokio::test]
    async fn second_reserve_cancels_first() {
        let reg = DdpRegistry::spawn();
        let k = key("127.0.0.1", 1);
        let id1 = StreamId::new();
        let id2 = StreamId::new();

        let (_res1, cancel_rx) = reg.reserve(k, id1).await.unwrap();

        let _res2 = reg.reserve(k, id2).await.unwrap();

        // The first reservation's cancel channel should fire.
        cancel_rx.await.unwrap();
    }

    #[tokio::test]
    async fn different_keys_coexist() {
        let reg = DdpRegistry::spawn();
        let _a = reg.reserve(key("127.0.0.1", 1), StreamId::new()).await.unwrap();
        let _b = reg.reserve(key("127.0.0.1", 2), StreamId::new()).await.unwrap();
        let _c = reg.reserve(key("127.0.0.2", 1), StreamId::new()).await.unwrap();
    }
}
