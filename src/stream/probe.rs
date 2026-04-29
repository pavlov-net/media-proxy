//! Probe a normalized source URL for image-vs-video routing.
//!
//! `classify()` decides on URL scheme + extension alone. That's wrong for
//! several real-world cases: `.jpg` URLs serving `multipart/x-mixed-replace`
//! (IP-camera MJPEG) need the video pipeline, and extension-less HTTP URLs
//! could be either. The Python version handled this with a pre-dispatch
//! HEAD request; this module restores that behavior.
//!
//! Strategy:
//! - `http(s)://` → HEAD request, branch on `Content-Type`.
//! - `file://` → read the first ~512 bytes, sniff with `infer`.
//! - `rtsp://` / `rtmp://` / `udp://` / `tcp://` → trust the scheme.
//! - Anything ambiguous falls back to extension-based [`classify`].

use std::time::Duration;

use tokio::io::AsyncReadExt;
use url::Url;

use crate::stream::http::CLIENT;
use crate::stream::url::{UrlKind, classify, classify_extension};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
}

const HEAD_TIMEOUT: Duration = Duration::from_secs(5);
const FILE_SNIFF_BYTES: usize = 512;

/// Probe `url` and return whether it should route to the image or video
/// pipeline. Never errors — probe failures (network, missing file, parse)
/// fall back to extension-based classification.
pub async fn probe(url: &str, user_agent: &str) -> MediaKind {
    let Ok(parsed) = Url::parse(url) else {
        return fallback(url);
    };
    match parsed.scheme() {
        "http" | "https" => probe_http(url, user_agent).await.unwrap_or_else(|| fallback(url)),
        "file" => probe_file(&parsed)
            .await
            .unwrap_or_else(|| fallback(parsed.as_str())),
        "rtsp" | "rtmp" | "udp" | "tcp" => MediaKind::Video,
        _ => fallback(url),
    }
}

async fn probe_http(url: &str, user_agent: &str) -> Option<MediaKind> {
    let resp = CLIENT
        .head(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .timeout(HEAD_TIMEOUT)
        .send()
        .await
        .ok()?;
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())?;
    classify_content_type(ct)
}

async fn probe_file(parsed: &Url) -> Option<MediaKind> {
    let path = parsed.to_file_path().ok()?;
    let mut file = tokio::fs::File::open(&path).await.ok()?;
    let mut buf = [0u8; FILE_SNIFF_BYTES];
    let n = file.read(&mut buf).await.ok()?;
    let t = infer::get(&buf[..n])?;
    match t.matcher_type() {
        infer::MatcherType::Image => Some(MediaKind::Image),
        infer::MatcherType::Video | infer::MatcherType::Audio => Some(MediaKind::Video),
        _ => None,
    }
}

fn classify_content_type(ct: &str) -> Option<MediaKind> {
    let mime = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    // IP-camera MJPEG and similar pushed-frames streams: `.jpg`-extension
    // URL but the response is a multipart stream that ffmpeg handles.
    if mime == "multipart/x-mixed-replace" {
        return Some(MediaKind::Video);
    }
    if mime.starts_with("image/") {
        return Some(MediaKind::Image);
    }
    if mime.starts_with("video/") || mime.starts_with("audio/") {
        return Some(MediaKind::Video);
    }
    // Common video-ish containers/manifests served as `application/*`.
    if matches!(
        mime.as_str(),
        "application/vnd.apple.mpegurl"
            | "application/x-mpegurl"
            | "application/dash+xml"
            | "application/mp4"
            | "application/x-mpegts"
    ) {
        return Some(MediaKind::Video);
    }
    // Everything else (`text/html`, `application/octet-stream`, missing) →
    // let the caller fall back to extension/scheme.
    None
}

fn fallback(url: &str) -> MediaKind {
    // Extension wins when present — covers both `file://` URLs (which
    // `classify` collapses to `LocalPath`) and HTTP URLs whose HEAD didn't
    // produce a useful Content-Type.
    if let Some(k) = classify_extension(url) {
        return match k {
            UrlKind::DirectImage => MediaKind::Image,
            _ => MediaKind::Video,
        };
    }
    match classify(url) {
        // No extension hint and bytes weren't recognizable. Local files
        // without an extension go to the image pipeline (matches Python
        // behavior); ambiguous HTTP URLs go to the resolver/ffmpeg path.
        UrlKind::LocalPath(_) => MediaKind::Image,
        _ => MediaKind::Video,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_mjpeg_is_video() {
        assert_eq!(
            classify_content_type("multipart/x-mixed-replace; boundary=foo"),
            Some(MediaKind::Video)
        );
    }

    #[test]
    fn ct_image_jpeg_is_image() {
        assert_eq!(classify_content_type("image/jpeg"), Some(MediaKind::Image));
    }

    #[test]
    fn ct_video_mp4_is_video() {
        assert_eq!(classify_content_type("video/mp4"), Some(MediaKind::Video));
    }

    #[test]
    fn ct_hls_manifest_is_video() {
        assert_eq!(
            classify_content_type("application/vnd.apple.mpegurl"),
            Some(MediaKind::Video)
        );
    }

    #[test]
    fn ct_html_is_unknown() {
        assert_eq!(classify_content_type("text/html; charset=utf-8"), None);
    }

    #[test]
    fn ct_octet_stream_is_unknown() {
        assert_eq!(classify_content_type("application/octet-stream"), None);
    }

    #[test]
    fn ct_case_insensitive() {
        assert_eq!(classify_content_type("IMAGE/PNG"), Some(MediaKind::Image));
    }

    #[test]
    fn fallback_image_extension() {
        assert_eq!(fallback("https://example.com/x.png"), MediaKind::Image);
    }

    #[test]
    fn fallback_unknown_http_is_video() {
        assert_eq!(fallback("https://www.youtube.com/watch?v=abc"), MediaKind::Video);
    }

    #[tokio::test]
    async fn probe_streaming_protocol_trusts_scheme() {
        assert_eq!(probe("rtsp://cam/live", "ua").await, MediaKind::Video);
        assert_eq!(probe("rtmp://server/live", "ua").await, MediaKind::Video);
    }

    #[tokio::test]
    async fn probe_local_image_via_magic_bytes() {
        // 1x1 PNG.
        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
            0x89,
        ];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("noext");
        tokio::fs::write(&path, png).await.unwrap();
        let url = Url::from_file_path(&path).unwrap().to_string();
        assert_eq!(probe(&url, "ua").await, MediaKind::Image);
    }

    #[tokio::test]
    async fn probe_local_missing_file_falls_back() {
        // No magic bytes available; falls through to extension-based
        // classify, which sees `.png` → Image.
        let url = "file:///nonexistent/path/to/foo.png";
        assert_eq!(probe(url, "ua").await, MediaKind::Image);
    }
}
