//! yt-dlp integration: pure helpers (format selector, output parsing) plus
//! the subprocess driver. Decoupled from the `Resolver` trait so anything in
//! the codebase can shell out to yt-dlp without going through the resolver
//! layer.
//!
//! Distribution assumption: `yt-dlp` and (for YouTube) `deno` are on `PATH`.
//! Recommended install:
//!
//! ```text
//! uv tool install 'yt-dlp[default,curl-cffi]'   # bundles yt-dlp-ejs
//! ```
//!
//! `[default]` includes `yt-dlp-ejs` which carries the JS bundle Deno
//! executes for YouTube's signature/n-sig challenges. Without it, YouTube
//! resolution fails. Other extractors don't need Deno.

pub mod format;
pub mod output;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::process::Command;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct YtDlp {
    bin: PathBuf,
    deno: Option<PathBuf>,
}

/// Extraction can launch Deno workers. Killing only yt-dlp leaves those
/// workers running after a timeout or stream cancellation.
#[cfg(unix)]
struct ProcessGroupGuard(Option<rustix::process::Pid>);

#[cfg(unix)]
impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
    }
}

#[derive(Debug, Error)]
pub enum YtDlpError {
    #[error("yt-dlp timed out")]
    Timeout,

    #[error("yt-dlp spawn failed: {0}")]
    Spawn(#[from] std::io::Error),

    /// yt-dlp exited non-zero. `message` is the last `ERROR:` line scraped
    /// from stderr, or a generic failure message if none was found.
    #[error("yt-dlp failed: {message}")]
    Failed { message: String, exit_code: Option<i32> },

    #[error("yt-dlp output parse: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

impl YtDlp {
    /// Look up `yt-dlp` and `deno` on `PATH`. Returns `None` if `yt-dlp` is
    /// missing — the caller decides whether that's fatal. A missing `deno`
    /// is not fatal here; only YouTube extraction fails without it, and we
    /// surface that as a runtime error per-resolution.
    pub fn detect() -> Option<Self> {
        let bin = which::which("yt-dlp").ok()?;
        let deno = which::which("deno").ok();
        if deno.is_none() {
            warn!(
                "yt-dlp present at {} but `deno` is not on PATH — YouTube extraction will fail",
                bin.display()
            );
        }
        Some(Self { bin, deno })
    }

    pub fn bin(&self) -> &Path {
        &self.bin
    }

    pub async fn resolve(
        &self,
        url: &str,
        format_expr: &str,
        timeout: Duration,
    ) -> Result<output::Info, YtDlpError> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(["--ignore-config", "-j", "--no-warnings", "--no-playlist", "-f"])
            .arg(format_expr)
            .kill_on_drop(true);

        if let Some(deno) = &self.deno {
            cmd.arg("--js-runtimes").arg(format!("deno:{}", deno.display()));
        }
        cmd.arg("--").arg(url);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        cmd.process_group(0);

        debug!(
            url = %url,
            format = %format_expr,
            timeout_ms = timeout.as_millis() as u64,
            "yt-dlp resolve start"
        );
        let started = Instant::now();
        let child = cmd.spawn()?;
        #[cfg(unix)]
        let mut process_group = ProcessGroupGuard(
            child
                .id()
                .and_then(|pid| i32::try_from(pid).ok())
                .and_then(rustix::process::Pid::from_raw),
        );
        let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => return Err(YtDlpError::Spawn(e)),
            Err(_) => {
                warn!(
                    url = %url,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    timeout_ms = timeout.as_millis() as u64,
                    "yt-dlp timeout fired"
                );
                return Err(YtDlpError::Timeout);
            }
        };
        // The child and its output pipes completed normally. No cancellation
        // cleanup remains, and the process group ID can now be reused.
        #[cfg(unix)]
        {
            process_group.0 = None;
        }
        debug!(
            url = %url,
            elapsed_ms = started.elapsed().as_millis() as u64,
            exit = ?out.status.code(),
            stderr_bytes = out.stderr.len(),
            "yt-dlp resolve done"
        );

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let message = stderr
                .lines()
                .rfind(|l| l.starts_with("ERROR:"))
                .map(str::to_string)
                .unwrap_or_else(|| {
                    // No `ERROR:` line — surface whatever stderr we got so the
                    // failure isn't opaque (argparse errors, deno crashes,
                    // proxy/CA issues all land here).
                    let trimmed = stderr.trim();
                    if trimmed.is_empty() {
                        "yt-dlp exited non-zero (no stderr)".to_string()
                    } else {
                        format!("yt-dlp exited non-zero: {trimmed}")
                    }
                });
            return Err(YtDlpError::Failed {
                message,
                exit_code: out.status.code(),
            });
        }

