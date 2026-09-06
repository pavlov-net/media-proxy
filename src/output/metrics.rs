//! Rolling event rates and inter-batch timing jitter.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Stores one timestamp per event batch. Rates count events; jitter measures
/// intervals between batches, avoiding an allocation for every event.
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

    /// Records a batch at `t` and evicts expired entries. A zero count is ignored.
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
        // Welford's online variance; single pass, no Vec alloc.
        let mut n: u64 = 0;
        let mut mean = 0.0f64;
        let mut m2 = 0.0f64;
        let mut iter = self.ts.iter();
        let Some(&(mut prev_t, _)) = iter.next() else {
            return 0.0;
        };
        for &(t, _) in iter {
            let d = t.duration_since(prev_t).as_secs_f64();
            n += 1;
            let delta = d - mean;
            mean += delta / n as f64;
            m2 += delta * (d - mean);
            prev_t = t;
        }
        if n < 2 {
            return 0.0;
        }
        let var = m2 / (n - 1) as f64;
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
        // 10 ticks over 900ms -> ~10Hz
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
        // Both meters see 1000 events across 900ms; rates must match.
        assert!((batched.rate_hz() - unitary.rate_hz()).abs() < 1e-6);
    }
}
