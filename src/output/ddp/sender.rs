//! DDP UDP sender. Owns the socket; the collision registry gates whether a
//! sender ever comes into existence for a given key.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::BytesMut;
use parking_lot::Mutex;
use tokio::net::UdpSocket;
use tokio::time;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::error::OutputError;
use crate::output::ddp::packet::{DDP_HEADER_LEN, DDP_MAX_DATA, PacketEncoder};
use crate::output::ddp::pixel;
use crate::output::ddp::spreading::{self, SpreadConfig};
use crate::output::metrics::RateMeter;
use crate::output::sink::{Frame, OutputSink, PixelFormat};

/// Per-stream DDP sender. One socket, bound to an ephemeral port, shared
/// across the stream's lifetime.
pub struct DdpSender {
    socket: Arc<UdpSocket>,
    dest: SocketAddr,
    output_id: u8,
    pixel_format: PixelFormat,
    still_redundancy: u32,
    spread: SpreadCfg,
    pace_hz: u32,
    metrics: Option<Mutex<Metrics>>,
    seq: AtomicU8,
    /// Reusable per-packet buffer: header + max chunk. Held under a mutex
    /// because `OutputSink::send_frame` takes `&self`, but in practice only
    /// one task touches it.
    pkt_buf: Mutex<BytesMut>,
}

#[derive(Debug, Clone, Copy)]
struct SpreadCfg {
    enabled: bool,
    max_fps: u32,
    min_spacing: Duration,
    max_sleeps: u32,
}

struct Metrics {
    frames: RateMeter,
    /// Unique packets — one tick per distinct packet (excludes still-frame
    /// redundancy duplicates). `pkt_jit` is read from this so still-frame
    /// bursts don't show up as crushed-to-zero jitter.
    unique_packets: RateMeter,
    /// Physical UDP sends — one tick per `send_to`. Used for `phy` rate when
    /// redundancy multiplies the on-wire packet count.
    physical_packets: RateMeter,
    log_interval: Duration,
    last_log: Instant,
    /// Lifetime physical sends since stream start.
    tx_total: u64,
    /// Last frame's `delay_ms` — for native-mode target FPS.
    last_delay_ms: f32,
    /// Did the most recent frame actually engage spreading?
    spread_active: bool,
}

impl Metrics {
    fn new(log_interval: Duration) -> Self {
        let window = log_interval.max(Duration::from_secs(1));
        Self {
            frames: RateMeter::new(window),
            unique_packets: RateMeter::new(window),
            physical_packets: RateMeter::new(window),
            log_interval,
            last_log: Instant::now(),
            tx_total: 0,
            last_delay_ms: 0.0,
            spread_active: false,
        }
    }
}

impl DdpSender {
    pub async fn bind(
        dest_ip: IpAddr,
        dest_port: u16,
        output_id: u8,
        pixel_format: PixelFormat,
        pace_hz: u32,
        config: &Config,
    ) -> Result<Self, OutputError> {
        let bind_addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let socket = UdpSocket::bind(bind_addr).await?;
        debug!(
            local = ?socket.local_addr().ok(),
            dest = %dest_ip,
            out = output_id,
            "DDP sender bound"
        );

        let spread = SpreadCfg {
            enabled: config.net.spread_packets,
            max_fps: config.net.spread_max_fps,
            min_spacing: Duration::from_micros((config.net.spread_min_ms * 1000.0).max(0.0) as u64),
            max_sleeps: config.net.spread_max_sleeps,
        };

        let metrics = config
            .log
            .metrics
            .then(|| Mutex::new(Metrics::new(Duration::from_millis(config.log.rate_ms.max(1)))));

        Ok(Self {
            socket: Arc::new(socket),
            dest: SocketAddr::new(dest_ip, dest_port),
            output_id,
            pixel_format,
            still_redundancy: config.playback_still.redundancy,
            spread,
            pace_hz,
            metrics,
            seq: AtomicU8::new(1),
            pkt_buf: Mutex::new(BytesMut::with_capacity(DDP_HEADER_LEN + DDP_MAX_DATA)),
        })
    }

    async fn send_packet(&self, pkt: &[u8]) -> Result<(), OutputError> {
        self.socket.send_to(pkt, self.dest).await?;
        Ok(())
    }

    /// Take ownership of the reusable per-packet buffer for the duration of
    /// one frame emit. The caller hands it back via `return_pkt_buf` so the
    /// allocation is reused on the next frame.
    fn take_pkt_buf(&self) -> BytesMut {
        std::mem::replace(
            &mut *self.pkt_buf.lock(),
            BytesMut::with_capacity(DDP_HEADER_LEN + DDP_MAX_DATA),
        )
    }

    fn return_pkt_buf(&self, buf: BytesMut) {
        *self.pkt_buf.lock() = buf;
    }

