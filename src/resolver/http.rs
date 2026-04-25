//! Real HTTP resolver — POSTs to the addon sidecar.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use tracing::warn;

use super::{ResolveRequest, ResolveResponse, Resolver};
use crate::error::ResolverError;
use crate::stream::url::classify;

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
        if classify(&req.url).is_direct_media() {
            return Ok(ResolveResponse {
                stream_url: req.url,
                headers: Default::default(),
                fps: None,
                codec: None,
                filesize: None,
                expires_at: None,
            });
        }

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
            warn!(status = %status, body = %body.chars().take(200).collect::<String>(), "resolver error response");
            return Err(ResolverError::Unavailable(format!(
                "HTTP {}: {}",
                status.as_u16(),
                body.chars().take(200).collect::<String>()
            )));
        }
        let parsed: ResolveResponse = resp
            .json()
            .await
            .map_err(|e| ResolverError::InvalidResponse(format!("parse: {e}")))?;
        Ok(parsed)
    }
}
