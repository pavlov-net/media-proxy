//! Packet spreading distributes DDP packets across one frame
//! interval so the receiver's input buffer doesn't take the whole frame in
//! one burst.

use std::time::Duration;

pub struct SpreadConfig {
    pub min_spacing: Duration,
    pub max_sleeps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    pub spacing: Option<Duration>,
    pub group_n: u32,
}

impl Plan {
    pub const NONE: Self = Self {
        spacing: None,
        group_n: 1,
    };
}

/// Computes `(spacing, group_n)` for spreading `pkt_count` packets across
/// `frame_interval`. If neither is useful (zero packets / zero interval),
/// returns `Plan::NONE`.
pub fn compute_spacing_and_group(pkt_count: u32, frame_interval: Duration, cfg: &SpreadConfig) -> Plan {
    if pkt_count == 0 || frame_interval.is_zero() {
        return Plan::NONE;
    }

    let ideal = frame_interval / pkt_count;
    let mut group_n: u32 = 1;

    if ideal > Duration::ZERO && ideal < cfg.min_spacing {
        let min_ns = cfg.min_spacing.as_nanos();
        let ideal_ns = ideal.as_nanos();
        if ideal_ns > 0 {
            group_n = min_ns.div_ceil(ideal_ns).max(1) as u32;
        }
    }

    if cfg.max_sleeps > 0 {
        let per_sleep = pkt_count.div_ceil(cfg.max_sleeps).max(1);
        group_n = group_n.max(per_sleep);
    }

    let mut spacing = ideal * group_n;
    if spacing > frame_interval {
        spacing = frame_interval;
    }

    Plan {
        spacing: Some(spacing),
        group_n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_packets_yields_no_spreading() {
        let cfg = SpreadConfig {
            min_spacing: Duration::from_millis(3),
            max_sleeps: 0,
        };
        assert_eq!(
            compute_spacing_and_group(0, Duration::from_millis(33), &cfg),
            Plan::NONE
        );
    }

    #[test]
    fn zero_interval_yields_no_spreading() {
        let cfg = SpreadConfig {
            min_spacing: Duration::from_millis(3),
            max_sleeps: 0,
        };
        assert_eq!(compute_spacing_and_group(10, Duration::ZERO, &cfg), Plan::NONE);
    }

    #[test]
    fn ideal_above_min_keeps_group_1() {
        // 33ms / 10 = 3.3ms > 3ms min -> group_n=1
        let cfg = SpreadConfig {
            min_spacing: Duration::from_millis(3),
            max_sleeps: 0,
        };
        let plan = compute_spacing_and_group(10, Duration::from_millis(33), &cfg);
        assert_eq!(plan.group_n, 1);
    }

    #[test]
    fn ideal_below_min_groups_up() {
        // 33ms / 100 = 0.33ms < 3ms -> group to reach min (10)
        let cfg = SpreadConfig {
            min_spacing: Duration::from_millis(3),
            max_sleeps: 0,
        };
        let plan = compute_spacing_and_group(100, Duration::from_millis(33), &cfg);
        assert!(plan.group_n >= 10);
    }
}
