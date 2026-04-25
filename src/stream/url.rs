//! URL classification — shared by the stream orchestrator, resolver, and fetcher.

use std::path::{Path, PathBuf};

const IMAGE_EXT: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tiff", ".webp"];
const VIDEO_EXT: &[&str] = &[
    ".mp4", ".mkv", ".webm", ".mov", ".avi", ".flv", ".m4v", ".3gp", ".ts",
];

/// Rewrite `internal:<path>[?query]` to `http://<server_host>/api/internal/<path>[?query]`.
/// Returns the input unchanged if it's not an `internal:` URL.
pub fn rewrite_internal(url: &str, server_host: &str) -> String {
    let Some(rest) = url.strip_prefix("internal:") else {
        return url.to_string();
    };
    // `internal:ha/sensor.temp?foo=bar` → path = `ha/sensor.temp`, query = `foo=bar`
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    match query {
        Some(q) => format!("http://{server_host}/api/internal/{path}?{q}"),
        None => format!("http://{server_host}/api/internal/{path}"),
    }
}

/// Decode percent-escapes in a source URL. Safe to apply before classification
/// since classification only inspects scheme + extension.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    // Invalid UTF-8 after decoding shouldn't happen for well-formed URLs; fall
    // back to the original string if it does.
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlKind {
    LocalPath(PathBuf),
    DirectImage,
    DirectVideo,
    StreamingProtocol(&'static str),
    /// HTTP/HTTPS without a recognized media extension — could be a page
    /// that needs the resolver, or a direct stream with no hint.
    HttpUnknown,
    Unknown,
}

pub fn classify(url: &str) -> UrlKind {
    if let Some(path) = as_local_path(url) {
        return UrlKind::LocalPath(path);
    }
    let lower = url.to_ascii_lowercase();
    if IMAGE_EXT.iter().any(|ext| lower.ends_with(ext)) {
        return UrlKind::DirectImage;
    }
    if VIDEO_EXT.iter().any(|ext| lower.ends_with(ext)) {
        return UrlKind::DirectVideo;
    }
    for scheme in ["rtmp", "rtsp", "udp", "tcp"] {
        if url.starts_with(&format!("{scheme}://")) {
            return UrlKind::StreamingProtocol(scheme);
        }
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return UrlKind::HttpUnknown;
    }
    UrlKind::Unknown
}

/// Returns a local filesystem path for `file://…` URLs and scheme-less paths.
pub fn as_local_path(url: &str) -> Option<PathBuf> {
    if let Some(rest) = url.strip_prefix("file://") {
        return Some(Path::new(rest).to_path_buf());
    }
    if !url.contains("://") {
        return Some(Path::new(url).to_path_buf());
    }
    None
}

impl UrlKind {
    /// True if the orchestrator should dispatch this URL through the video
    /// (ffmpeg-sidecar) pipeline instead of the still-image pipeline.
    pub fn is_video(&self) -> bool {
        matches!(self, Self::DirectVideo | Self::StreamingProtocol(_))
    }

    /// True if the resolver can short-circuit — the URL already points at
    /// direct media and doesn't need yt-dlp.
    pub fn is_direct_media(&self) -> bool {
        matches!(self, Self::LocalPath(_) | Self::DirectImage | Self::DirectVideo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_image_is_direct_image() {
        assert_eq!(classify("https://example.com/foo.png"), UrlKind::DirectImage);
    }

    #[test]
    fn local_path_without_scheme() {
        let k = classify("/tmp/foo.png");
        assert!(matches!(k, UrlKind::LocalPath(_)));
        assert!(k.is_direct_media());
    }

    #[test]
    fn mp4_is_video() {
        let k = classify("https://example.com/foo.MP4");
        assert_eq!(k, UrlKind::DirectVideo);
        assert!(k.is_video());
    }

    #[test]
    fn rtmp_is_streaming() {
        assert_eq!(classify("rtmp://server/live"), UrlKind::StreamingProtocol("rtmp"));
    }

    #[test]
    fn youtube_page_is_http_unknown() {
        let k = classify("https://www.youtube.com/watch?v=abc");
        assert_eq!(k, UrlKind::HttpUnknown);
        assert!(!k.is_direct_media());
    }

    #[test]
    fn internal_rewrite_adds_api_prefix() {
        assert_eq!(
            rewrite_internal("internal:placeholder/64x64.png", "192.168.1.1:8788"),
            "http://192.168.1.1:8788/api/internal/placeholder/64x64.png"
        );
    }

    #[test]
    fn internal_rewrite_preserves_query() {
        assert_eq!(
            rewrite_internal("internal:ha/sensor.temp?fmt=c", "h:80"),
            "http://h:80/api/internal/ha/sensor.temp?fmt=c"
        );
    }

    #[test]
    fn rewrite_passes_through_non_internal() {
        assert_eq!(
            rewrite_internal("https://example.com/x.png", "ignored"),
            "https://example.com/x.png"
        );
    }

    #[test]
    fn percent_decode_basic() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("%2Ftmp%2Ffoo"), "/tmp/foo");
    }

    #[test]
    fn percent_decode_leaves_invalid_sequences() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%ZZ"), "%ZZ");
    }
}
