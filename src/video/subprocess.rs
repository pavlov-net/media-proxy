//! Spawn ffmpeg via `tokio::process`, pipe RGB24 frames, parse timing.
//!
//! Architectural choices:
//! - **Subprocess, not in-process libav.** Rust bindings to FFmpeg are thin
//!   and solo-maintained; the CLI has been API-stable for a decade.
//! - **`-f rawvideo -pix_fmt rgb24`** on stdout — one byte frame of
//!   `w * h * 3` per frame.
//! - **`-vf showinfo`** in stderr for per-frame PTS. The parser matches on
//!   `pts_time:<float>`. `showinfo` emits at `AV_LOG_INFO`, so we must run
//!   ffmpeg at `-loglevel info` (or higher) — at `error` the lines are
//!   suppressed and every frame falls back to a 33ms default delay.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use crate::error::{MediaError, VideoError};
use crate::platform::HwBackend;
use crate::stream::url::is_http_url;

#[derive(Debug, Clone, Copy)]
enum ErrSeverity {
    Permanent,
    Transient,
}

type LastStderrErr = Arc<Mutex<Option<(ErrSeverity, String)>>>;

/// Channels handed back to the dispatch layer when ffmpeg starts. The
/// `completion` receiver yields the final result once ffmpeg exits — `Ok`
/// for clean exit, `Err(MediaError)` carrying the classified stderr line
/// (e.g. "Server returned 403 Forbidden") for failures.
///
/// Dropping the handles triggers an explicit ffmpeg kill via `_kill_tx`:
/// the wait task selects on `child.wait()` vs the kill receiver, and
/// dropping the sender resolves the receiver with `Err(RecvError)` on the
/// same scheduler tick. Without this, a stalled HTTP read inside ffmpeg
/// can keep the child alive long after the Rust side has cancelled —
/// `kill_on_drop(true)` only fires when `Child` itself drops, but we move
/// it into the wait task to call `wait()`. The kill channel re-establishes
/// drop-driven termination.
pub(crate) struct FfmpegHandles {
    pub frames: mpsc::Receiver<FfmpegFrame>,
    pub completion: oneshot::Receiver<Result<(), MediaError>>,
    /// Drop guard the wait task watches: when this sender drops, the wait
    /// task explicit-kills the ffmpeg child. The dispatch layer must move
    /// it into the `VideoSource` (or another long-lived owner) so it
    /// outlives the FrameSource consumer.
    pub(crate) kill_guard: KillGuard,
}

/// Newtype wrapping the kill-channel sender. Carries no value — its only
/// observable effect is when it drops, which resolves the matching
/// receiver with `Err(RecvError)` and triggers the kill in the wait task.
/// The inner sender is intentionally unread; the field exists only to be
/// dropped.
pub(crate) struct KillGuard(#[allow(dead_code)] oneshot::Sender<std::convert::Infallible>);

impl KillGuard {
    /// Construct a guard whose drop has no observable effect — the matching
    /// receiver is dropped immediately. Used for tests that build a
    /// `VideoSource` without a real ffmpeg subprocess behind it.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        let (tx, _rx) = oneshot::channel::<std::convert::Infallible>();
        Self(tx)
    }
}

/// Detects hung connections so reconnect can fire instead of stalling.
const HTTP_RW_TIMEOUT: Duration = Duration::from_secs(10);

