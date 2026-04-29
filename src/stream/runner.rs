//! Per-stream runners: native cadence and paced mode.
//!
//! Native runs at the source's frame timing; paced samples the latest frame
//! at a fixed `pace_hz` with optional EMA blending. Both reset their
//! deadline when more than 100ms behind to prevent runaway catch-up bursts.

use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use parking_lot::Mutex;
use tokio::time;
use tracing::{debug, warn};
use wide::f32x8;

use crate::control::fields::StreamFields;
use crate::error::{MediaError, StreamError};
use crate::output::sink::{Frame, FrameMeta, OutputSink};
use crate::stream::frame_source::{FrameSource, RgbFrame};

/// Upper bound on time between frames — if the source stalls this long we
/// cut the stream and let the orchestrator decide whether to restart.
const FRAME_WATCHDOG: Duration = Duration::from_secs(30);

/// Watchdog trips surface as a retryable network condition rather than
/// `Cancelled`, so the orchestrator's per-stream retry budget can rebuild
/// the source instead of treating a stall as a permanent stop.
fn watchdog_error(source_url: &str) -> StreamError {
    StreamError::Media(MediaError::Network {
        source_url: source_url.to_string(),
        message: "frame watchdog tripped (>30s idle)".into(),
        error_code: None,
        retryable: true,
    })
}