        let info: output::Info = serde_json::from_slice(&out.stdout)?;
        Ok(info)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(script: &str) -> (tempfile::TempDir, YtDlp) {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("yt-dlp");
        std::fs::write(&bin, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, YtDlp { bin, deno: None })
    }

    #[tokio::test]
    async fn subprocess_arguments_and_headers_round_trip() {
        let (_dir, mut driver) = executable(
            r#"
[ "$1" = '--ignore-config' ] || exit 1
[ "$6" = 'best[height<=64]' ] || exit 2
[ "$7" = '--js-runtimes' ] || exit 3
[ "$8" = 'deno:/a path/deno' ] || exit 4
[ "$9" = '--' ] || exit 5
shift 9
[ "$1" = '--not-an-option' ] || exit 6
printf '%s' '{"url":"https://cdn.example/video","fps":60,"filesize":123,"http_headers":{"Referer":"https://example.com/"}}'
"#,
        );
        driver.deno = Some("/a path/deno".into());
        let info = driver
            .resolve("--not-an-option", "best[height<=64]", Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(info.fps, Some(60.0));
        assert_eq!(info.filesize, Some(123));
        assert_eq!(info.http_headers.unwrap()["Referer"], "https://example.com/");
    }

    #[tokio::test]
    async fn subprocess_errors_and_deadlines_surface() {
        let (_dir, driver) = executable("echo 'ERROR: extraction failed' >&2; exit 7");
        assert!(matches!(
            driver
                .resolve("https://example.com", "best", Duration::from_secs(5))
                .await,
            Err(YtDlpError::Failed {
                exit_code: Some(7),
                ..
            })
        ));
        let (_dir, driver) = executable("exec sleep 30");
        assert!(matches!(
            driver
                .resolve("https://example.com", "best", Duration::from_millis(50))
                .await,
            Err(YtDlpError::Timeout)
        ));
    }

    struct ChildCleanup(rustix::process::Pid);

    impl Drop for ChildCleanup {
        fn drop(&mut self) {
            // Keep a failing regression from leaving its sleep process behind.
            let _ = rustix::process::kill_process(self.0, rustix::process::Signal::KILL);
        }
    }

    async fn spawned_worker(bin: &Path) -> ChildCleanup {
        let pid_file = bin.with_extension("pid");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(&pid_file).await
                    && let Ok(raw) = text.trim().parse::<i32>()
                    && let Some(pid) = rustix::process::Pid::from_raw(raw)
                {
                    return ChildCleanup(pid);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("extractor must start its worker")
    }

    async fn assert_worker_stopped(worker: &ChildCleanup) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if rustix::process::test_kill_process(worker.0) == Err(rustix::io::Errno::SRCH) {
                    return;
                }
                // Container PID 1 may not reap orphaned children promptly.
                // A zombie has exited and cannot execute or retain pipes.
                #[cfg(target_os = "linux")]
                if let Ok(stat) =
                    tokio::fs::read_to_string(format!("/proc/{}/stat", worker.0.as_raw_pid())).await
                    && stat
                        .rsplit_once(')')
                        .is_some_and(|(_, fields)| fields.trim_start().starts_with('Z'))
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("extractor worker must exit with its parent");
    }

    #[tokio::test]
    async fn subprocess_timeout_kills_worker() {
        let (_dir, driver) = executable("sleep 30 &\nprintf '%s\\n' \"$!\" > \"$0.pid\"\nwait");
        let bin = driver.bin.clone();
        let task = tokio::spawn(async move {
            driver
                .resolve("https://example.com", "best", Duration::from_millis(300))
                .await
        });
        let worker = spawned_worker(&bin).await;
        assert!(matches!(task.await.unwrap(), Err(YtDlpError::Timeout)));
        assert_worker_stopped(&worker).await;
    }

    #[tokio::test]
    async fn subprocess_cancellation_kills_worker() {
        let (_dir, driver) = executable("sleep 30 &\nprintf '%s\\n' \"$!\" > \"$0.pid\"\nwait");
        let bin = driver.bin.clone();
        let task = tokio::spawn(async move {
            driver
                .resolve("https://example.com", "best", Duration::from_secs(30))
                .await
        });
        let worker = spawned_worker(&bin).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_worker_stopped(&worker).await;
    }
}
