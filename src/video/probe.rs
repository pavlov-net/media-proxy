//! ffprobe pre-pass: extract source width/height/SAR/rotation/fps so the
//! filter graph can do source-aware decisions (Auto-fit direct-scale,
//! transpose for rotated content) before the main ffmpeg spawn.
//!
//! All failures are non-fatal — on probe error or timeout, callers fall
//! back to the target-only graph (Pad fit, no rotation handling). This
//! matters most for live sources (RTSP, MJPEG) where ffprobe can hang or
//! return nothing useful: graceful degradation beats blocking forever.

use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;
use tokio::time;
use tracing::{debug, warn};

use crate::stream::url::is_http_url;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Reused for ffprobe's `-rw_timeout` (microseconds) so a stalled socket
/// doesn't outlive `PROBE_TIMEOUT`. Slightly tighter than `PROBE_TIMEOUT`
/// to give ffprobe a chance to fail gracefully before tokio aborts it.
const HTTP_RW_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoProbeData {
    /// Coded source dimensions, before container rotation is applied.
    /// For display-orientation dims (what a player would render), use
    /// [`VideoProbeData::display_dims`].
    pub width: u32,
    pub height: u32,
    pub sar_num: u32,
    pub sar_den: u32,
    /// Container rotation, normalised by [`normalise_rotation`] to one of
    /// `{0, 90, 180, 270}`. `parse_ffprobe` is the only construction site
    /// that produces `VideoProbeData`, so the invariant holds for all
    /// values that reach external callers.
    pub rotation_deg: u32,
}

impl VideoProbeData {
    /// Coded `(width, height)` — pre-rotation, as ffprobe reported.
    pub fn dims(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Display `(width, height)` — coded dims swapped for 90°/270°
    /// container rotations, since ffmpeg auto-rotates at decode time and
    /// the filter graph reasons in display orientation.
    pub fn display_dims(&self) -> (u32, u32) {
        if matches!(self.rotation_deg, 90 | 270) {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

/// Run ffprobe with a hard timeout. Returns `None` on any failure
/// (binary missing, timeout, JSON parse, no video stream). Callers must
/// be prepared to continue without source-side metadata.
pub async fn probe_video_metadata(url: &str, headers: Option<&str>) -> Option<VideoProbeData> {
    let raw = match time::timeout(PROBE_TIMEOUT, run_ffprobe(Command::new("ffprobe"), url, headers)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            debug!(%url, error = %e, "ffprobe failed");
            return None;
        }
        Err(_) => {
            warn!(%url, "ffprobe timed out");
            return None;
        }
    };
    parse_ffprobe(&raw)
}

async fn run_ffprobe(mut cmd: Command, url: &str, headers: Option<&str>) -> Result<String, std::io::Error> {
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=width,height,sample_aspect_ratio,side_data_list:stream_tags=rotate")
        .arg("-of")
        .arg("json");

    if is_http_url(url) {
        let timeout_us = HTTP_RW_TIMEOUT.as_micros().to_string();
        cmd.args(["-rw_timeout", &timeout_us]);
    }
    if let Some(h) = headers {
        cmd.arg("-headers").arg(h);
    }

    cmd.arg("-i").arg(url);
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .kill_on_drop(true);

    let out = cmd.output().await?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!("ffprobe exited {}", out.status)));
    }
    String::from_utf8(out.stdout).map_err(std::io::Error::other)
}

#[derive(Deserialize)]
struct FfprobeJson {
    streams: Vec<FfprobeStream>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    sample_aspect_ratio: Option<String>,
    #[serde(default)]
    side_data_list: Vec<FfprobeSideData>,
    #[serde(default)]
    tags: FfprobeTags,
}

