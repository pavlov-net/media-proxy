//! Resolves video URLs, probes metadata and autocrop, and starts ffmpeg frame delivery.

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::Config;
use crate::control::fields::StreamFields;
use crate::error::StreamError;
use crate::resolver::{ResolveRequest, Resolver};
use crate::stream::frame_source::{FrameSource, RgbFrame, VideoSource};
use crate::video::autocrop;
use crate::video::filter_graph::{AutocropRect, FilterGraphParams, build_filter_graph};
use crate::video::probe::{self, VideoProbeData};
use crate::video::subprocess::{FfmpegArgs, FfmpegFrame, FfmpegInput, spawn_ffmpeg};
use crate::video::{hwaccel, timing};

/// Fallback frame delay in milliseconds when PTS and frame rate are unavailable.
const DEFAULT_FRAME_DELAY_MS: f32 = 33.333;

pub async fn build_video_source(
    fields: &StreamFields,
    config: &Config,
    resolver: &Arc<dyn Resolver>,
) -> Result<FrameSource, StreamError> {
    let mut resolved = resolver
        .resolve(ResolveRequest {
            url: fields.source.clone(),
            target_w: fields.width,
            target_h: fields.height,
            hw_prefer: Some(fields.hw.as_canon().unwrap_or("none").to_string()),
            prefer_60fps: config.youtube.prefer_60fps,
        })
        .await?;

    if (resolved.stream_url.starts_with("http://") || resolved.stream_url.starts_with("https://"))
        && !resolved
            .headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case("user-agent"))
    {
        resolved
            .headers
            .insert("User-Agent".into(), config.net.user_agent.clone());
    }
    let http_headers = format_http_headers(&resolved.headers);

    // The probes are independent; mapping crop edges to source pixels needs both results.
    let metadata_fut = probe::probe_video_metadata(&resolved.stream_url, http_headers.as_deref());
    let (probe_data, raw_autocrop_rect) = if config.video.autocrop.enabled {
        let crop_fut = autocrop::probe_autocrop_rect(
            &resolved.stream_url,
            http_headers.as_deref(),
            &config.video.autocrop,
        );
        tokio::join!(metadata_fut, crop_fut)
    } else {
        (metadata_fut.await, None)
    };
    let autocrop_rect = match (probe_data, raw_autocrop_rect) {
        (Some(p), Some(rect)) => Some(autocrop::finalize_autocrop_rect(
            rect,
            p.display_dims(),
            &config.video.autocrop,
        )),
        _ => None,
    };

    let hw_backend = hwaccel::pick_for(fields.hw);
    let filter_graph = build_filter_graph_for(fields, probe_data.as_ref(), autocrop_rect);

    // `cache:` permits in-process seeking for small looping files. Other looping
    // videos restart through the orchestrator when ffmpeg reaches EOF.
    let input = if resolved.should_cache(
        fields.r#loop,
        config.youtube.cache.enabled,
        config.youtube.cache.max_size,
    ) {
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
        http_headers,
    })
    .await?;

    let avg_ms = resolved
        .fps
        .and_then(|fps| if fps > 0.0 { Some(1000.0 / fps) } else { None });

    // Synthetic MJPEG timestamps do not reflect arrival time; use fixed delays.
    let unreliable_pts = is_unreliable_pts(&fields.source);

    let (frame_tx, frame_rx) = mpsc::channel::<RgbFrame>(8);
    tokio::spawn(convert_frames(handles.frames, frame_tx, avg_ms, unreliable_pts));

    Ok(FrameSource::Video(VideoSource::new(
        frame_rx,
        handles.completion,
        handles.kill_guard,
    )))
}

/// Identifies MJPEG URLs whose synthetic PTS requires fixed frame delays.
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

/// Builds filters using display dimensions for aspect-ratio comparisons.
/// ffmpeg applies rotation during decode; rotating here would apply it twice.
fn build_filter_graph_for(
    fields: &StreamFields,
    probe: Option<&VideoProbeData>,
    autocrop_rect: Option<AutocropRect>,
) -> String {
    let (src_dims, sar_num, sar_den) = match probe {
        Some(p) => (Some(p.display_dims()), p.sar_num, p.sar_den),
        None => (None, 1, 1),
    };
    build_filter_graph(&FilterGraphParams {
        src_dims,
        sar_num,
        sar_den,
        rotation_deg: 0,
        target_width: fields.width,
        target_height: fields.height,
        fit: fields.fit,
        expand: fields.expand,
        autocrop: autocrop_rect,
    })
}

/// Converts PTS to frame delays until either channel closes.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::fields::{Fit, HwPref};
    use crate::output::sink::PixelFormat;
    use std::net::{IpAddr, Ipv4Addr};

    fn fields(w: u32, h: u32, fit: Fit) -> StreamFields {
        StreamFields {
            output_id: 0,
            width: w,
            height: h,
            source: "x".into(),
            ddp_port: 4048,
            ddp_host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            r#loop: false,
            expand: 0,
            hw: HwPref::None,
            fit,
            fmt: PixelFormat::Rgb888,
            pace: 0,
            ema: 0.0,
        }
    }

    fn probe(w: u32, h: u32, sar_num: u32, sar_den: u32, rotation_deg: u32) -> VideoProbeData {
        VideoProbeData {
            width: w,
            height: h,
            sar_num,
            sar_den,
            rotation_deg,
        }
    }

    #[test]
    fn display_dims_swaps_for_quarter_rotations() {
        assert_eq!(probe(1920, 1080, 1, 1, 0).display_dims(), (1920, 1080));
        assert_eq!(probe(1920, 1080, 1, 1, 90).display_dims(), (1080, 1920));
        assert_eq!(probe(1920, 1080, 1, 1, 180).display_dims(), (1920, 1080));
        assert_eq!(probe(1920, 1080, 1, 1, 270).display_dims(), (1080, 1920));
    }

    #[test]
    fn graph_without_probe_falls_back_to_pad() {
        let g = build_filter_graph_for(&fields(64, 64, Fit::Auto), None, None);
        assert!(g.contains("pad=64:64"));
    }

    #[test]
    fn graph_with_matching_ratio_picks_direct_scale() {
        let p = probe(1080, 1080, 1, 1, 0);
        let g = build_filter_graph_for(&fields(64, 64, Fit::Auto), Some(&p), None);
        assert!(!g.contains("pad="), "graph should not pad on matching ratio: {g}");
        assert!(g.contains("scale=64:64"));
    }

    #[test]
    fn graph_with_portrait_rotated_source_uses_display_dims() {
        // Rotation makes the portrait-coded source match the landscape target.
        let p = probe(1080, 1920, 1, 1, 90);
        let g = build_filter_graph_for(&fields(1920, 1080, Fit::Auto), Some(&p), None);
        assert!(
            !g.contains("pad="),
            "rotated source should compare ratios in display space, got: {g}"
        );
    }

    #[test]
    fn graph_with_autocrop_emits_crop_filter_first() {
        let p = probe(1920, 1080, 1, 1, 0);
        let rect = AutocropRect {
            l: 10,
            r: 10,
            t: 5,
            b: 5,
        };
        let g = build_filter_graph_for(&fields(64, 64, Fit::Pad), Some(&p), Some(rect));
        assert!(
            g.starts_with("crop="),
            "autocrop rect should emit crop= as the first filter, got: {g}"
        );
    }
}
