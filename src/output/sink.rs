//! Frame sink interface and shared output types.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::error::OutputError;

/// Internal stream identity, independent of sink-specific destination addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StreamId(u64);

impl StreamId {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for StreamId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "s{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PixelFormat {
    Rgb888,
    Rgb565Le,
    Rgb565Be,
}

impl PixelFormat {
    pub fn from_str_canon(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            // ddp-esphome converts RGB input for its RGBW renderer.
            "rgb888" | "rgbw" => Some(Self::Rgb888),
            "rgb565le" => Some(Self::Rgb565Le),
            "rgb565be" => Some(Self::Rgb565Be),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FrameMeta {
    pub sequence: u32,
    pub delay_ms: f32,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub is_still: bool,
    pub is_last_frame: bool,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub data: Bytes,
    pub meta: FrameMeta,
}

/// Frame destination for a stream whose address reservation is already held.
#[async_trait::async_trait]
pub trait OutputSink: Send + Sync {
    async fn send_frame(&self, frame: Frame) -> Result<(), OutputError>;

    /// Requests sink cleanup. Transport ownership controls socket lifetime.
    async fn close(&self) -> Result<(), OutputError>;
}
