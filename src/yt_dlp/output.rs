//! Parsing helpers for `yt-dlp -j` stdout. The shape of `Info` is the
//! intersection we actually consume, not yt-dlp's full info-dict schema.
//! `serde(default)` everywhere because extractors are inconsistent about
//! which fields they populate (e.g. live streams skip `filesize`).

use std::collections::HashMap;

use serde::Deserialize;

/// Subset of yt-dlp's info-dict that we use. yt-dlp emits dozens of fields;
/// we only deserialize the ones the resolver consumes.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct Info {
    /// The selected stream URL. Populated when `-f` picks a single format
    /// (i.e. no `+` merge). For merged selections the URL lives in
    /// `requested_formats[*].url` instead — we don't support that here.
    #[serde(default)]
    pub url: Option<String>,

    /// Per-stream HTTP headers (User-Agent, Accept-Language, etc.) that the
    /// CDN expects. Forwarded into ffmpeg's `-headers` option verbatim.
    #[serde(default)]
    pub http_headers: Option<HashMap<String, String>>,

    #[serde(default)]
    pub fps: Option<f32>,

    /// Codec descriptor like `av01.0.01M.08` or `avc1.4D401E`.
    #[serde(default)]
    pub vcodec: Option<String>,

    #[serde(default)]
    pub filesize: Option<u64>,

    /// yt-dlp's range-derived size estimate when the exact size isn't known
    /// up front. Use as a fallback when `filesize` is absent.
    #[serde(default)]
    pub filesize_approx: Option<u64>,
}

/// Extract the unix-second expiry timestamp from a googlevideo CDN URL.
///
/// YouTube stream URLs include `?expire=<unix-seconds>`. Returns `None` for
/// any URL that doesn't carry the parameter (non-YouTube extractors,
/// non-googlevideo hosts, malformed URLs).
pub fn parse_googlevideo_expire(url: &str) -> Option<i64> {
    url.split_once('?')?
        .1
        .split('&')
        .find_map(|pair| pair.strip_prefix("expire=").and_then(|v| v.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_googlevideo_url() {
        let url = "https://rr1---sn-fxc25nn-nwje.googlevideo.com/videoplayback\
                   ?expire=1777158755&ei=foo&ip=1.2.3.4";
        assert_eq!(parse_googlevideo_expire(url), Some(1_777_158_755));
    }

    #[test]
    fn missing_expire_returns_none() {
        assert_eq!(parse_googlevideo_expire("https://example.com/a.mp4"), None);
        assert_eq!(parse_googlevideo_expire("https://example.com/a?other=1"), None);
    }

    #[test]
    fn malformed_expire_returns_none() {
        assert_eq!(
            parse_googlevideo_expire("https://example.com/a?expire=notanumber"),
            None
        );
    }

    #[test]
    fn expire_in_any_position() {
        assert_eq!(
            parse_googlevideo_expire("https://x/y?a=1&expire=42&b=2"),
            Some(42)
        );
        assert_eq!(parse_googlevideo_expire("https://x/y?expire=42"), Some(42));
        assert_eq!(
            parse_googlevideo_expire("https://x/y?a=1&b=2&expire=42"),
            Some(42)
        );
    }

    #[test]
    fn info_deserializes_minimal_yt_dlp_output() {
        // Minimum yt-dlp -j output for a non-YouTube extractor.
        let json = r#"{"url": "https://cdn.example/v.mp4", "vcodec": "h264"}"#;
        let info: Info = serde_json::from_str(json).expect("parse");
        assert_eq!(info.url.as_deref(), Some("https://cdn.example/v.mp4"));
        assert_eq!(info.vcodec.as_deref(), Some("h264"));
        assert!(info.http_headers.is_none());
        assert!(info.fps.is_none());
    }

    #[test]
    fn info_deserializes_full_youtube_output() {
        let json = r#"{
            "url": "https://googlevideo.com/v?expire=42",
            "http_headers": {"User-Agent": "Mozilla/5.0", "Accept": "*/*"},
            "fps": 30.0,
            "vcodec": "av01.0.01M.08",
            "filesize": 5685561
        }"#;
        let info: Info = serde_json::from_str(json).expect("parse");
        assert_eq!(info.fps, Some(30.0));
        assert_eq!(info.filesize, Some(5_685_561));
        let headers = info.http_headers.expect("headers");
        assert_eq!(headers.get("User-Agent").map(String::as_str), Some("Mozilla/5.0"));
    }

    #[test]
    fn info_tolerates_unknown_fields() {
        // yt-dlp's info-dict is huge; we should ignore everything we don't ask for.
        let json = r#"{
            "url": "x",
            "title": "ignored",
            "thumbnails": [],
            "automatic_captions": {},
            "_unknown": 42
        }"#;
        let info: Info = serde_json::from_str(json).expect("parse");
        assert_eq!(info.url.as_deref(), Some("x"));
    }

    #[test]
    fn info_falls_back_to_filesize_approx() {
        let json = r#"{"url": "x", "filesize_approx": 1000}"#;
        let info: Info = serde_json::from_str(json).expect("parse");
        assert_eq!(info.filesize, None);
        assert_eq!(info.filesize_approx, Some(1000));
    }
}
