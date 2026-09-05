//! Detects black bars from grayscale frame samples. Edge medians are capped by
//! `max_bar_ratio`, then mapped from probe dimensions to source pixels.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time;
use tracing::{debug, warn};

use crate::config::AutocropConfig;
use crate::stream::url::is_http_url;
use crate::video::filter_graph::AutocropRect;
use crate::video::subprocess::add_http_reconnect_args;

/// Probe dimensions balance edge precision against decoding cost.
const PROBE_W: u32 = 160;
const PROBE_H: u32 = 120;

/// Limits probe time so stalled live sources cannot block stream startup.
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// HTTP read timeout, shorter than the outer probe budget.
const HTTP_RW_TIMEOUT: Duration = Duration::from_secs(6);

/// Returns per-edge medians across frame samples.
pub fn median_rect(samples: &[AutocropRect]) -> Option<AutocropRect> {
    if samples.is_empty() {
        return None;
    }
    let mut ls: Vec<u32> = samples.iter().map(|s| s.l).collect();
    let mut rs: Vec<u32> = samples.iter().map(|s| s.r).collect();
    let mut ts: Vec<u32> = samples.iter().map(|s| s.t).collect();
    let mut bs: Vec<u32> = samples.iter().map(|s| s.b).collect();
    for v in [&mut ls, &mut rs, &mut ts, &mut bs] {
        v.sort_unstable();
    }
    let mid = samples.len() / 2;
    Some(AutocropRect {
        l: ls[mid],
        r: rs[mid],
        t: ts[mid],
        b: bs[mid],
    })
}

/// Returns median bar widths in probe coordinates; `None` means probing failed
/// or produced no frames. Use [`finalize_autocrop_rect`] once source dimensions
/// are available, allowing this probe to run alongside metadata extraction.
pub async fn probe_autocrop_rect(
    url: &str,
    headers: Option<&str>,
    cfg: &AutocropConfig,
) -> Option<AutocropRect> {
    let n = cfg.probe_frames.max(1);
    let frame_bytes = (PROBE_W as usize) * (PROBE_H as usize);

    let raw_frames = match time::timeout(
        PROBE_TIMEOUT,
        decode_grayscale_frames(url, headers, n, frame_bytes),
    )
    .await
    {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            debug!(%url, error = %e, "autocrop decode failed");
            return None;
        }
        Err(_) => {
            warn!(%url, "autocrop probe timed out");
            return None;
        }
    };
    if raw_frames.is_empty() {
        return None;
    }

    let samples: Vec<AutocropRect> = raw_frames
        .iter()
        .map(|f| detect_bars(f, PROBE_W, PROBE_H, cfg))
        .collect();
    median_rect(&samples)
}

/// Maps probe edges to source pixels and suppresses bars below `min_bar_px`.
pub fn finalize_autocrop_rect(
    probe: AutocropRect,
    src_dims: (u32, u32),
    cfg: &AutocropConfig,
) -> AutocropRect {
    scale_to_source(probe, src_dims, cfg)
}

async fn decode_grayscale_frames(
    url: &str,
    headers: Option<&str>,
    n_frames: u32,
    frame_bytes: usize,
) -> Result<Vec<Vec<u8>>, std::io::Error> {
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-nostdin")
        .arg("-loglevel")
        .arg("error");

    if is_http_url(url) {
        add_http_reconnect_args(&mut cmd, HTTP_RW_TIMEOUT, 2);
    }
    if let Some(h) = headers {
        cmd.arg("-headers").arg(h);
    }

    cmd.arg("-i").arg(url);
    cmd.arg("-vframes").arg(n_frames.to_string());
    cmd.arg("-vf")
        .arg(format!("scale={PROBE_W}:{PROBE_H},format=gray"));
    cmd.arg("-f")
        .arg("rawvideo")
        .arg("-pix_fmt")
        .arg("gray")
        .arg("-an")
        .arg("pipe:1");

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .kill_on_drop(true);

    let mut child = cmd.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("ffmpeg stdout pipe missing"))?;

    let mut frames: Vec<Vec<u8>> = Vec::with_capacity(n_frames as usize);
    for _ in 0..n_frames {
        let mut buf = vec![0u8; frame_bytes];
        match stdout.read_exact(&mut buf).await {
            Ok(_) => frames.push(buf),
            // Short clips can provide fewer samples; zero samples disable autocrop.
            Err(_) => break,
        }
    }
    let _ = child.wait().await;
    Ok(frames)
}

/// Finds each edge's first row or column above `luma_thresh`.
/// `max_bar_ratio` limits cropping of dark content mistaken for letterboxing.
fn detect_bars(frame: &[u8], w: u32, h: u32, cfg: &AutocropConfig) -> AutocropRect {
    let max_v = ((h as f32) * cfg.max_bar_ratio).floor() as u32;
    let max_h = ((w as f32) * cfg.max_bar_ratio).floor() as u32;
    AutocropRect {
        t: walk_edge(max_v.min(h), cfg.luma_thresh, |r| row_median(frame, w, r)),
        b: walk_edge(max_v.min(h), cfg.luma_thresh, |r| row_median(frame, w, h - 1 - r)),
        l: walk_edge(max_h.min(w), cfg.luma_thresh, |c| col_median(frame, w, h, c)),
        r: walk_edge(max_h.min(w), cfg.luma_thresh, |c| {
            col_median(frame, w, h, w - 1 - c)
        }),
    }
}

