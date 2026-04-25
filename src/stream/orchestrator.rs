//! Orchestrator: spawn per-stream tasks, own the DDP registry, hand back
//! `StreamHandle`s for session-level cancellation.

use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{error, info, warn};

use crate::Config;
use crate::control::fields::StreamFields;
use crate::error::StreamError;
use crate::image::animated::cache::FrameCache;
use crate::output::ddp::{DdpKey, DdpRegistry, DdpSender};
use crate::output::sink::{OutputSink, StreamId};
use crate::resolver::Resolver;
use crate::stream::handle::StreamHandle;
use crate::stream::runner;

/// Upper bound on retry attempts for transient errors (network failure,
/// YouTube URL expiry, retryable decode faults).
const MAX_RETRIES: u32 = 3;

pub struct Orchestrator {
    pub config: Arc<Config>,
    pub ddp_registry: DdpRegistry,
    pub resolver: Arc<dyn Resolver>,
    pub frame_cache: FrameCache,
}

impl Orchestrator {
    pub fn new(config: Arc<Config>, resolver: Arc<dyn Resolver>) -> Self {
        let frame_cache = FrameCache::new(config.image.frame_cache_mb, config.image.frame_cache_min_frames);
        Self {
            config,
            ddp_registry: DdpRegistry::spawn(),
            resolver,
            frame_cache,
        }
    }

    /// Spawn a stream task for `fields`. Returns a handle the caller uses
    /// to cancel it. On cancellation or error, the task cleans itself up
    /// via `Drop` semantics (reservation, sink, ffmpeg child).
    pub async fn spawn_stream(self: &Arc<Self>, fields: StreamFields) -> Result<StreamHandle, StreamError> {
        let stream_id = StreamId::new();
        let handle = StreamHandle::new(stream_id, fields.clone());
        let cancel_handle = handle.clone();
        let config = self.config.clone();
        let ddp_registry = self.ddp_registry.clone();
        let resolver = self.resolver.clone();
        let frame_cache = self.frame_cache.clone();

        let _task: JoinHandle<()> = tokio::spawn(async move {
            let result = run_stream(
                fields,
                stream_id,
                cancel_handle.clone(),
                ddp_registry,
                config,
                frame_cache,
                resolver,
            )
            .await;
            match result {
                Ok(()) => info!(%stream_id, "stream finished"),
                Err(e) => error!(%stream_id, error = %e, "stream failed"),
            }
        });

        Ok(handle)
    }
}

/// Execute one stream lifecycle. The task exits on cancellation, when the
/// source ends (non-looping), or on error.
async fn run_stream(
    fields: StreamFields,
    stream_id: StreamId,
    handle: StreamHandle,
    ddp_registry: DdpRegistry,
    config: Arc<Config>,
    frame_cache: FrameCache,
    resolver: Arc<dyn Resolver>,
) -> Result<(), StreamError> {
    // Reserve DDP address — this displaces any previous stream on the same key.
    let key = DdpKey {
        dest: fields.ddp_host,
        output_id: crate::control::fields::output_id_byte(fields.output_id),
    };
    let (_reservation, cancel_rx) = ddp_registry
        .reserve(key, stream_id)
        .await
        .map_err(StreamError::Output)?;

    // Build DDP sender.
    let sender: Arc<dyn OutputSink> = Arc::new(
        DdpSender::bind(
            fields.ddp_host,
            fields.ddp_port,
            crate::control::fields::output_id_byte(fields.output_id),
            fields.fmt,
            &config,
        )
        .await
        .map_err(StreamError::Output)?,
    );
    let sink = sender;

    tokio::select! {
        r = run_with_retry(&fields, &config, &frame_cache, &resolver, sink.as_ref()) => r?,
        () = handle.cancelled() => return Err(StreamError::Cancelled),
        _ = cancel_rx => return Err(StreamError::Cancelled),
    }

    sink.close().await.map_err(StreamError::Output)?;
    Ok(())
}

/// Build the source and run it, retrying on transient errors with
/// exponential backoff (0.5 → 1 → 2 s, ±50 ms jitter). Matches the
/// YouTube-URL-expiry and network-blip recovery path.
async fn run_with_retry(
    fields: &StreamFields,
    config: &Config,
    frame_cache: &FrameCache,
    resolver: &Arc<dyn Resolver>,
    sink: &dyn OutputSink,
) -> Result<(), StreamError> {
    let mut attempt: u32 = 0;
    loop {
        let source = match build_source(fields, config, frame_cache, resolver).await {
            Ok(s) => s,
            Err(e) if is_retryable(&e) && attempt < MAX_RETRIES => {
                let delay = backoff(attempt);
                warn!(attempt, error = %e, backoff_ms = delay.as_millis() as u64, "build_source retry");
                time::sleep(delay).await;
                attempt += 1;
                continue;
            }
            Err(e) => return Err(e),
        };

        let outcome = if fields.pace > 0 {
            runner::run_paced(source, sink, fields).await
        } else {
            runner::run_native(source, sink, fields).await
        };

        match outcome {
            Ok(()) => {
                // Non-cached video can only loop by rebuilding from scratch.
                if fields.r#loop {
                    attempt = 0;
                    continue;
                }
                return Ok(());
            }
            Err(e) if is_retryable(&e) && attempt < MAX_RETRIES => {
                let delay = backoff(attempt);
                warn!(attempt, error = %e, backoff_ms = delay.as_millis() as u64, "stream retry");
                time::sleep(delay).await;
                attempt += 1;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
}

fn is_retryable(e: &StreamError) -> bool {
    match e {
        StreamError::Media(m) => m.is_retryable(),
        StreamError::Video(crate::error::VideoError::Media(m)) => m.is_retryable(),
        StreamError::Image(crate::error::ImageError::Media(m)) => m.is_retryable(),
        StreamError::Resolver(_) => true,
        _ => false,
    }
}

fn backoff(attempt: u32) -> Duration {
    // 0.5s * 2^n, capped at 5s, plus 0..50ms jitter.
    let base_ms = 500u64.saturating_mul(1 << attempt).min(5000);
    let jitter_ms = rand::rng().random_range(0..50);
    Duration::from_millis(base_ms + jitter_ms)
}

async fn build_source(
    fields: &StreamFields,
    config: &Config,
    frame_cache: &FrameCache,
    resolver: &Arc<dyn Resolver>,
) -> Result<crate::stream::frame_source::FrameSource, StreamError> {
    if crate::stream::url::classify(&fields.source).is_video() {
        crate::video::dispatch::build_video_source(fields, config, resolver).await
    } else {
        crate::image::dispatch::build_image_source(fields, config, frame_cache).await
    }
}
