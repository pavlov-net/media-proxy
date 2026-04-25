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
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::process::Command;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct YtDlp {
    bin: PathBuf,
    deno: Option<PathBuf>,
    /// Captured at construction; passed to `--ca-certs` so yt-dlp respects
    /// custom CA bundles in sandboxes / TLS-intercepting proxies. yt-dlp
    /// otherwise uses bundled `certifi` and ignores `SSL_CERT_FILE`.
    ca_bundle: Option<PathBuf>,
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
        let ca_bundle = std::env::var_os("SSL_CERT_FILE")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty());
        Some(Self { bin, deno, ca_bundle })
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
        cmd.args(["-j", "--no-warnings", "--no-playlist", "-f"])
            .arg(format_expr)
            .arg(url)
            .kill_on_drop(true);

        if let Some(deno) = &self.deno {
            cmd.arg("--js-runtimes").arg(format!("deno:{}", deno.display()));
        }
        if let Some(bundle) = &self.ca_bundle {
            cmd.arg("--ca-certs").arg(bundle);
        }

        debug!(
            url = %url,
            format = %format_expr,
            timeout_ms = timeout.as_millis() as u64,
            "yt-dlp resolve start"
        );
        let started = Instant::now();
        let out = match tokio::time::timeout(timeout, cmd.output()).await {
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
                .unwrap_or_else(|| "yt-dlp exited non-zero".to_string());
            return Err(YtDlpError::Failed {
                message,
                exit_code: out.status.code(),
            });
        }

        let info: output::Info = serde_json::from_slice(&out.stdout)?;
        Ok(info)
    }
}
