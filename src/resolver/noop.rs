//! Rejects extraction requests when no resolver is configured.
//! [`super::PassthroughLayer`] handles direct media before this resolver.

use async_trait::async_trait;

use super::{ResolveRequest, ResolveResponse, Resolver};
use crate::error::ResolverError;

#[derive(Default)]
pub struct NoopResolver;

#[async_trait]
impl Resolver for NoopResolver {
    async fn resolve(&self, req: ResolveRequest) -> Result<ResolveResponse, ResolverError> {
        Err(ResolverError::Unavailable(format!(
            "no resolver available — install yt-dlp or set resolver.url to handle: {}",
            req.url
        )))
    }
}
