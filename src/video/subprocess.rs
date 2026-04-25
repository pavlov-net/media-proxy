//! Spawn ffmpeg via `tokio::process`, pipe RGB24 frames, parse timing.
//!
//! Architectural choices:
//! - **Subprocess, not in-process libav.** Rust bindings to FFmpeg are thin
//!   and solo-maintained; the CLI has been API-stable for a decade.
//! - **`-f rawvideo -pix_fmt rgb24`** on stdout — one byte frame of
//!   `w * h * 3` per frame.
//! - **`-vf showinfo`** in stderr for per-frame PTS. The parser matches on
//!   `pts_time:<float>`.

use std::process::Stdio;
use std::time::Duration;

use bytes::Bytes;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout, Command};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::error::VideoError;
use crate::platform::HwBackend;
use crate::stream::url::is_http_url;

/// Detects hung connections so reconnect can fire instead of stalling.
const HTTP_RW_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct FfmpegFrame {
    pub rgb24: Bytes,
    pub pts_s: Option<f64>,
}

/// Input source for ffmpeg. `CachedLooping` ties together `cache:` (which
/// makes the input seekable) and `-stream_loop -1` (which needs that
/// seekability) so they can't be enabled independently and silently fail.
pub enum FfmpegInput<'a> {
    Direct(&'a str),
    CachedLooping(&'a str),
}

impl FfmpegInput<'_> {
    fn url(&self) -> &str {
        match self {
            Self::Direct(u) | Self::CachedLooping(u) => u,
        }
    }
}

pub struct FfmpegArgs<'a> {
    pub input: FfmpegInput<'a>,
    pub filter_graph: &'a str,
    pub output_width: u32,
    pub output_height: u32,
    pub hw: Option<HwBackend>,
    /// CRLF-joined `K: V` pairs for ffmpeg's `-headers`, if any.
    pub http_headers: Option<String>,
}

/// Spawn ffmpeg and stream RGB24 frames over a channel. Caller drops the
/// channel to stop.
pub async fn spawn_ffmpeg(args: FfmpegArgs<'_>) -> Result<mpsc::Receiver<FfmpegFrame>, VideoError> {
    let frame_size = (args.output_width as usize) * (args.output_height as usize) * 3;

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-nostdin")
        .arg("-loglevel")
        .arg("error")
        .arg("-fflags")
        .arg("+genpts+discardcorrupt");

    if let Some(b) = args.hw {
        cmd.arg("-hwaccel").arg(b.as_ffmpeg_flag());
    }

    // Without reconnect, a mid-stream TLS reset is treated as EOF and ffmpeg
    // exits 0 with a partial file. The `cache:` wrapper doesn't shield this.
    if is_http_url(args.input.url()) {
        let timeout_us = HTTP_RW_TIMEOUT.as_micros().to_string();
        cmd.args([
            "-reconnect",
            "1",
            "-reconnect_streamed",
            "1",
            "-reconnect_at_eof",
            "1",
            "-reconnect_delay_max",
            "5",
            "-rw_timeout",
            &timeout_us,
        ]);
    }
    if matches!(args.input, FfmpegInput::CachedLooping(_)) {
        cmd.args(["-stream_loop", "-1"]);
    }
    if let Some(headers) = &args.http_headers {
        cmd.arg("-headers").arg(headers);
    }

    let input_arg = match &args.input {
        FfmpegInput::Direct(u) => (*u).to_string(),
        FfmpegInput::CachedLooping(u) => format!("cache:{u}"),
    };
    cmd.arg("-i").arg(&input_arg);

    // Showinfo first so PTS logs come out before the output filters.
    let vf = format!("{},showinfo", args.filter_graph);
    cmd.arg("-vf").arg(&vf);

    cmd.arg("-f")
        .arg("rawvideo")
        .arg("-pix_fmt")
        .arg("rgb24")
        .arg("-an")
        .arg("pipe:1");

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);

    debug!(?cmd, "spawning ffmpeg");

    let mut child = cmd.spawn().map_err(|e| VideoError::Ffmpeg(e.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| VideoError::Ffmpeg("stdout missing".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| VideoError::Ffmpeg("stderr missing".into()))?;

    let (tx, rx) = mpsc::channel::<FfmpegFrame>(8);

    // Collect PTS values from stderr into a shared queue.
    let (pts_tx, pts_rx) = mpsc::channel::<f64>(64);
    tokio::spawn(stderr_reader(stderr, pts_tx));

    tokio::spawn(frame_reader(stdout, frame_size, tx, pts_rx));

    tokio::spawn(async move {
        if let Ok(status) = child.wait().await {
            debug!(%status, "ffmpeg exited");
        }
    });

    Ok(rx)
}

async fn frame_reader(
    mut stdout: ChildStdout,
    frame_size: usize,
    tx: mpsc::Sender<FfmpegFrame>,
    mut pts_rx: mpsc::Receiver<f64>,
) {
    loop {
        let mut buf = vec![0u8; frame_size];
        if stdout.read_exact(&mut buf).await.is_err() {
            return;
        }
        let pts_s = pts_rx.try_recv().ok();
        if tx
            .send(FfmpegFrame {
                rgb24: Bytes::from(buf),
                pts_s,
            })
            .await
            .is_err()
        {
            return;
        }
    }
}

async fn stderr_reader(stderr: ChildStderr, tx: mpsc::Sender<f64>) {
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        // Look for `pts_time:1.234` in the showinfo output.
        if let Some(idx) = line.find("pts_time:") {
            let rest = &line[idx + "pts_time:".len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                .unwrap_or(rest.len());
            if let Ok(v) = rest[..end].parse::<f64>() {
                let _ = tx.send(v).await;
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Classify common ffmpeg stderr lines so operators don't see every
        // transient network blip as a `warn!`.
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("http error 4")
            || lower.contains("server returned 4")
            || lower.contains("invalid data")
        {
            warn!(line = trimmed, "ffmpeg permanent error");
        } else if lower.contains("http error")
            || lower.contains("i/o error")
            || lower.contains("connection refused")
            || lower.contains("connection reset")
            || lower.contains("timed out")
        {
            warn!(line = trimmed, "ffmpeg transient error");
        } else {
            debug!(line = trimmed, "ffmpeg");
        }
    }
}
