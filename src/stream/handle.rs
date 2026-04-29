//! `StreamHandle` — the orchestrator returns this when a stream is spawned;
//! the session owns it and calls `cancel()` on stop / disconnect.

use std::sync::Arc;

use tokio::sync::Notify;

use crate::control::fields::StreamFields;
use crate::output::sink::StreamId;

#[derive(Debug, Clone)]
pub struct StreamHandle {
    stream_id: StreamId,
    cancel: Arc<Notify>,
    fields: Arc<StreamFields>,
}

impl StreamHandle {
    pub fn new(stream_id: StreamId, fields: StreamFields) -> Self {
        Self {
            stream_id,
            cancel: Arc::new(Notify::new()),
            fields: Arc::new(fields),
        }
    }

    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// The resolved fields the stream was spawned with. `update` uses these
    /// as the base for merging partial updates.
    pub fn fields(&self) -> &StreamFields {
        &self.fields
    }

    /// Signal the stream task to stop. Idempotent.
    pub fn cancel(&self) {
        self.cancel.notify_waiters();
    }

    /// Wait for cancellation. Stream task's async select arm.
    pub async fn cancelled(&self) {
        self.cancel.notified().await;
    }
}