/// Returns the first index above `thresh`, or `max` if the band stays dark.
fn walk_edge(max: u32, thresh: u8, median_at: impl Fn(u32) -> u8) -> u32 {
    for i in 0..max {
        if median_at(i) > thresh {
            return i;
        }
    }
    max
}

fn row_median(frame: &[u8], w: u32, row: u32) -> u8 {
    let off = (row as usize) * (w as usize);
    let mut buf: Vec<u8> = frame[off..off + w as usize].to_vec();
    buf.sort_unstable();
    buf[buf.len() / 2]
}

fn col_median(frame: &[u8], w: u32, h: u32, col: u32) -> u8 {
    let mut buf: Vec<u8> = (0..h)
        .map(|r| frame[(r as usize) * (w as usize) + col as usize])
        .collect();
    buf.sort_unstable();
    buf[buf.len() / 2]
}

/// Maps probe edges to source pixels, discarding bars below `min_bar_px`
/// to avoid cropping isolated dark edge pixels.
fn scale_to_source(probe: AutocropRect, src: (u32, u32), cfg: &AutocropConfig) -> AutocropRect {
    let (sw, sh) = src;
    let scale = |v: u32, probe_dim: u32, src_dim: u32| -> u32 {
        ((v as u64 * src_dim as u64) / probe_dim.max(1) as u64) as u32
    };
    let l = scale(probe.l, PROBE_W, sw);
    let r = scale(probe.r, PROBE_W, sw);
    let t = scale(probe.t, PROBE_H, sh);
    let b = scale(probe.b, PROBE_H, sh);
    let floor = |v: u32| if v < cfg.min_bar_px { 0 } else { v };
    AutocropRect {
        l: floor(l),
        r: floor(r),
        t: floor(t),
        b: floor(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AutocropConfig {
        AutocropConfig {
            enabled: true,
            probe_frames: 4,
            luma_thresh: 16,
            max_bar_ratio: 0.5,
            min_bar_px: 2,
        }
    }

    /// 160x120 with 20-row black bars top/bottom, white middle.
    fn frame_with_horizontal_bars(top: u32, bottom: u32) -> Vec<u8> {
        let w = PROBE_W as usize;
        let h = PROBE_H as usize;
        let mut f = vec![255u8; w * h];
        for r in 0..top as usize {
            for c in 0..w {
                f[r * w + c] = 0;
            }
        }
        for r in 0..bottom as usize {
            let row = h - 1 - r;
            for c in 0..w {
                f[row * w + c] = 0;
            }
        }
        f
    }

    #[test]
    fn detects_top_and_bottom_bars() {
        let f = frame_with_horizontal_bars(20, 20);
        let r = detect_bars(&f, PROBE_W, PROBE_H, &cfg());
        assert_eq!(r.t, 20);
        assert_eq!(r.b, 20);
        assert_eq!(r.l, 0);
        assert_eq!(r.r, 0);
    }

    #[test]
    fn no_bars_returns_zero() {
        let w = PROBE_W as usize;
        let h = PROBE_H as usize;
        let f = vec![255u8; w * h];
        let r = detect_bars(&f, PROBE_W, PROBE_H, &cfg());
        assert_eq!((r.l, r.r, r.t, r.b), (0, 0, 0, 0));
    }

    #[test]
    fn walk_capped_by_max_ratio() {
        // The ratio cap also applies to fully black frames.
        let w = PROBE_W as usize;
        let h = PROBE_H as usize;
        let f = vec![0u8; w * h];
        let mut c = cfg();
        c.max_bar_ratio = 0.5;
        let r = detect_bars(&f, PROBE_W, PROBE_H, &c);
        assert_eq!(r.t, (PROBE_H as f32 * 0.5) as u32);
        assert_eq!(r.l, (PROBE_W as f32 * 0.5) as u32);
    }

    #[test]
    fn scale_to_source_applies_floor() {
        // Isolated dark pixels fall below the minimum bar width.
        let probe_rect = AutocropRect {
            l: 1,
            r: 0,
            t: 0,
            b: 0,
        };
        let r = scale_to_source(probe_rect, (PROBE_W, PROBE_H), &cfg());
        assert_eq!(r.l, 0, "tiny bars must be zeroed by min_bar_px floor");
    }

    #[test]
    fn scale_to_source_maps_proportionally() {
        // 24/120 of probe height -> 24/120 * 1080 = 216px of source.
        let probe_rect = AutocropRect {
            l: 0,
            r: 0,
            t: 24,
            b: 24,
        };
        let r = scale_to_source(probe_rect, (1920, 1080), &cfg());
        assert_eq!(r.t, 216);
        assert_eq!(r.b, 216);
    }
}