/// Run at the source's native cadence. One frame pulled → one frame emitted,
/// with timing based on the frame's `delay_ms` hint. Returns the number of
/// frames emitted so the orchestrator can detect a source that died
/// immediately (e.g. ffmpeg failing on a 403) versus one that ran normally.
pub async fn run_native(
    mut source: FrameSource,
    sink: &dyn OutputSink,
    fields: &StreamFields,
) -> Result<u64, StreamError> {
    let mut seq: u32 = 0;
    let mut next_frame_time = Instant::now();
    let mut emitted = 0u64;

    loop {
        let pulled = time::timeout(FRAME_WATCHDOG, source.next()).await;
        let frame = match pulled {
            Ok(Some(f)) => f,
            Ok(None) => {
                if let Some(err) = source.take_error().await {
                    debug!(emitted, %err, "native source ended with error");
                    return Err(err);
                }
                if fields.r#loop && source.try_rewind() {
                    debug!(emitted, "native source rewound for loop");
                    continue;
                }
                debug!(emitted, "native source exhausted");
                return Ok(emitted);
            }
            Err(_) => {
                warn!("frame watchdog tripped (>30s idle)");
                return Err(watchdog_error(&fields.source));
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
///
/// The shared slot is a `parking_lot::Mutex<Option<Bytes>>` — the lock is
/// only held for the duration of a clone or a swap, never across `.await`,
/// so we don't need the heavier `tokio::Mutex`. The EMA path reuses one
/// `BytesMut` across ticks instead of allocating a fresh out_buf + Bytes
/// every emit.
///
/// Both sub-tasks return `Result<u64, StreamError>`: producer yields the
/// frame count or the source-side error; sampler resolves only on a sink
/// failure (otherwise loops forever, dropped when producer returns first).
pub async fn run_paced(
    source: FrameSource,
    sink: &dyn OutputSink,
    fields: &StreamFields,
) -> Result<u64, StreamError> {
    let pace_hz = fields.pace.max(1) as f32;
    let ema_alpha = fields.ema.clamp(0.0, 1.0);
    let tick = Duration::from_secs_f32(1.0 / pace_hz);

    let latest: Mutex<Option<Bytes>> = Mutex::new(None);

    let producer = async {
        let mut source = source;
        let mut count: u64 = 0;
        loop {
            match time::timeout(FRAME_WATCHDOG, source.next()).await {
                Ok(Some(f)) => {
                    count += 1;
                    *latest.lock() = Some(f.rgb888);
                }
                Ok(None) => {
                    if let Some(err) = source.take_error().await {
                        return Err(err);
                    }
                    return Ok(count);
                }
                Err(_) => {
                    warn!("paced frame watchdog tripped (>30s idle)");
                    return Err(watchdog_error(&fields.source));
                }
            }
        }
    };

    let sampler = async {
        let mut next = Instant::now();
        let mut seq: u32 = 0;
        let mut ema_buf: Option<Vec<f32>> = None;
        let mut out_buf: BytesMut = BytesMut::new();

        loop {
            let snap = latest.lock().clone();
            if let Some(frame) = snap {
                let data = if ema_alpha > 0.0 {
                    let src = &frame;
                    let buf = match &mut ema_buf {
                        Some(b) if b.len() == src.len() => b,
                        slot => slot.insert(src.iter().map(|&b| b as f32).collect()),
                    };
                    if out_buf.capacity() < buf.len() {
                        out_buf = BytesMut::with_capacity(buf.len());
                    }
                    out_buf.clear();
                    out_buf.resize(buf.len(), 0);
                    ema_blend_into(buf, src, ema_alpha, &mut out_buf[..]);
                    let frozen = out_buf.split().freeze();
                    out_buf.reserve(buf.len());
                    frozen
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
                sink.send_frame(Frame { data, meta })
                    .await
                    .map_err(StreamError::Output)?;
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
        // Unreachable: the loop above only exits via `?` propagating a sink error.
        // The trailing expression gives the async block a concrete `Ok` arm so
        // both `select!` branches share `Result<u64, StreamError>`.
        #[allow(unreachable_code)]
        Ok::<u64, StreamError>(0)
    };

    tokio::select! {
        r = producer => r,
        r = sampler => r,
    }
}

/// Update an EMA accumulator from a u8 frame and write the rounded/clamped
/// u8 result into `out`. SIMD path processes 8 lanes at a time via
/// `wide::f32x8`; tail handled scalar. With `target-cpu=x86-64-v3` this maps
/// to AVX2 + FMA.
///
/// Math: `buf[i] = buf[i] * (1-α) + src[i] * α`, then
/// `out[i] = clamp(round(buf[i]), 0, 255) as u8`.
///
/// Both paths use `mul_add` to fuse the multiply-add into a single rounded
/// op, so the SIMD and scalar lanes produce bit-identical buffers.
fn ema_blend_into(buf: &mut [f32], src: &[u8], alpha: f32, out: &mut [u8]) {
    debug_assert_eq!(buf.len(), src.len());
    debug_assert_eq!(buf.len(), out.len());
    let n = buf.len();
    let inv = 1.0 - alpha;

    let alpha_v = f32x8::splat(alpha);
    let inv_v = f32x8::splat(inv);
    let half_v = f32x8::splat(0.5);
    let lo_v = f32x8::splat(0.0);
    let hi_v = f32x8::splat(255.0);

    let main_n = n - (n % 8);
    let mut i = 0;
    while i < main_n {
        let bv = f32x8::new([
            buf[i],
            buf[i + 1],
            buf[i + 2],
            buf[i + 3],
            buf[i + 4],
            buf[i + 5],
            buf[i + 6],
            buf[i + 7],
        ]);
        let sv = f32x8::new([
            f32::from(src[i]),
            f32::from(src[i + 1]),
            f32::from(src[i + 2]),
            f32::from(src[i + 3]),
            f32::from(src[i + 4]),
            f32::from(src[i + 5]),
            f32::from(src[i + 6]),
            f32::from(src[i + 7]),
        ]);
        // FMA: sv * alpha_v + bv * inv_v, computed as one fused op.
        let new_b = sv.mul_add(alpha_v, bv * inv_v);
        let new_b_arr = new_b.to_array();
        buf[i..i + 8].copy_from_slice(&new_b_arr);

        let rounded = (new_b + half_v).max(lo_v).min(hi_v);
        let r_arr = rounded.to_array();
        for k in 0..8 {
            out[i + k] = r_arr[k] as u8;
        }
        i += 8;
    }

    while i < n {
        buf[i] = f32::from(src[i]).mul_add(alpha, buf[i] * inv);
        out[i] = (buf[i] + 0.5).clamp(0.0, 255.0) as u8;
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ema_blend_naive(buf: &mut [f32], src: &[u8], alpha: f32, out: &mut [u8]) {
        let inv = 1.0 - alpha;
        for i in 0..buf.len() {
            buf[i] = f32::from(src[i]).mul_add(alpha, buf[i] * inv);
            out[i] = (buf[i] + 0.5).clamp(0.0, 255.0) as u8;
        }
    }

    #[test]
    fn ema_simd_matches_scalar() {
        for &n in &[0usize, 1, 7, 8, 9, 23, 64, 191, 1024] {
            for &alpha in &[0.05_f32, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
                let mut buf_simd: Vec<f32> = (0..n).map(|i| (i as f32) * 0.7).collect();
                let mut buf_naive = buf_simd.clone();
                let src: Vec<u8> = (0..n).map(|i| i.wrapping_mul(73) as u8).collect();
                let mut out_simd = vec![0u8; n];
                let mut out_naive = vec![0u8; n];
                ema_blend_into(&mut buf_simd, &src, alpha, &mut out_simd);
                ema_blend_naive(&mut buf_naive, &src, alpha, &mut out_naive);
                assert_eq!(out_simd, out_naive, "out mismatch n={n} alpha={alpha}");
                for i in 0..n {
                    assert!(
                        (buf_simd[i] - buf_naive[i]).abs() < 1e-3,
                        "buf mismatch i={i} n={n} alpha={alpha} simd={} naive={}",
                        buf_simd[i],
                        buf_naive[i],
                    );
                }
            }
        }
    }

    #[test]
    fn ema_blend_full_alpha_replaces_buffer() {
        let mut buf = vec![100.0_f32; 16];
        let src = vec![200u8; 16];
        let mut out = vec![0u8; 16];
        ema_blend_into(&mut buf, &src, 1.0, &mut out);
        for &b in &buf {
            assert!((b - 200.0).abs() < 1e-4);
        }
        assert!(out.iter().all(|&v| v == 200));
    }

    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::atomic::{AtomicU32, Ordering};

    use async_trait::async_trait;
    use tokio::sync::{mpsc, oneshot};

    use crate::control::fields::{Fit, HwPref};
    use crate::error::OutputError;
    use crate::output::sink::PixelFormat;
    use crate::stream::frame_source::{FrameSource, VideoSource};

    fn test_fields() -> StreamFields {
        StreamFields {
            output_id: 0,
            width: 4,
            height: 4,
            source: "test://input".into(),
            ddp_port: 4048,
            ddp_host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            r#loop: false,
            expand: 0,
            hw: HwPref::None,
            fit: Fit::Pad,
            fmt: PixelFormat::Rgb888,
            pace: 30,
            ema: 0.0,
        }
    }

    fn test_video_source() -> (
        FrameSource,
        mpsc::Sender<RgbFrame>,
        oneshot::Sender<Result<(), MediaError>>,
    ) {
        use crate::video::subprocess::KillGuard;
        let (tx, rx) = mpsc::channel::<RgbFrame>(8);
        let (comp_tx, comp_rx) = oneshot::channel::<Result<(), MediaError>>();
        let src = FrameSource::Video(VideoSource::new(rx, comp_rx, KillGuard::detached()));
        (src, tx, comp_tx)
    }

    struct CountingSink {
        sent: AtomicU32,
        fail_after: u32,
    }

    impl CountingSink {
        fn new(fail_after: u32) -> Self {
            Self {
                sent: AtomicU32::new(0),
                fail_after,
            }
        }
    }

    #[async_trait]
    impl OutputSink for CountingSink {
        async fn send_frame(&self, _frame: Frame) -> Result<(), OutputError> {
            let n = self.sent.fetch_add(1, Ordering::Relaxed) + 1;
            if n >= self.fail_after {
                return Err(OutputError::Sink("test forced send failure".into()));
            }
            Ok(())
        }
        async fn close(&self) -> Result<(), OutputError> {
            Ok(())
        }
    }

    fn rgb_frame(n: usize) -> RgbFrame {
        RgbFrame {
            rgb888: Bytes::from(vec![0u8; n]),
            delay_ms: 33.0,
        }
    }

    /// Paced watchdog: source never produces a frame, watchdog must trip
    /// with a retryable error (so the orchestrator's retry budget applies).
    #[tokio::test(start_paused = true)]
    async fn paced_watchdog_returns_retryable() {
        let (source, _tx, _comp) = test_video_source();
        let sink = CountingSink::new(u32::MAX);
        let fields = test_fields();

        let result = tokio::select! {
            r = run_paced(source, &sink, &fields) => r,
            () = async {
                // Advance past the watchdog window.
                tokio::time::advance(FRAME_WATCHDOG + Duration::from_secs(1)).await;
                std::future::pending::<()>().await;
            } => unreachable!(),
        };

        let err = result.expect_err("watchdog must trip");
        match err {
            StreamError::Media(MediaError::Network { retryable: true, .. }) => {}
            other => panic!("expected retryable Network error, got {other:?}"),
        }
    }

    /// Native watchdog: same shape — must surface a retryable error so the
    /// orchestrator can rebuild the source instead of giving up.
    #[tokio::test(start_paused = true)]
    async fn native_watchdog_returns_retryable() {
        let (source, _tx, _comp) = test_video_source();
        let sink = CountingSink::new(u32::MAX);
        let mut fields = test_fields();
        fields.pace = 0;

        let result = tokio::select! {
            r = run_native(source, &sink, &fields) => r,
            () = async {
                tokio::time::advance(FRAME_WATCHDOG + Duration::from_secs(1)).await;
                std::future::pending::<()>().await;
            } => unreachable!(),
        };

        let err = result.expect_err("watchdog must trip");
        match err {
            StreamError::Media(MediaError::Network { retryable: true, .. }) => {}
            other => panic!("expected retryable Network error, got {other:?}"),
        }
    }

    /// Paced sink failure must propagate, not be swallowed: the runner
    /// returns the Output error so the orchestrator sees the broken sink.
    #[tokio::test(start_paused = true)]
    async fn paced_sink_error_propagates() {
        let (source, tx, _comp) = test_video_source();
        let sink = CountingSink::new(1); // fail on first send
        let fields = test_fields();

        // Push one frame so the sampler has something to send.
        tx.send(rgb_frame(fields.width as usize * fields.height as usize * 3))
            .await
            .unwrap();

        let result = tokio::select! {
            r = run_paced(source, &sink, &fields) => r,
            () = async {
                // Let the sampler tick at least once; pace=30 → 33ms tick.
                tokio::time::advance(Duration::from_millis(100)).await;
                std::future::pending::<()>().await;
            } => unreachable!(),
        };

        let err = result.expect_err("sink failure must surface");
        assert!(matches!(err, StreamError::Output(_)), "got {err:?}");
    }
}