    /// Compute the spreading plan for a frame whose pacing wants
    /// `delay_ms` between emits. Returns `None` when spreading is
    /// disabled or the frame rate exceeds `spread_max_fps`.
    fn plan_spread(&self, delay_ms: f32, payload_bytes: usize) -> Option<spreading::Plan> {
        if !self.spread.enabled {
            return None;
        }
        let frame_rate = 1000.0 / delay_ms.max(1.0);
        if frame_rate > self.spread.max_fps as f32 {
            return None;
        }
        let cfg = SpreadConfig {
            min_spacing: self.spread.min_spacing,
            max_sleeps: self.spread.max_sleeps,
        };
        let pkt_count = payload_bytes.div_ceil(DDP_MAX_DATA) as u32;
        let plan =
            spreading::compute_spacing_and_group(pkt_count, Duration::from_secs_f32(delay_ms / 1000.0), &cfg);
        plan.spacing.is_some().then_some(plan)
    }

    fn record_frame(
        &self,
        now: Instant,
        unique_packets: u32,
        physical_packets: u32,
        delay_ms: f32,
        spread_active: bool,
    ) {
        let Some(m) = &self.metrics else { return };
        let mut m = m.lock();
        m.frames.tick(now);
        m.unique_packets.tick_n(now, unique_packets);
        m.physical_packets.tick_n(now, physical_packets);
        m.tx_total = m.tx_total.saturating_add(physical_packets as u64);
        m.last_delay_ms = delay_ms;
        m.spread_active = spread_active;

        if now.duration_since(m.last_log) >= m.log_interval {
            let fps = m.frames.rate_hz();
            let unique_pps = m.unique_packets.rate_hz();
            let physical_pps = m.physical_packets.rate_hz();
            let frm_jit = m.frames.jitter_ms();
            let pkt_jit = m.unique_packets.jitter_ms();
            let tx = m.tx_total;
            let spread = m.spread_active;
            let last_delay_ms = m.last_delay_ms;

            // pps formatting: surface redundancy multiplier when active.
            let pps_str = if (physical_pps - unique_pps).abs() > 0.5 {
                let factor = if unique_pps > 0.0 {
                    physical_pps / unique_pps
                } else {
                    1.0
                };
                format!("{unique_pps:.0} ({physical_pps:.0}phy, {factor:.1}x)")
            } else {
                format!("{unique_pps:.0}")
            };

            // Mode tag: pace=NHz vs native (~tgt fps from delay_ms).
            let mode_str = if self.pace_hz > 0 {
                format!("pace={}Hz", self.pace_hz)
            } else {
                let tgt = 1000.0 / last_delay_ms.max(1.0);
                format!("native (~{tgt:.1} tgt)")
            };

            let spread_tag = if spread { " (spread)" } else { "" };

            info!(
                "out={} {} fps={fps:.2} pps={pps_str} pkt_jit={pkt_jit:.1}ms frm_jit={frm_jit:.1}ms tx={tx}{spread_tag}",
                self.output_id, mode_str
            );
            m.last_log = now;
        }
    }
}

#[async_trait]
impl OutputSink for DdpSender {
    async fn send_frame(&self, frame: Frame) -> Result<(), OutputError> {
        let payload = pixel::encode_frame(&frame.data, self.pixel_format);

        let redundancy = if frame.meta.is_still && frame.meta.is_last_frame {
            self.still_redundancy.max(1)
        } else {
            1
        };

        let spread_plan = self.plan_spread(frame.meta.delay_ms, payload.len());

        let start_seq = self.seq.load(Ordering::Relaxed);
        let mut unique_packets: u32 = 0;
        let mut physical_packets: u32 = 0;
        let start = Instant::now();
        let mut slot = 0u32;
        let mut group_left = spread_plan.as_ref().map(|p| p.group_n).unwrap_or(1);

        let mut pkt_buf = self.take_pkt_buf();
        let mut encoder = PacketEncoder::new(&payload, self.output_id, start_seq, self.pixel_format);
        while let Some(len) = encoder.encode_next(&mut pkt_buf) {
            for _ in 0..redundancy {
                if let Err(e) = self.send_packet(&pkt_buf[..len]).await {
                    warn!(?e, "DDP send failed");
                    self.return_pkt_buf(pkt_buf);
                    return Err(e);
                }
                physical_packets = physical_packets.saturating_add(1);
            }
            unique_packets = unique_packets.saturating_add(1);

            // Apply spreading between unique-packet groups, not between
            // redundant copies — matches the Python spec for still-frame
            // redundancy that must be transmitted back-to-back.
            if let Some(plan) = &spread_plan
                && let Some(spacing) = plan.spacing
            {
                group_left = group_left.saturating_sub(1);
                if group_left == 0 {
                    slot += 1;
                    let target = start + spacing * slot;
                    let now = Instant::now();
                    if target > now {
                        time::sleep(target - now).await;
                    }
                    group_left = plan.group_n;
                }
            }
        }
        self.seq.store(encoder.current_seq(), Ordering::Relaxed);
        self.return_pkt_buf(pkt_buf);
        self.record_frame(
            Instant::now(),
            unique_packets,
            physical_packets,
            frame.meta.delay_ms,
            spread_plan.is_some(),
        );

        Ok(())
    }

    async fn close(&self) -> Result<(), OutputError> {
        // UdpSocket drops when the Arc drops.
        Ok(())
    }
}

/// Number of DDP packets a payload of `bytes` will split into.
#[inline]
pub fn packets_for(bytes: usize) -> usize {
    bytes.div_ceil(DDP_MAX_DATA)
}
