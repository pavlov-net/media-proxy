//! Resolves source URLs to playable media and HTTP headers. [`PassthroughLayer`]
//! bypasses extraction for recognized media; subprocess and HTTP implementations
//! handle the remaining URLs, while [`NoopResolver`] rejects them.

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
    /// URL expiry in Unix seconds; retained as metadata without scheduled refresh.
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

    /// Returns whether looping playback can cache the known file size within `max_bytes`.
    pub fn should_cache(&self, r#loop: bool, enabled: bool, max_bytes: u64) -> bool {
        enabled && r#loop && self.filesize.is_some_and(|sz| sz <= max_bytes)
    }
}

#[async_trait]
pub trait Resolver: Send + Sync {
    /// Returns playable media and headers. [`PassthroughLayer`] handles direct
    /// media before invoking an extraction implementation.
    async fn resolve(&self, req: ResolveRequest) -> Result<ResolveResponse, ResolverError>;
}

pub use fake::FakeResolver;
pub use http::HttpResolver;
pub use noop::NoopResolver;
pub use passthrough::PassthroughLayer;
pub use subprocess::SubprocessResolver;

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(filesize: Option<u64>) -> ResolveResponse {
        ResolveResponse {
            stream_url: "x".into(),
            headers: Default::default(),
            fps: None,
            codec: None,
            filesize,
            expires_at: None,
        }
    }

    #[test]
    fn cache_disabled_overrides_loop_and_size() {
        let r = resp(Some(1_000_000));
        assert!(!r.should_cache(true, false, 5_000_000));
    }

    #[test]
    fn cache_enabled_requires_loop_and_size_under_limit() {
        let r = resp(Some(1_000_000));
        assert!(r.should_cache(true, true, 5_000_000));
        assert!(!r.should_cache(false, true, 5_000_000));
        assert!(!r.should_cache(true, true, 500_000));
    }

    #[test]
    fn cache_enabled_unknown_size_does_not_cache() {
        let r = resp(None);
        assert!(!r.should_cache(true, true, 5_000_000));
    }
}
