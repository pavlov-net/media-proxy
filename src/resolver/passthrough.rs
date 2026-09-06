//! Bypasses extraction for media identified by URL hints or HTTP headers.

use async_trait::async_trait;

use super::{ResolveRequest, ResolveResponse, Resolver};
use crate::error::ResolverError;
use crate::stream::url::classify;

pub struct PassthroughLayer {
    inner: Box<dyn Resolver>,
    user_agent: String,
}

impl PassthroughLayer {
    pub fn new(inner: Box<dyn Resolver>, user_agent: String) -> Self {
        Self { inner, user_agent }
    }
}

#[async_trait]
impl Resolver for PassthroughLayer {
    async fn resolve(&self, req: ResolveRequest) -> Result<ResolveResponse, ResolverError> {
        if classify(&req.url).is_direct_media() {
            return Ok(ResolveResponse::passthrough(req.url));
        }
        // Header checks recognize extensionless cameras and signed media URLs.
        if matches!(classify(&req.url), crate::stream::url::UrlKind::HttpUnknown)
            && crate::stream::probe::probe_http(&req.url, &self.user_agent)
                .await
                .is_some()
        {
            return Ok(ResolveResponse::passthrough(req.url));
        }
        self.inner.resolve(req).await
    }
}
