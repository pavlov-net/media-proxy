//! HTTP / file:// → bytes. Shared by static-image and animated paths.
//!
//! Enforces the 50 MB cap from [`MAX_SIZE_LIMIT`].

use std::path::Path;
use std::time::Duration;

use reqwest::Client;

use crate::error::{ImageError, MediaError};
use crate::image::decode::{MAX_SIZE_LIMIT, MEMORY_THRESHOLD};
use crate::stream::url::{UrlKind, classify};

pub async fn fetch_bytes(src_url: &str, user_agent: &str) -> Result<Vec<u8>, ImageError> {
    match classify(src_url) {
        UrlKind::LocalPath(path) => read_local(&path, src_url).await,
        UrlKind::HttpUnknown | UrlKind::DirectImage | UrlKind::DirectVideo => {
            fetch_http(src_url, user_agent).await
        }
        _ => Err(ImageError::Media(MediaError::Format {
            source_url: src_url.into(),
            message: "unsupported URL scheme".into(),
        })),
    }
}

async fn read_local(path: &Path, src_url: &str) -> Result<Vec<u8>, ImageError> {
    let bytes = tokio::fs::read(path).await.map_err(|_| {
        ImageError::Media(MediaError::NotFound {
            source_url: src_url.into(),
        })
    })?;
    if bytes.len() > MAX_SIZE_LIMIT {
        return Err(ImageError::DecompressionBomb {
            actual: bytes.len(),
            limit: MAX_SIZE_LIMIT,
        });
    }
    Ok(bytes)
}

async fn fetch_http(src_url: &str, user_agent: &str) -> Result<Vec<u8>, ImageError> {
    use futures::StreamExt;

    let client = Client::builder()
        .user_agent(user_agent)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| network_err(src_url, e.to_string(), None))?;

    let resp = client
        .get(src_url)
        .send()
        .await
        .map_err(|e| network_err(src_url, e.to_string(), e.status().map(|s| s.as_u16() as i32)))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(ImageError::Media(MediaError::Network {
            source_url: src_url.into(),
            message: format!("HTTP {}", status.as_u16()),
            error_code: Some(status.as_u16() as i32),
            retryable: status.is_server_error(),
        }));
    }

    if let Some(len) = resp.content_length()
        && (len as usize) > MAX_SIZE_LIMIT
    {
        return Err(ImageError::DecompressionBomb {
            actual: len as usize,
            limit: MAX_SIZE_LIMIT,
        });
    }

    // Stream the body chunk-by-chunk so a server that lies about (or omits)
    // Content-Length can't push past our cap — the trailing byte check
    // alone would already have buffered MAX_SIZE_LIMIT + overflow first.
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(MEMORY_THRESHOLD);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| network_err(src_url, e.to_string(), None))?;
        if buf.len() + chunk.len() > MAX_SIZE_LIMIT {
            return Err(ImageError::DecompressionBomb {
                actual: buf.len() + chunk.len(),
                limit: MAX_SIZE_LIMIT,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

fn network_err(src_url: &str, message: String, error_code: Option<i32>) -> ImageError {
    ImageError::Media(MediaError::Network {
        source_url: src_url.into(),
        message,
        error_code,
        retryable: true,
    })
}
