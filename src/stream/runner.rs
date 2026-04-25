//! Per-stream runners: native cadence and paced mode.
//!
//! Native runs at the source's frame timing; paced samples the latest frame
//! at a fixed `pace_hz` with optional EMA blending. Both reset their
//! deadline when more than 100ms behind to prevent runaway catch-up bursts.

use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::Mutex;
use tokio::time;
use tracing::{debug, warn};

use crate::control::fields::StreamFields;
use crate::error::StreamError;
use crate::output::sink::{Frame, FrameMeta, OutputSink};
use crate::stream::frame_source::{FrameSource, RgbFrame};

/// Upper bound on time between frames — if the source stalls this long we
/// cut the stream and let the orchestrator decide whether to restart.
const FRAME_WATCHDOG: Duration = Duration::from_secs(30);

/// Run at the source's native cadence. One frame pulled → one frame emitted,
/// with timing based on the frame's `delay_ms` hint.
pub async fn run_native(
    mut source: FrameSource,
    sink: &dyn OutputSink,
    fields: &StreamFields,
) -> Result<(), StreamError> {
    let mut seq: u32 = 0;
    let mut next_frame_time = Instant::now();
    let mut emitted = 0u64;

    loop {
        let pulled = time::timeout(FRAME_WATCHDOG, source.next()).await;
        let frame = match pulled {
            Ok(Some(f)) => f,
            Ok(None) => {
                debug!(emitted, "native source exhausted");
                return Ok(());
            }
            Err(_) => {
                warn!("frame watchdog tripped (>30s idle)");
                return Err(StreamError::Cancelled);
            }
        };
        let RgbFrame { rgb888, delay_ms } = frame;

        let is_first_still = !fields.r#loop && emitted == 0;
        let meta = FrameMeta {
            sequence: seq,
            delay_ms,
            width: fields.width,
            height: fields.height,
            format: fields.fmt,
            is_still: is_first_still,
            is_last_frame: is_first_still,
        };
        sink.send_frame(Frame { data: rgb888, meta })
            .await
            .map_err(StreamError::Output)?;
        emitted += 1;
        seq = seq.wrapping_add(1);

        next_frame_time += Duration::from_secs_f32(delay_ms.max(10.0) / 1000.0);
        let now = Instant::now();
        if next_frame_time + Duration::from_millis(100) < now {
            next_frame_time = now;
        } else if next_frame_time > now {
            time::sleep(next_frame_time - now).await;
        }
    }
}

/// Paced mode: producer pushes latest frame into a shared slot at source
/// cadence; sampler emits `pace_hz` times per second, optionally EMA-blended
/// against the previous emit.
pub async fn run_paced(
    source: FrameSource,
    sink: &dyn OutputSink,
    fields: &StreamFields,
) -> Result<(), StreamError> {
    let pace_hz = fields.pace.max(1) as f32;
    let ema_alpha = fields.ema.clamp(0.0, 1.0);
    let tick = Duration::from_secs_f32(1.0 / pace_hz);

    let latest: Mutex<Option<Bytes>> = Mutex::new(None);

    let producer = async {
        let mut source = source;
        while let Some(f) = source.next().await {
            *latest.lock().await = Some(f.rgb888);
        }
    };

    let sampler = async {
        let mut next = Instant::now();
        let mut seq: u32 = 0;
        let mut ema_buf: Option<Vec<f32>> = None;
        let mut out_buf: Vec<u8> = Vec::new();

        loop {
            let snap = latest.lock().await.clone();
            if let Some(frame) = snap {
                let data = if ema_alpha > 0.0 {
                    let src = &frame;
                    let buf = match &mut ema_buf {
                        Some(b) if b.len() == src.len() => b,
                        slot => slot.insert(src.iter().map(|&b| b as f32).collect()),
                    };
                    let inv = 1.0 - ema_alpha;
                    for (i, &b) in src.iter().enumerate() {
                        buf[i] = buf[i] * inv + b as f32 * ema_alpha;
                    }
                    out_buf.clear();
                    out_buf.reserve(buf.len());
                    out_buf.extend(buf.iter().map(|&v| v.round().clamp(0.0, 255.0) as u8));
                    Bytes::copy_from_slice(&out_buf)
                } else {
                    frame
                };

                let meta = FrameMeta {
                    sequence: seq,
                    delay_ms: tick.as_secs_f32() * 1000.0,
                    width: fields.width,
                    height: fields.height,
                    format: fields.fmt,
                    is_still: false,
                    is_last_frame: false,
                };
                if let Err(e) = sink.send_frame(Frame { data, meta }).await {
                    warn!(?e, "sink send failed in paced mode");
                }
                seq = seq.wrapping_add(1);
            }

            next += tick;
            let now = Instant::now();
            if next > now {
                time::sleep(next - now).await;
            } else {
                // Missed tick — reset.
                next = now;
            }
        }
    };

    tokio::select! {
        _ = producer => {},
        _ = sampler => {},
    }
    Ok(())
}
