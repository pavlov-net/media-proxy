//! Fake resolver for integration tests: canned responses for registered
//! URLs, optional passthrough for the rest.

use std::collections::HashMap;

use async_trait::async_trait;

use super::{ResolveRequest, ResolveResponse, Resolver};
use crate::error::ResolverError;

#[derive(Default)]
pub struct FakeResolver {
    map: HashMap<String, ResolveResponse>,
    /// When set, returned for any URL not in `map`.
    passthrough: bool,
}

impl FakeResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_passthrough(mut self) -> Self {
        self.passthrough = true;
        self
    }

    pub fn insert(&mut self, url: impl Into<String>, response: ResolveResponse) {
        self.map.insert(url.into(), response);
    }
}

#[async_trait]
impl Resolver for FakeResolver {
    async fn resolve(&self, req: ResolveRequest) -> Result<ResolveResponse, ResolverError> {
        if let Some(resp) = self.map.get(&req.url) {
            return Ok(resp.clone());
        }
        if self.passthrough {
            return Ok(ResolveResponse::passthrough(req.url));
        }
        Err(ResolverError::Unavailable(format!(
            "fake resolver has no entry for: {}",
            req.url
        )))
    }
}
