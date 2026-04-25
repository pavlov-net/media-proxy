//! Video dispatch: resolver → ffmpeg → `VideoSource` frame channel.
//!
//! Flow for a `StartStream` pointing at a video URL:
//!
//! 1. `Resolver::resolve` yields a direct stream URL + HTTP headers.
//!    For local files and already-direct media, the passthrough resolver
//!    returns the input unchanged.
//! 2. We pick a hwaccel backend from `config.hw.prefer` intersected with
//!    ffmpeg's reported `-hwaccels`.
//! 3. We build the `-vf` chain using the target-only path
//!    (no source dims known up front — autocrop and Auto-vs-Pad smart
//!    scaling need a probe pass; that's a follow-up).
//! 4. We spawn ffmpeg with `-f rawvideo -pix_fmt rgb24` on stdout + showinfo
//!    on stderr.
//! 5. A conversion task drains the ffmpeg frame receiver, computes each
//!    frame's display delay from consecutive PTS, and forwards `RgbFrame`s
//!    into the `VideoSource` channel the runner consumes.

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::Config;
use crate::control::fields::StreamFields;
use crate::error::StreamError;
use crate::resolver::{ResolveRequest, Resolver};
use crate::stream::frame_source::{FrameSource, RgbFrame, VideoSource};
use crate::video::filter_graph::{FilterGraphParams, build_filter_graph};
use crate::video::subprocess::{FfmpegArgs, FfmpegFrame, FfmpegInput, spawn_ffmpeg};
use crate::video::{hwaccel, timing};

/// Fallback delay when neither PTS deltas nor an advertised fps are
/// available — ≈ 30 fps.
const DEFAULT_FRAME_DELAY_MS: f32 = 33.333;

pub async fn build_video_source(
    fields: &StreamFields,
    config: &Config,
    resolver: &Arc<dyn Resolver>,
) -> Result<FrameSource, StreamError> {
    let resolved = resolver
        .resolve(ResolveRequest {
            url: fields.source.clone(),
            target_w: fields.width,
            target_h: fields.height,
            hw_prefer: fields.hw.as_canon().map(str::to_string),
            prefer_60fps: config.youtube.prefer_60fps,
        })
        .await?;

    let hw_backend = hwaccel::pick_for(fields.hw);
    let filter_graph = build_target_only_graph(fields);

    // `cache:` makes the input seekable so ffmpeg's `-stream_loop -1` can
    // rewind in-place. For large or non-looping media, the orchestrator
    // rebuilds the whole source from scratch instead.
    let input = if resolved.should_cache(fields.r#loop, config.youtube.cache.max_size) {
        debug!(src = %fields.source, "ffmpeg cache: + stream_loop");
        FfmpegInput::CachedLooping(&resolved.stream_url)
    } else {
        FfmpegInput::Direct(&resolved.stream_url)
    };

    let handles = spawn_ffmpeg(FfmpegArgs {
        input,
        filter_graph: &filter_graph,
        output_width: fields.width,
        output_height: fields.height,
        hw: hw_backend,
        http_headers: format_http_headers(&resolved.headers),
    })
    .await?;

    let avg_ms = resolved
        .fps
        .and_then(|fps| if fps > 0.0 { Some(1000.0 / fps) } else { None });

    // MJPEG / jpeg_pipe streams advertise synthetic PTS that doesn't reflect
    // real frame arrival. Force fixed-interval pacing for these so burst
    // arrivals don't produce burst emits downstream.
    let unreliable_pts = is_unreliable_pts(&fields.source);

    let (frame_tx, frame_rx) = mpsc::channel::<RgbFrame>(8);
    tokio::spawn(convert_frames(handles.frames, frame_tx, avg_ms, unreliable_pts));

    Ok(FrameSource::Video(VideoSource::new(frame_rx, handles.completion)))
}

/// URL-level heuristic for formats that deliver synthetic PTS (MJPEG over
/// HTTP is the common case on IP cameras). Keeping this at the URL layer
/// avoids a mid-stream probe; any stream URL that smells like MJPEG is
/// treated as "trust fps, not PTS".
fn is_unreliable_pts(src_url: &str) -> bool {
    let lower = src_url.to_ascii_lowercase();
    lower.ends_with(".mjpeg") || lower.ends_with(".mjpg") || lower.contains("mjpg_streamer")
}

/// Skips entries whose name/value contains control chars to prevent a
/// malicious resolver response from injecting extra headers.
fn format_http_headers(headers: &std::collections::HashMap<String, String>) -> Option<String> {
    let mut joined = String::new();
    for (k, v) in headers {
        let k = k.trim();
        let v = v.trim();
        if k.is_empty() || contains_control(k) || contains_control(v) {
            continue;
        }
        joined.push_str(&format!("{k}: {v}\r\n"));
    }
    (!joined.is_empty()).then_some(joined)
}

fn contains_control(s: &str) -> bool {
    s.bytes().any(|b| b == b'\r' || b == b'\n' || b < 0x20)
}

/// Target-only filter graph — source dimensions are unknown without a
/// separate ffprobe pass, so Auto-fit and autocrop degrade gracefully:
/// the filter-graph builder treats `src_dims: None` as "fall through to
/// Pad" for Auto, and skips the crop filter for autocrop.
///
/// Rotation is not emitted: ffmpeg auto-rotates from container metadata by
/// default (since 4.2), so rotated captures render correctly without us
/// building `transpose` filters ourselves.
fn build_target_only_graph(fields: &StreamFields) -> String {
    build_filter_graph(&FilterGraphParams {
        src_dims: None,
        sar_num: 1,
        sar_den: 1,
        rotation_deg: 0,
        target_width: fields.width,
        target_height: fields.height,
        fit: fields.fit,
        expand: fields.expand,
        autocrop: None,
    })
}

/// Consume ffmpeg frames, convert each into an `RgbFrame` with a computed
/// display delay, and forward. Terminates when the upstream closes (ffmpeg
/// exited) or the downstream closes (runner dropped the receiver).
async fn convert_frames(
    mut src: mpsc::Receiver<FfmpegFrame>,
    dst: mpsc::Sender<RgbFrame>,
    avg_ms: Option<f32>,
    unreliable_pts: bool,
) {
    let mut clock = timing::DelayClock::new(avg_ms, DEFAULT_FRAME_DELAY_MS);
    while let Some(frame) = src.recv().await {
        let pts = if unreliable_pts { None } else { frame.pts_s };
        let delay_ms = clock.next_delay(pts);
        if dst
            .send(RgbFrame {
                rgb888: frame.rgb24,
                delay_ms,
            })
            .await
            .is_err()
        {
            debug!("video frame channel closed by consumer");
            return;
        }
    }
    if clock.frames_seen() == 0 {
        warn!("video: ffmpeg produced zero frames");
    }
}
