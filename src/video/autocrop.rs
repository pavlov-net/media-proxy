//! Black-bar autocrop.
//!
//! Algorithm:
//! - Probe N frames.
//! - For each, walk inward from each edge; stop once a row/column's median
//!   luma exceeds `luma_thresh`.
//! - Cap walk distance at `max_bar_ratio * dim`.
//! - Record per-frame results; take per-edge median at end of probe.

use crate::video::filter_graph::AutocropRect;

/// Compute the median-of-medians for each edge over a set of per-frame samples.
pub fn median_rect(samples: &[AutocropRect]) -> Option<AutocropRect> {
    if samples.is_empty() {
        return None;
    }
    let mut ls: Vec<u32> = samples.iter().map(|s| s.l).collect();
    let mut rs: Vec<u32> = samples.iter().map(|s| s.r).collect();
    let mut ts: Vec<u32> = samples.iter().map(|s| s.t).collect();
    let mut bs: Vec<u32> = samples.iter().map(|s| s.b).collect();
    for v in [&mut ls, &mut rs, &mut ts, &mut bs] {
        v.sort_unstable();
    }
    let mid = samples.len() / 2;
    Some(AutocropRect {
        l: ls[mid],
        r: rs[mid],
        t: ts[mid],
        b: bs[mid],
    })
}
