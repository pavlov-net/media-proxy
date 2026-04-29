//! Real HTTP resolver — POSTs to an external resolver sidecar.
//!
//! Wrap with [`super::PassthroughLayer`] for direct-media short-circuiting;
//! this impl assumes every URL it sees needs full extraction.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use tracing::warn;

use super::{ResolveRequest, ResolveResponse, Resolver};
use crate::error::ResolverError;

pub struct HttpResolver {
    endpoint: String,
    client: Client,
}

impl HttpResolver {
    pub fn new(endpoint: String, timeout: Duration) -> Result<Self, ResolverError> {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(ResolverError::from)?;
        Ok(Self { endpoint, client })
    }
}

#[async_trait]
impl Resolver for HttpResolver {
    async fn resolve(&self, req: ResolveRequest) -> Result<ResolveResponse, ResolverError> {
        let resp = self
            .client
            .post(&self.endpoint)
            .json(&req)
            .send()
            .await
            .map_err(ResolverError::from)?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let snippet: String = body.chars().take(200).collect();
            warn!(status = %status, body = %snippet, "resolver error response");
            return Err(ResolverError::Unavailable(format!(
                "HTTP {}: {snippet}",
                status.as_u16()
            )));
        }
        let parsed: ResolveResponse = resp
            .json()
            .await
            .map_err(|e| ResolverError::InvalidResponse(format!("parse: {e}")))?;
        Ok(parsed)
    }
}
