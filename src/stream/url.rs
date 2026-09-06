//! Normalizes source strings at control boundaries and classifies URL hints
//! for routing, fetching, and resolver bypass.

use std::path::PathBuf;

use url::Url;

const IMAGE_EXT: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tiff", ".webp"];
const VIDEO_EXT: &[&str] = &[
    ".mp4", ".mkv", ".webm", ".mov", ".avi", ".flv", ".m4v", ".3gp", ".ts",
];

/// Returns a URL after trimming, percent-decoding, and rewriting internal sources.
/// Bare paths become file URLs relative to the server working directory; Windows
/// drive paths are recognized by the absence of `://`. Returns an error for empty
/// input, an unavailable working directory, or an unrepresentable file URL.
pub fn normalize_source(raw: &str, server_host: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("src must not be empty".into());
    }
    let decoded = percent_decode(trimmed);
    let rewritten = rewrite_internal(&decoded, server_host);

    if rewritten.contains("://") {
        return Ok(rewritten);
    }

    let path = std::path::Path::new(&rewritten);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("cwd unavailable for relative path: {e}"))?
            .join(path)
    };
    Url::from_file_path(&abs)
        .map(|u| u.into())
        .map_err(|()| format!("not a valid file path: {}", abs.display()))
}

/// Rewrites `internal:<path>[?query]` under the server's `/api/internal/` route.
/// Other source strings pass through unchanged.
pub fn rewrite_internal(url: &str, server_host: &str) -> String {
    let Some(rest) = url.strip_prefix("internal:") else {
        return url.to_string();
    };
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    match query {
        Some(q) => format!("http://{server_host}/api/internal/{path}?{q}"),
        None => format!("http://{server_host}/api/internal/{path}"),
    }
}

/// Returns whether ffmpeg HTTP options apply to this URL.
pub fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Decodes percent escapes, preserving the input if decoding produces invalid UTF-8.
pub fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|cow| cow.into_owned())
        .unwrap_or_else(|_| s.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlKind {
    LocalPath(PathBuf),
    DirectImage,
    DirectVideo,
    StreamingProtocol(&'static str),
    /// HTTP URL requiring header inspection or extraction to identify its media.
    HttpUnknown,
    Unknown,
}

pub fn classify(url: &str) -> UrlKind {
    if let Some(path) = as_local_path(url) {
        return UrlKind::LocalPath(path);
    }
    if let Some(k) = classify_extension(url) {
        return k;
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

/// Classifies recognized image or video extensions, ignoring query and fragment.
pub fn classify_extension(url: &str) -> Option<UrlKind> {
    let path_part = url.split_once('?').map_or(url, |(p, _)| p);
    let path_part = path_part.split_once('#').map_or(path_part, |(p, _)| p);
    let lower = path_part.to_ascii_lowercase();
    if IMAGE_EXT.iter().any(|ext| lower.ends_with(ext)) {
        return Some(UrlKind::DirectImage);
    }
    if VIDEO_EXT.iter().any(|ext| lower.ends_with(ext)) {
        return Some(UrlKind::DirectVideo);
    }
    None
}

/// Converts file URLs to paths, including platform-specific drive and UNC syntax.
pub fn as_local_path(url: &str) -> Option<PathBuf> {
    Url::parse(url).ok().and_then(|u| u.to_file_path().ok())
}

impl UrlKind {
    /// Returns whether URL hints suggest video, including HTTP sources needing extraction.
    pub fn is_video(&self) -> bool {
        matches!(
            self,
            Self::DirectVideo | Self::StreamingProtocol(_) | Self::HttpUnknown
        )
    }

    /// Returns whether URL hints identify media that can bypass extraction.
    pub fn is_direct_media(&self) -> bool {
        matches!(
            self,
            Self::LocalPath(_) | Self::DirectImage | Self::DirectVideo | Self::StreamingProtocol(_)
        )
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
    fn file_url_is_local_path() {
        let k = classify("file:///tmp/foo.png");
        assert!(matches!(k, UrlKind::LocalPath(_)));
        assert!(k.is_direct_media());
    }

    #[test]
    fn normalize_bare_path_to_file_url() {
        let out = normalize_source("/tmp/foo.png", "h:80").unwrap();
        assert_eq!(out, "file:///tmp/foo.png");
    }

    #[test]
    fn normalize_passes_http_through() {
        let out = normalize_source("https://example.com/x.png", "h:80").unwrap();
        assert_eq!(out, "https://example.com/x.png");
    }

    #[test]
    fn normalize_rewrites_internal() {
        let out = normalize_source("internal:placeholder/64x64.png", "h:80").unwrap();
        assert_eq!(out, "http://h:80/api/internal/placeholder/64x64.png");
    }

    #[test]
    fn normalize_percent_decodes_path() {
        let out = normalize_source("%2Ftmp%2Fmy%20file.png", "h:80").unwrap();
        // `Url::from_file_path` re-encodes spaces in the URL form.
        assert!(out.starts_with("file:///tmp/my"));
        assert!(out.ends_with("file.png"));
    }

    #[test]
    fn normalize_rejects_empty() {
        assert!(normalize_source("   ", "h:80").is_err());
    }

    #[test]
    fn normalize_preserves_streaming_protocol() {
        let out = normalize_source("rtsp://cam/live", "h:80").unwrap();
        assert_eq!(out, "rtsp://cam/live");
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
        assert!(k.is_video());
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
