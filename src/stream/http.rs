//! Shared `reqwest::Client` for media fetches and probes.
//!
//! `reqwest::Client` is `Arc`-backed and owns the connection pool, DNS
//! cache, and TLS config. Constructing one per call defeats keep-alive.
//! Per-request settings (timeout, user-agent, method) are layered on the
//! `RequestBuilder`, so a single shared client serves probe HEADs and
//! fetcher GETs alike.

use std::sync::LazyLock;

use reqwest::Client;

pub static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    // No top-level timeout — callers set their own per request. Default
    // redirect policy (limited(10)) covers our needs.
    Client::builder().build().unwrap_or_else(|_| Client::new())
});
