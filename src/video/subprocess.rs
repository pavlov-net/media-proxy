//! Streams RGB24 frames from ffmpeg stdout and reads PTS from stderr.
//! The CLI isolates decoding from the Rust process and avoids libav binding dependencies.

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

/// Owns frame and completion receivers plus the ffmpeg cancellation guard.
/// Completion carries classified stderr errors for failed processes.
pub(crate) struct FfmpegHandles {
    pub frames: mpsc::Receiver<FfmpegFrame>,
    pub completion: oneshot::Receiver<Result<(), MediaError>>,
    /// Must live with the source; dropping it tells the wait task to kill ffmpeg.
    pub(crate) kill_guard: KillGuard,
}

/// Signals cancellation on drop. The wait task owns `Child`, so child
/// `kill_on_drop` alone cannot terminate a stalled source when its consumer exits.
pub(crate) struct KillGuard(#[allow(dead_code)] oneshot::Sender<std::convert::Infallible>);

impl KillGuard {
    /// Returns a guard without a worker, for synthetic test sources.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        let (tx, _rx) = oneshot::channel::<std::convert::Infallible>();
        Self(tx)
    }
}

/// Detects hung connections so reconnect can fire instead of stalling.
const HTTP_RW_TIMEOUT: Duration = Duration::from_secs(10);

/// Configures bounded HTTP reads and retries so transport failures do not
/// become successful partial-file EOF. Autocrop uses a shorter retry delay.
pub(crate) fn add_http_reconnect_args(cmd: &mut Command, rw_timeout: Duration, delay_max_secs: u32) {
    let timeout_us = rw_timeout.as_micros().to_string();
    let delay = delay_max_secs.to_string();
    cmd.arg("-reconnect")
        .arg("1")
        .arg("-reconnect_streamed")
        .arg("1")
        // Reconnecting at normal EOF stalls finite files. Looping live inputs that
        // close cleanly restart through the orchestrator; transport failures retry here.
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

/// Couples seekable `cache:` input with `-stream_loop -1` for cached playback.
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

/// Starts RGB24 frame delivery. Drop the returned kill guard to stop ffmpeg.
pub(crate) async fn spawn_ffmpeg(args: FfmpegArgs<'_>) -> Result<FfmpegHandles, VideoError> {
    let source_url = args.input.url().to_string();
    let frame_size = (args.output_width as usize) * (args.output_height as usize) * 3;

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-nostdin")
        // `showinfo` logs PTS at INFO; lower levels remove frame timing.
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

    // Read timing after output filters so PTS corresponds to emitted frames.
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

    // The wait task reads classified errors recorded by the stderr reader.
    let last_err: LastStderrErr = Arc::new(Mutex::new(None));

    let (pts_tx, pts_rx) = mpsc::channel::<f64>(64);
    tokio::spawn(stderr_reader(stderr, pts_tx, last_err.clone()));

    tokio::spawn(frame_reader(stdout, frame_size, tx, pts_rx));

    let (completion_tx, completion_rx) = oneshot::channel();
    let (kill_tx, mut kill_rx) = oneshot::channel::<std::convert::Infallible>();
    tokio::spawn(async move {
        // Cancellation must kill and reap even when ffmpeg is blocked on input.
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
                    // Unclassified failures use the orchestrator's bounded retry policy.
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
        // Error severity controls the orchestrator's retry decision.
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

/// Keeps the first permanent error so a later connection teardown cannot mask it.
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

    #[tokio::test]
    #[allow(clippy::print_stderr)]
    async fn loglevel_info_emits_showinfo_pts_lines() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH; skipping loglevel_info_emits_showinfo_pts_lines");
            return;
        }
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
