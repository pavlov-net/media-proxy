//! Rolling-window metrics: frame/packet rate, jitter, queue occupancy.
//!
//! One `RateMeter` per metric, logged every `log_interval`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Each entry is `(timestamp, count)` so callers can record a batch of
/// events that share an `Instant` (e.g. all DDP packets from one frame)
/// without allocating an entry per event. Rate sums counts; jitter still
/// reads inter-batch timestamps only.
pub struct RateMeter {
    window: Duration,
    ts: VecDeque<(Instant, u32)>,
}

impl RateMeter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            ts: VecDeque::new(),
        }
    }

    pub fn tick(&mut self, t: Instant) {
        self.tick_n(t, 1);
    }

    /// Record `count` events that all happened at `t`. Stored as a single
    /// entry — keeps the deque bounded by emit cadence, not by event count.
    pub fn tick_n(&mut self, t: Instant, count: u32) {
        if count == 0 {
            return;
        }
        self.ts.push_back((t, count));
        let cutoff = t - self.window;
        while let Some(&(front, _)) = self.ts.front() {
            if front < cutoff {
                self.ts.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn rate_hz(&self) -> f64 {
        let (Some(&(first, _)), Some(&(last, _))) = (self.ts.front(), self.ts.back()) else {
            return 0.0;
        };
        if self.ts.len() < 2 {
            return 0.0;
        }
        let dur = last.duration_since(first).as_secs_f64();
        if dur <= 0.0 {
            return 0.0;
        }
        // (n - 1) / dur: subtract one event for the window's open edge,
        // matching the unit-tick formula. Counted across batches.
        let total: u64 = self.ts.iter().map(|(_, c)| *c as u64).sum();
        total.saturating_sub(1) as f64 / dur
    }

    pub fn jitter_ms(&self) -> f64 {
        if self.ts.len() < 3 {
            return 0.0;
        }
        let diffs: Vec<f64> = self
            .ts
            .iter()
            .zip(self.ts.iter().skip(1))
            .map(|((a, _), (b, _))| b.duration_since(*a).as_secs_f64())
            .collect();
        let mean = diffs.iter().sum::<f64>() / diffs.len() as f64;
        let var = diffs.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / (diffs.len() - 1) as f64;
        var.sqrt() * 1000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rate_is_zero() {
        let m = RateMeter::new(Duration::from_secs(1));
        assert_eq!(m.rate_hz(), 0.0);
    }

    #[test]
    fn rate_approximates_input() {
        let mut m = RateMeter::new(Duration::from_secs(10));
        let t0 = Instant::now();
        for i in 0..10 {
            m.tick(t0 + Duration::from_millis(i * 100));
        }
        // 10 ticks over 900ms → ~10Hz
        let r = m.rate_hz();
        assert!(r > 9.0 && r < 11.0, "rate was {r}");
    }

    #[test]
    fn tick_n_matches_repeated_tick_for_rate() {
        let mut batched = RateMeter::new(Duration::from_secs(10));
        let mut unitary = RateMeter::new(Duration::from_secs(10));
        let t0 = Instant::now();
        // 10 frames, 100 packets each, 100ms apart.
        for i in 0..10 {
            let t = t0 + Duration::from_millis(i * 100);
            batched.tick_n(t, 100);
            for _ in 0..100 {
                unitary.tick(t);
            }
        }
        // Both meters see 1000 events across 900ms — rates must match.
        assert!((batched.rate_hz() - unitary.rate_hz()).abs() < 1e-6);
    }
}
