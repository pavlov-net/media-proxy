//! Adapts resolver requests to yt-dlp format selection and metadata extraction.

use std::time::Duration;

use async_trait::async_trait;

use super::{ResolveRequest, ResolveResponse, Resolver};
use crate::error::ResolverError;
use crate::platform::pick_hw_backend;
use crate::video::hwaccel;
use crate::yt_dlp::YtDlp;
use crate::yt_dlp::format::{FormatParams, build_format};
use crate::yt_dlp::output::parse_googlevideo_expire;

pub struct SubprocessResolver {
    yt_dlp: YtDlp,
    timeout: Duration,
}

impl SubprocessResolver {
    pub fn new(yt_dlp: YtDlp, timeout: Duration) -> Self {
        Self { yt_dlp, timeout }
    }
}

#[async_trait]
impl Resolver for SubprocessResolver {
    async fn resolve(&self, req: ResolveRequest) -> Result<ResolveResponse, ResolverError> {
        // Codec preference must match the backend available for decoding.
        let hw = pick_hw_backend(req.hw_prefer.as_deref().unwrap_or("auto"), hwaccel::available());
        let format_expr = build_format(&FormatParams {
            height: req.target_h,
            hw,
            prefer_60fps: req.prefer_60fps,
            video_only: true,
        });

        let info = self.yt_dlp.resolve(&req.url, &format_expr, self.timeout).await?;

        let stream_url = info.url.ok_or_else(|| {
            ResolverError::InvalidResponse("yt-dlp returned no `url` (merged format selection?)".into())
        })?;
        let expires_at = parse_googlevideo_expire(&stream_url);

        Ok(ResolveResponse {
            stream_url,
            headers: info.http_headers.unwrap_or_default(),
            fps: info.fps,
            codec: info.vcodec,
            filesize: info.filesize.or(info.filesize_approx),
            expires_at,
        })
    }
}
