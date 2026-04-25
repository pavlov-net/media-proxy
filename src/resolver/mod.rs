//! Upstream URL resolver — talks to the addon sidecar over HTTP.
//!
//! Two implementations behind the trait: a real HTTP client and a `Fake` for
//! tests. yt-dlp itself lives in the addon, not here.

pub mod fake;
pub mod http;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ResolverError;

#[derive(Debug, Clone, Serialize)]
pub struct ResolveRequest {
    pub url: String,
    pub target_h: u32,
    pub target_w: u32,
    pub hw_prefer: Option<String>,
    pub prefer_60fps: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveResponse {
    pub stream_url: String,
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub fps: Option<f32>,
    #[serde(default)]
    pub codec: Option<String>,
    /// Size in bytes, if known.
    #[serde(default)]
    pub filesize: Option<u64>,
    /// Unix timestamp; caller should re-resolve after this.
    #[serde(default)]
    pub expires_at: Option<i64>,
}

impl ResolveResponse {
    /// Should the ffmpeg `cache:` protocol wrap this URL? Enabled for
    /// looping playback of small files (typical YouTube short clips), where
    /// fetching the same bytes per loop is wasteful.
    pub fn should_cache(&self, r#loop: bool, max_bytes: u64) -> bool {
        r#loop && self.filesize.is_some_and(|sz| sz <= max_bytes)
    }
}

#[async_trait]
pub trait Resolver: Send + Sync {
    /// Resolve `url`. If the URL doesn't need resolution (direct file://
    /// or HTTP media URL), implementations return a passthrough
    /// `ResolveResponse`.
    async fn resolve(&self, req: ResolveRequest) -> Result<ResolveResponse, ResolverError>;
}

pub use fake::FakeResolver;
pub use http::HttpResolver;