#[derive(Deserialize, Default)]
struct FfprobeTags {
    #[serde(default)]
    rotate: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeSideData {
    #[serde(default)]
    rotation: Option<f64>,
}

fn parse_ffprobe(raw: &str) -> Option<VideoProbeData> {
    let parsed: FfprobeJson = serde_json::from_str(raw).ok()?;
    let s = parsed.streams.into_iter().next()?;
    let width = s.width?;
    let height = s.height?;
    if width == 0 || height == 0 {
        return None;
    }
    let (sar_num, sar_den) = parse_sar(s.sample_aspect_ratio.as_deref());
    let rotation_deg = normalise_rotation(extract_rotation(&s));
    Some(VideoProbeData {
        width,
        height,
        sar_num,
        sar_den,
        rotation_deg,
    })
}

/// `"10:11"` → `(10, 11)`. `"0:1"` and unparseable values become `(1, 1)`,
/// matching ffmpeg's "unknown SAR → assume square pixels" convention.
fn parse_sar(s: Option<&str>) -> (u32, u32) {
    let Some(s) = s else { return (1, 1) };
    let Some((n, d)) = s.split_once(':') else {
        return (1, 1);
    };
    let n: u32 = n.parse().unwrap_or(0);
    let d: u32 = d.parse().unwrap_or(0);
    if n == 0 || d == 0 { (1, 1) } else { (n, d) }
}

/// Rotation source priority: side_data_list[].rotation (newer
/// container metadata, signed degrees) wins over the legacy `rotate`
/// stream tag (positive degrees as a string).
fn extract_rotation(s: &FfprobeStream) -> f64 {
    if let Some(side) = s.side_data_list.iter().find_map(|sd| sd.rotation) {
        return side;
    }
    s.tags
        .rotate
        .as_deref()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// Coerce arbitrary rotation values into one of {0, 90, 180, 270}.
/// Negative values come from displaymatrix side data
/// (e.g. -90 means 90° counter-clockwise = 270° clockwise). The filter
/// graph emits transposes in clockwise terms, so we normalise here.
fn normalise_rotation(deg: f64) -> u32 {
    let n = ((deg.round() as i64).rem_euclid(360)) as u32;
    match n {
        45..=134 => 90,
        135..=224 => 180,
        225..=314 => 270,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    mod cancellation {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        struct ProbeCleanup(rustix::process::Pid);

        impl Drop for ProbeCleanup {
            fn drop(&mut self) {
                let _ = rustix::process::kill_process(self.0, rustix::process::Signal::KILL);
            }
        }

        #[tokio::test]
        async fn deadline_and_cancellation_reap_probe() {
            for cancel in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let bin = dir.path().join("ffprobe");
                let pid_file = bin.with_extension("pid");
                std::fs::write(&bin, "#!/bin/sh\necho $$ > \"$0.pid\"\nexec sleep 30\n").unwrap();
                std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
                let timeout = Duration::from_millis(if cancel { 30_000 } else { 300 });
                let task = tokio::spawn(async move {
                    time::timeout(
                        timeout,
                        run_ffprobe(Command::new(bin), "rtsp://camera/live", None),
                    )
                    .await
                });
                let probe = time::timeout(Duration::from_secs(5), async {
                    loop {
                        if let Ok(text) = tokio::fs::read_to_string(&pid_file).await
                            && let Ok(pid) = text.trim().parse::<i32>()
                            && let Some(pid) = rustix::process::Pid::from_raw(pid)
                        {
                            break ProbeCleanup(pid);
                        }
                        time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("probe must start");

                if cancel {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                } else {
                    assert!(task.await.unwrap().is_err(), "probe must exceed its deadline");
                }
                time::timeout(Duration::from_secs(2), async {
                    while rustix::process::test_kill_process(probe.0) != Err(rustix::io::Errno::SRCH) {
                        time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("cancelled or timed-out ffprobe must be killed and reaped");
            }
        }
    }

    #[test]
    fn parses_basic_stream() {
        let raw = r#"{"streams":[{"width":1920,"height":1080,"sample_aspect_ratio":"1:1"}]}"#;
        let d = parse_ffprobe(raw).unwrap();
        assert_eq!(d.width, 1920);
        assert_eq!(d.height, 1080);
        assert_eq!(d.sar_num, 1);
        assert_eq!(d.sar_den, 1);
        assert_eq!(d.rotation_deg, 0);
    }

    #[test]
    fn parses_non_square_sar() {
        let raw = r#"{"streams":[{"width":720,"height":480,"sample_aspect_ratio":"10:11"}]}"#;
        let d = parse_ffprobe(raw).unwrap();
        assert_eq!((d.sar_num, d.sar_den), (10, 11));
    }

    #[test]
    fn unknown_sar_becomes_square() {
        for s in [r#""0:1""#, r#""""#, r#""bogus""#] {
            let raw = format!(r#"{{"streams":[{{"width":1,"height":1,"sample_aspect_ratio":{s}}}]}}"#);
            let d = parse_ffprobe(&raw).unwrap();
            assert_eq!((d.sar_num, d.sar_den), (1, 1), "raw={raw}");
        }
    }

    #[test]
    fn rotation_from_legacy_tag() {
        let raw = r#"{"streams":[{"width":100,"height":100,"tags":{"rotate":"90"}}]}"#;
        let d = parse_ffprobe(raw).unwrap();
        assert_eq!(d.rotation_deg, 90);
    }

    #[test]
    fn rotation_from_side_data_negative() {
        // Display matrix encodes -90 ≡ 270° clockwise.
        let raw = r#"{"streams":[{"width":100,"height":100,"side_data_list":[{"rotation":-90.0}]}]}"#;
        let d = parse_ffprobe(raw).unwrap();
        assert_eq!(d.rotation_deg, 270);
    }

    #[test]
    fn rotation_side_data_wins_over_tag() {
        let raw = r#"{
            "streams":[{
                "width":100,"height":100,
                "side_data_list":[{"rotation":180.0}],
                "tags":{"rotate":"90"}
            }]
        }"#;
        let d = parse_ffprobe(raw).unwrap();
        assert_eq!(d.rotation_deg, 180);
    }

    #[test]
    fn empty_streams_returns_none() {
        assert!(parse_ffprobe(r#"{"streams":[]}"#).is_none());
    }

    #[test]
    fn zero_dim_rejected() {
        let raw = r#"{"streams":[{"width":0,"height":1080}]}"#;
        assert!(parse_ffprobe(raw).is_none());
    }

    #[test]
    fn malformed_json_returns_none() {
        assert!(parse_ffprobe("not json").is_none());
    }

    #[test]
    fn rotation_normalisation_buckets() {
        assert_eq!(normalise_rotation(0.0), 0);
        assert_eq!(normalise_rotation(89.5), 90);
        assert_eq!(normalise_rotation(180.0), 180);
        assert_eq!(normalise_rotation(-90.0), 270);
        assert_eq!(normalise_rotation(360.0), 0);
        assert_eq!(normalise_rotation(720.0), 0);
    }

    fn ffprobe_available() -> bool {
        std::process::Command::new("ffprobe")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// End-to-end: spawn ffprobe against a known synthetic source and
    /// verify the parse + classification pipeline produces sensible
    /// numbers. Pins the JSON shape against the actual ffprobe binary.
    #[tokio::test]
    #[allow(clippy::print_stderr)]
    async fn probe_lavfi_testsrc_returns_dims() {
        if !ffprobe_available() {
            eprintln!("ffprobe not on PATH; skipping probe_lavfi_testsrc_returns_dims");
            return;
        }
        // We can't pass `-f lavfi` through `probe_video_metadata` (it
        // builds the args itself), so verify the full pipeline by
        // running ffprobe directly against a tiny generated input. The
        // shared parser is what we're really pinning here; the binary
        // call is just to keep the JSON shape honest against the
        // installed ffprobe version.
        let out = Command::new("ffprobe")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height,sample_aspect_ratio,side_data_list:stream_tags=rotate",
                "-of",
                "json",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=0.1:size=64x48:rate=10",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
            .expect("spawn ffprobe");
        assert!(out.status.success(), "ffprobe non-zero: {}", out.status);
        let raw = String::from_utf8(out.stdout).unwrap();
        let d = parse_ffprobe(&raw).expect("parse");
        assert_eq!((d.width, d.height), (64, 48));
        assert_eq!(d.rotation_deg, 0);
    }
}