/// Apply ffmpeg's HTTP reconnect/timeout flags to a `Command`. Used by
/// the main spawn and the autocrop probe so a mid-stream TLS reset
/// doesn't masquerade as a clean EOF (without reconnect, ffmpeg exits 0
/// on the partial file). The two callers differ on `delay_max_secs` —
/// the main pipeline gives reconnects more headroom, the probe wants to
/// fail fast — but the rest of the chain is identical.
pub(crate) fn add_http_reconnect_args(cmd: &mut Command, rw_timeout: Duration, delay_max_secs: u32) {
    let timeout_us = rw_timeout.as_micros().to_string();
    let delay = delay_max_secs.to_string();
    cmd.arg("-reconnect")
        .arg("1")
        .arg("-reconnect_streamed")
        .arg("1")
        .arg("-reconnect_at_eof")
        .arg("1")
        .arg("-reconnect_delay_max")
        .arg(&delay)
        .arg("-rw_timeout")
        .arg(&timeout_us);
}

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
pub(crate) async fn spawn_ffmpeg(args: FfmpegArgs<'_>) -> Result<FfmpegHandles, VideoError> {
    let source_url = args.input.url().to_string();
    let frame_size = (args.output_width as usize) * (args.output_height as usize) * 3;

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-nostdin")
        // `info` is required for `showinfo` (AV_LOG_INFO) to reach the
        // stderr parser. The extra startup metadata at info level is
        // harmless: the stderr classifier debug-routes anything it doesn't
        // recognise as a known warning class.
        .arg("-loglevel")
        .arg("info")
        .arg("-fflags")
        .arg("+genpts+discardcorrupt");

    if let Some(b) = args.hw {
        cmd.arg("-hwaccel").arg(b.as_ffmpeg_flag());
    }

    if is_http_url(args.input.url()) {
        add_http_reconnect_args(&mut cmd, HTTP_RW_TIMEOUT, 5);
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

    // Shared slot the stderr reader fills with the most relevant ffmpeg
    // error line (first permanent wins; transient is overwritten freely).
    // The wait task reads it on exit to construct a structured error.
    let last_err: LastStderrErr = Arc::new(Mutex::new(None));

    // Collect PTS values from stderr into a shared queue.
    let (pts_tx, pts_rx) = mpsc::channel::<f64>(64);
    tokio::spawn(stderr_reader(stderr, pts_tx, last_err.clone()));

    tokio::spawn(frame_reader(stdout, frame_size, tx, pts_rx));

    let (completion_tx, completion_rx) = oneshot::channel();
    let (kill_tx, mut kill_rx) = oneshot::channel::<std::convert::Infallible>();
    tokio::spawn(async move {
        // Two ways to leave: ffmpeg exits on its own, or the handles drop
        // and `kill_rx` resolves (with Err since the sender is never used
        // to send). On kill, we explicit-kill and reap; we skip emitting
        // a completion result because the consumer has already gone away.
        let status = tokio::select! {
            s = child.wait() => Some(s),
            _ = &mut kill_rx => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                None
            }
        };
        let Some(status) = status else {
            return;
        };
        let last = last_err.lock().take();
        let result: Result<(), MediaError> = match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => {
                let (message, retryable) = match last {
                    Some((ErrSeverity::Permanent, m)) => (m, false),
                    Some((ErrSeverity::Transient, m)) => (m, true),
                    // No classified line — treat as transient and let the
                    // orchestrator's retry budget cap it.
                    None => (format!("ffmpeg exited {s}"), true),
                };
                debug!(%s, retryable, %message, "ffmpeg exited with error");
                Err(MediaError::Network {
                    source_url: source_url.clone(),
                    message,
                    error_code: s.code(),
                    retryable,
                })
            }
            Err(e) => Err(MediaError::Network {
                source_url: source_url.clone(),
                message: format!("waiting for ffmpeg: {e}"),
                error_code: None,
                retryable: true,
            }),
        };
        let _ = completion_tx.send(result);
    });

    Ok(FfmpegHandles {
        frames: rx,
        completion: completion_rx,
        kill_guard: KillGuard(kill_tx),
    })
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

async fn stderr_reader(stderr: ChildStderr, tx: mpsc::Sender<f64>, last_err: LastStderrErr) {
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
        // transient network blip as a `warn!`. The classification also feeds
        // the wait-task's structured error: permanent → non-retryable.
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("http error 4")
            || lower.contains("server returned 4")
            || lower.contains("invalid data")
        {
            warn!(line = trimmed, "ffmpeg permanent error");
            record_stderr_err(&last_err, ErrSeverity::Permanent, trimmed);
        } else if lower.contains("http error")
            || lower.contains("i/o error")
            || lower.contains("connection refused")
            || lower.contains("connection reset")
            || lower.contains("timed out")
        {
            warn!(line = trimmed, "ffmpeg transient error");
            record_stderr_err(&last_err, ErrSeverity::Transient, trimmed);
        } else {
            debug!(line = trimmed, "ffmpeg");
        }
    }
}

/// First permanent line wins; later lines (permanent or transient) are
/// dropped once a permanent is set. This ensures a 403 line isn't masked
/// by a later "connection reset" the OS emits as the socket tears down.
fn record_stderr_err(slot: &LastStderrErr, sev: ErrSeverity, line: &str) {
    let mut g = slot.lock();
    if matches!(&*g, Some((ErrSeverity::Permanent, _))) {
        return;
    }
    *g = Some((sev, line.to_string()));
}

#[cfg(test)]
mod tests {
    use std::process::Stdio;
    use tokio::io::AsyncBufReadExt;
    use tokio::process::Command;

    fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Pins the assumption that drives `spawn_ffmpeg`'s log-level choice:
    /// `showinfo` emits at `AV_LOG_INFO`, so the loglevel must be at least
    /// `info` for the per-frame `pts_time:` lines to reach our stderr
    /// parser. The previous setting of `error` silently suppressed every
    /// PTS line and demoted timing to a fixed 33ms default. Reads enough
    /// stderr to see at least two distinct pts values, then aborts ffmpeg.
    #[tokio::test]
    #[allow(clippy::print_stderr)]
    async fn loglevel_info_emits_showinfo_pts_lines() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH; skipping loglevel_info_emits_showinfo_pts_lines");
            return;
        }
        // 0.3s of 10fps synthetic input = 3 frames. Tiny size keeps the
        // test fast (<200ms in practice).
        let mut child = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "info",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=0.3:size=16x16:rate=10",
                "-vf",
                "showinfo",
                "-f",
                "null",
                "-",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn ffmpeg");

        let stderr = child.stderr.take().expect("stderr piped");
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        let mut pts_values: Vec<f64> = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(idx) = line.find("pts_time:") {
                let rest = &line[idx + "pts_time:".len()..];
                let end = rest
                    .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                    .unwrap_or(rest.len());
                if let Ok(v) = rest[..end].parse::<f64>() {
                    pts_values.push(v);
                    if pts_values.len() >= 2 {
                        break;
                    }
                }
            }
        }
        let _ = child.wait().await;
        assert!(
            pts_values.len() >= 2,
            "expected >=2 distinct pts_time: lines, got {pts_values:?} — \
             showinfo loglevel regression?"
        );
        assert!(
            pts_values[1] > pts_values[0],
            "pts must advance: got {pts_values:?}"
        );
    }
}
