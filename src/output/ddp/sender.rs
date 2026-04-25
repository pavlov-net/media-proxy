//! DDP UDP sender. Owns the socket; the collision registry gates whether a
//! sender ever comes into existence for a given key.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::net::UdpSocket;
use tokio::time;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::error::OutputError;
use crate::output::ddp::packet::{DDP_MAX_DATA, iter_packets};
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
    metrics: Option<Mutex<Metrics>>,
    seq: Mutex<u8>,
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
    packets: RateMeter,
    log_interval: Duration,
    last_log: Instant,
}

impl Metrics {
    fn new(log_interval: Duration) -> Self {
        let window = log_interval.max(Duration::from_secs(1));
        Self {
            frames: RateMeter::new(window),
            packets: RateMeter::new(window),
            log_interval,
            last_log: Instant::now(),
        }
    }
}

impl DdpSender {
    pub async fn bind(
        dest_ip: IpAddr,
        dest_port: u16,
        output_id: u8,
        pixel_format: PixelFormat,
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
            metrics,
            seq: Mutex::new(1),
        })
    }

    async fn send_packet(&self, pkt: &[u8]) -> Result<(), OutputError> {
        self.socket.send_to(pkt, self.dest).await?;
        Ok(())
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

    fn record_frame(&self, now: Instant, packets: u32) {
        let Some(m) = &self.metrics else { return };
        let mut m = m.lock();
        m.frames.tick(now);
        for _ in 0..packets {
            m.packets.tick(now);
        }
        if now.duration_since(m.last_log) >= m.log_interval {
            let fps = m.frames.rate_hz();
            let pps = m.packets.rate_hz();
            let frm_jit = m.frames.jitter_ms();
            let pkt_jit = m.packets.jitter_ms();
            info!(
                out = self.output_id,
                fps = format!("{fps:.2}"),
                pps = format!("{pps:.0}"),
                frm_jit_ms = format!("{frm_jit:.1}"),
                pkt_jit_ms = format!("{pkt_jit:.1}"),
                "ddp metrics"
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

        let start_seq = *self.seq.lock();
        let mut last_next = start_seq;
        let mut total_packets: u32 = 0;
        let start = Instant::now();
        let mut slot = 0u32;
        let mut group_left = spread_plan.as_ref().map(|p| p.group_n).unwrap_or(1);

        for (pkt, next) in iter_packets(&payload, self.output_id, start_seq, self.pixel_format) {
            for _ in 0..redundancy {
                if let Err(e) = self.send_packet(&pkt).await {
                    warn!(?e, "DDP send failed");
                    return Err(e);
                }
                total_packets = total_packets.saturating_add(1);
            }
            last_next = next;

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
        *self.seq.lock() = last_next;
        self.record_frame(Instant::now(), total_packets);

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
