//! Session-owned stream parameters and cancellation notifications.

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

    /// Returns the original resolved parameters used as the base for partial updates.
    pub fn fields(&self) -> &StreamFields {
        &self.fields
    }

    /// Notifies current cancellation waiters.
    pub fn cancel(&self) {
        self.cancel.notify_waiters();
    }

    /// Waits for the next cancellation notification.
    pub async fn cancelled(&self) {
        self.cancel.notified().await;
    }
}
