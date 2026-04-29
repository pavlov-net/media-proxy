//! Fail-closed resolver. Used when neither a local `yt-dlp` nor an HTTP
//! resolver sidecar is configured. Direct-media URLs still work because
//! [`super::PassthroughLayer`] short-circuits them before reaching here.

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
