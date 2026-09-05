//! Extracts dimensions, sample aspect ratio, and rotation for video filters.
//! Probe failures return no metadata so playback can proceed with target-only fitting.

use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;
use tokio::time;
use tracing::{debug, warn};

use crate::stream::url::is_http_url;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// HTTP read timeout, shorter than the outer probe deadline.
const HTTP_RW_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoProbeData {
    /// Coded width before rotation; see [`Self::display_dims`] for display orientation.
    pub width: u32,
    pub height: u32,
    pub sar_num: u32,
    pub sar_den: u32,
    /// Clockwise rotation in degrees, rounded to a quarter-turn by the parser.
    pub rotation_deg: u32,
}

impl VideoProbeData {
    /// Returns coded dimensions before rotation.
    pub fn dims(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Returns dimensions after rotation, matching ffmpeg's decoded orientation.
    pub fn display_dims(&self) -> (u32, u32) {
        if matches!(self.rotation_deg, 90 | 270) {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

/// Returns metadata, or `None` on timeout, process failure, or unusable output.
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

/// Parses sample aspect ratio; missing, invalid, or zero values mean square pixels.
fn parse_sar(s: Option<&str>) -> (u32, u32) {
    let Some(s) = s else { return (1, 1) };
    let Some((n, d)) = s.split_once(':') else {
        return (1, 1);
    };
    let n: u32 = n.parse().unwrap_or(0);
    let d: u32 = d.parse().unwrap_or(0);
    if n == 0 || d == 0 { (1, 1) } else { (n, d) }
}

/// Prefers display-matrix rotation over the string-valued `rotate` tag.
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

/// Rounds rotation to a clockwise quarter-turn, accepting negative degrees.
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
        // Display matrix encodes -90 = 270 degrees clockwise.
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

    #[tokio::test]
    #[allow(clippy::print_stderr)]
    async fn probe_lavfi_testsrc_returns_dims() {
        if !ffprobe_available() {
            eprintln!("ffprobe not on PATH; skipping probe_lavfi_testsrc_returns_dims");
            return;
        }
        // The synthetic input needs lavfi options outside the production probe interface.
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
