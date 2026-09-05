//! Shared HTTP client for connection reuse across media fetches and probes.

use std::sync::LazyLock;

use reqwest::Client;

pub static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    // Callers set deadlines; the client retains reqwest's default redirect limit.
    Client::builder().build().unwrap_or_else(|_| Client::new())
});
