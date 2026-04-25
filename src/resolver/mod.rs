//! Resolver layer — translates an arbitrary input URL into a directly
//! streamable URL plus any HTTP headers the upstream CDN expects.
//!
//! Implementations:
//! - [`SubprocessResolver`] — local `yt-dlp` subprocess (default when on PATH)
//! - [`HttpResolver`] — POST to an external resolver sidecar
//! - [`NoopResolver`] — fail-closed for anything not already direct
//! - [`FakeResolver`] — for tests
//!
//! All concrete impls are wrapped by [`PassthroughLayer`], which short-circuits
//! direct-media URLs (file://, *.mp4, …) without calling through to the inner
//! resolver. So every impl below only sees URLs that genuinely need extracting.

pub mod fake;
pub mod http;
pub mod noop;
pub mod passthrough;
pub mod subprocess;

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
    pub fn passthrough(url: String) -> Self {
        Self {
            stream_url: url,
            headers: Default::default(),
            fps: None,
            codec: None,
            filesize: None,
            expires_at: None,
        }
    }

    /// Should the ffmpeg `cache:` protocol wrap this URL? Enabled for
    /// looping playback of small files (typical YouTube short clips), where
    /// fetching the same bytes per loop is wasteful.
    pub fn should_cache(&self, r#loop: bool, max_bytes: u64) -> bool {
        r#loop && self.filesize.is_some_and(|sz| sz <= max_bytes)
    }
}

#[async_trait]
pub trait Resolver: Send + Sync {
    /// Resolve `url`. Direct-media URLs are short-circuited by
    /// [`passthrough::PassthroughLayer`] before reaching impls; impls only
    /// need to handle URLs that genuinely require extraction.
    async fn resolve(&self, req: ResolveRequest) -> Result<ResolveResponse, ResolverError>;
}

pub use fake::FakeResolver;
pub use http::HttpResolver;
pub use noop::NoopResolver;
pub use passthrough::PassthroughLayer;
pub use subprocess::SubprocessResolver;
