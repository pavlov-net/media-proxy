//! URL classification — shared by the stream orchestrator, resolver, and fetcher.
//!
//! Sources from the wire are normalized to URL strings at the entry-point
//! boundaries via [`normalize_source`]; everything downstream assumes URL
//! form (`http://`, `https://`, `file://`, `rtsp://`, …).

use std::path::PathBuf;

use url::Url;

const IMAGE_EXT: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tiff", ".webp"];
const VIDEO_EXT: &[&str] = &[
    ".mp4", ".mkv", ".webm", ".mov", ".avi", ".flv", ".m4v", ".3gp", ".ts",
];

/// Normalize a wire-level `src` into a URL string. Run at every entry-point
/// boundary so downstream code can assume URL form.
///
/// Pipeline: trim → percent-decode → rewrite `internal:` → wrap bare paths
/// in `file://`. Relative paths resolve against the server's cwd. Windows
/// drive letters fall through to the path branch (`c:\foo` has `:` but
/// not `://`).
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

/// True for `http://` / `https://` URLs. Used by ffmpeg-arg builders that
/// want to attach HTTP-protocol options (`-headers`, `-reconnect`).
pub fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Decode percent-escapes in a source URL. Falls back to the original
/// string if the decoded bytes aren't valid UTF-8.
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
    /// HTTP/HTTPS without a recognized media extension — could be a page
    /// that needs the resolver, or a direct stream with no hint.
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

/// Inspect the URL's extension (ignoring query/fragment) and return
/// `DirectImage` / `DirectVideo` if recognized. Used by [`probe`] to
/// classify `file://` URLs by extension when magic-byte detection fails,
/// and as a fallback when HEAD doesn't return a useful Content-Type.
pub fn classify_extension(url: &str) -> Option<UrlKind> {
    // Strip query and fragment so `foo.png?token=…` still matches `.png`.
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

/// Returns a local filesystem path for `file://…` URLs. After
/// [`normalize_source`] runs at every entry-point boundary, scheme-less
/// paths shouldn't reach this function; only `file://` URLs need to be
/// recognized. `Url::to_file_path` handles cross-platform decoding (drive
/// letters on Windows, UNC shares, percent-decoding).
pub fn as_local_path(url: &str) -> Option<PathBuf> {
    Url::parse(url).ok().and_then(|u| u.to_file_path().ok())
}

impl UrlKind {
    /// True if the orchestrator should dispatch this URL through the video
    /// pipeline (resolver + ffmpeg). `HttpUnknown` is included so yt-dlp
    /// pages reach the resolver and bare HTTP streams reach ffmpeg.
    pub fn is_video(&self) -> bool {
        matches!(
            self,
            Self::DirectVideo | Self::StreamingProtocol(_) | Self::HttpUnknown
        )
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
    fn file_url_is_local_path() {
        // After normalization, local paths reach `classify` as `file://`
        // URLs; bare paths are no longer valid input here.
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
