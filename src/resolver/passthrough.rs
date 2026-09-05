//! Decorator that short-circuits the inner resolver for URLs already
//! pointing at direct media (file://, *.mp4, *.gif, …).

use async_trait::async_trait;

use super::{ResolveRequest, ResolveResponse, Resolver};
use crate::error::ResolverError;
use crate::stream::url::classify;

pub struct PassthroughLayer {
    inner: Box<dyn Resolver>,
}

impl PassthroughLayer {
    pub fn new(inner: Box<dyn Resolver>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl Resolver for PassthroughLayer {
    async fn resolve(&self, req: ResolveRequest) -> Result<ResolveResponse, ResolverError> {
        if classify(&req.url).is_direct_media() {
            return Ok(ResolveResponse::passthrough(req.url));
        }
        // Extensionless camera endpoints and signed CDN URLs may already be
        // direct media. Only HTML/unknown responses need an extractor.
        if matches!(classify(&req.url), crate::stream::url::UrlKind::HttpUnknown)
            && let Ok(response) = crate::stream::http::CLIENT
                .head(&req.url)
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
            && response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(crate::stream::probe::classify_content_type)
                .is_some()
        {
            return Ok(ResolveResponse::passthrough(req.url));
        }
        self.inner.resolve(req).await
    }
}
