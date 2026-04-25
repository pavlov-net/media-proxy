//! Per-frame display-delay calculator from consecutive PTS values.
//!
//! - First frame: if `avg_ms` is known (from declared fps) use it; else use
//!   the caller's default.
//! - Subsequent frames: `delta = (pts - prev_pts) * 1000`. With `avg_ms`
//!   known, clamp `delta` to `[0.75 * avg, 1.25 * avg]` to absorb decoder
//!   jitter. Without it, a non-positive delta gets the default.
//! - Floor at `MIN_DELAY_MS` so downstream pacing never divides by 0.

pub const MIN_DELAY_MS: f32 = 10.0;

pub struct DelayClock {
    avg_ms: Option<f32>,
    prev_pts_s: Option<f64>,
    default_ms: f32,
    frames: u64,
}

impl DelayClock {
    pub fn new(avg_ms: Option<f32>, default_ms: f32) -> Self {
        Self {
            avg_ms,
            prev_pts_s: None,
            default_ms,
            frames: 0,
        }
    }

    pub fn frames_seen(&self) -> u64 {
        self.frames
    }

    /// Compute the delay to hold the current frame before the next, never
    /// below [`MIN_DELAY_MS`]. Always returns a valid delay — falls back
    /// through `avg_ms` and then the `default_ms` the clock was built with.
    pub fn next_delay(&mut self, pts_s: Option<f64>) -> f32 {
        self.frames += 1;

        let delta_ms = pts_s.zip(self.prev_pts_s).map(|(p, q)| ((p - q) * 1000.0) as f32);

        // Always advance `prev_pts_s` when we have one, even when the delta
        // is unusable (backwards PTS, reorders). Otherwise we'd compute the
        // next delta against a stale anchor and drift indefinitely.
        if let Some(p) = pts_s {
            self.prev_pts_s = Some(p);
        }

        let delay = match (delta_ms, self.avg_ms) {
            (Some(d), Some(avg)) => d.clamp(0.75 * avg, 1.25 * avg),
            (Some(d), None) if d > 0.0 => d,
            (_, Some(avg)) => avg,
            (None, None) => self.default_ms,
            (Some(_), None) => self.default_ms,
        };
        delay.max(MIN_DELAY_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_with_avg_returns_avg() {
        let mut c = DelayClock::new(Some(33.33), 50.0);
        let d = c.next_delay(Some(0.0));
        assert!((d - 33.33).abs() < 0.01);
    }

    #[test]
    fn steady_pts_matches_delta() {
        let mut c = DelayClock::new(Some(33.33), 50.0);
        c.next_delay(Some(0.0));
        let d = c.next_delay(Some(0.0333));
        assert!((d - 33.3).abs() < 1.0);
    }

    #[test]
    fn pts_jitter_clamped_to_avg_window() {
        let mut c = DelayClock::new(Some(33.33), 50.0);
        c.next_delay(Some(0.0));
        // Burst: PTS moved 500 ms — must clamp to 1.25 × avg.
        let d = c.next_delay(Some(0.5));
        assert!(d <= 1.25 * 33.33);
    }

    #[test]
    fn no_pts_no_avg_returns_default() {
        let mut c = DelayClock::new(None, 42.0);
        assert!((c.next_delay(None) - 42.0).abs() < 0.01);
    }

    #[test]
    fn floor_at_min_delay() {
        let mut c = DelayClock::new(Some(5.0), 50.0);
        let d = c.next_delay(Some(0.0));
        assert!(d >= MIN_DELAY_MS);
    }

    #[test]
    fn backwards_pts_advances_anchor() {
        // Consecutive PTS: 0.0, 0.0 (stall), 0.1. After the stall, the
        // anchor must be the stalled value — not the pre-stall — so the
        // 0.1 frame's delta is 100ms, not 200ms. This is the regression
        // for the pre-fix bug where the early-return skipped the anchor update.
        let mut c = DelayClock::new(None, 33.0);
        c.next_delay(Some(0.0));
        c.next_delay(Some(0.0));
        let d = c.next_delay(Some(0.1));
        assert!((d - 100.0).abs() < 1.0, "expected 100ms, got {d}");
    }
}
