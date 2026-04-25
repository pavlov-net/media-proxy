//! Rolling-window metrics: frame/packet rate, jitter, queue occupancy.
//!
//! One `RateMeter` per metric, logged every `log_interval`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub struct RateMeter {
    window: Duration,
    ts: VecDeque<Instant>,
}

impl RateMeter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            ts: VecDeque::new(),
        }
    }

    pub fn tick(&mut self, t: Instant) {
        self.ts.push_back(t);
        let cutoff = t - self.window;
        while let Some(&front) = self.ts.front() {
            if front < cutoff {
                self.ts.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn rate_hz(&self) -> f64 {
        let (Some(&first), Some(&last)) = (self.ts.front(), self.ts.back()) else {
            return 0.0;
        };
        if self.ts.len() < 2 {
            return 0.0;
        }
        let dur = last.duration_since(first).as_secs_f64();
        if dur <= 0.0 {
            return 0.0;
        }
        (self.ts.len() - 1) as f64 / dur
    }

    pub fn jitter_ms(&self) -> f64 {
        if self.ts.len() < 3 {
            return 0.0;
        }
        let diffs: Vec<f64> = self
            .ts
            .iter()
            .zip(self.ts.iter().skip(1))
            .map(|(a, b)| b.duration_since(*a).as_secs_f64())
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
}
