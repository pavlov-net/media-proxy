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
        self.inner.resolve(req).await
    }
}
