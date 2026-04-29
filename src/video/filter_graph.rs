//! Build the `-vf` filter graph string. Pure function, table-driven tests.
//!
//! Filter order matters: ffmpeg applies filters left-to-right and later
//! filters depend on upstream normalization (square pixels, post-rotation
//! orientation, range expansion baked into the downscale).
//!
//! Layout:
//! ```text
//! [crop_autocrop?] → scale iw*sar:ih → setsar=1 → transpose×N (rotation)
//!     → scale target (+ expand range) → pad|crop (fit mode) → setdar=1
//!     → format=rgb24
//! ```

use std::fmt::Write;

use crate::control::fields::Fit;

pub struct AutocropRect {
    pub l: u32,
    pub r: u32,
    pub t: u32,
    pub b: u32,
}

pub struct FilterGraphParams {
    /// Source dimensions (post-rotation), when known via a probe pass.
    /// When `None`, Auto-fit conservatively falls through to Pad — the
    /// smart direct-scale path needs source ratios to compare.
    pub src_dims: Option<(u32, u32)>,
    pub sar_num: u32,
    pub sar_den: u32,
    pub rotation_deg: u32, // 0/90/180/270
    pub target_width: u32,
    pub target_height: u32,
    pub fit: Fit,
    pub expand: u8, // 0=never, 1=auto, 2=force (tv→pc)
    pub autocrop: Option<AutocropRect>,
}

/// Build the `-vf` filter chain as a single string.
///
/// The output is wired into `ffmpeg -vf <this>` — comma-separated filter
/// entries — not the lower-level `filter_complex` graph syntax.
pub fn build_filter_graph(p: &FilterGraphParams) -> String {
    let mut s = String::new();

    // Autocrop first, in source pixel coordinates. Without a source-dims
    // probe we can't compute the crop rect — the dispatcher leaves
    // `autocrop: None` in that case.
    if let (Some(ac), Some((src_w, src_h))) = (&p.autocrop, p.src_dims) {
        let cw = src_w.saturating_sub(ac.l + ac.r).max(1);
        let ch = src_h.saturating_sub(ac.t + ac.b).max(1);
        let _ = write!(s, "crop={cw}:{ch}:{}:{},", ac.l, ac.t);
    }

    // Unsqueeze PAR, then force SAR=1 so downstream scale works on square pixels.
    s.push_str("scale=iw*sar:ih,setsar=1,");

    // Rotation via transpose. 180° is two `transpose=clock` chained.
    match p.rotation_deg {
        90 => s.push_str("transpose=clock,"),
        180 => s.push_str("transpose=clock,transpose=clock,"),
        270 => s.push_str("transpose=cclock,"),
        _ => {}
    }

    // Range-expand args live in the scale that performs the downscale to target.
    let expand_args: &str = match p.expand {
        2 => ":in_range=tv:out_range=pc",
        1 => ":in_range=auto:out_range=pc",
        _ => "",
    };

    match p.fit {
        Fit::Cover => {
            let _ = write!(
                s,
                "scale={}:{}:flags=bilinear:force_original_aspect_ratio=increase{},",
                p.target_width, p.target_height, expand_args
            );
            let _ = write!(
                s,
                "crop={}:{}:(in_w-{})/2:(in_h-{})/2,",
                p.target_width, p.target_height, p.target_width, p.target_height
            );
        }
        Fit::Auto => {
            // Scale direct if aspect ratios match; without source dims we
            // can't compare ratios, so fall through to Pad conservatively.
            let direct = p.src_dims.is_some_and(|(sw, sh)| {
                let src_ratio = (sw as f64 * p.sar_num as f64) / (sh as f64 * p.sar_den.max(1) as f64);
                let tgt_ratio = p.target_width as f64 / p.target_height.max(1) as f64;
                (src_ratio - tgt_ratio).abs() < 0.01
            });
            if direct {
                let _ = write!(
                    s,
                    "scale={}:{}:flags=bilinear{},",
                    p.target_width, p.target_height, expand_args
                );
            } else {
                pad_chain(&mut s, p, expand_args);
            }
        }
        Fit::Pad => pad_chain(&mut s, p, expand_args),
    }

    s.push_str("setdar=1,format=rgb24");
    s
}

fn pad_chain(s: &mut String, p: &FilterGraphParams, expand_args: &str) {
    let _ = write!(
        s,
        "scale={}:{}:flags=bilinear:force_original_aspect_ratio=decrease{},",
        p.target_width, p.target_height, expand_args
    );
    let _ = write!(
        s,
        "pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,",
        p.target_width, p.target_height
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic(fit: Fit) -> FilterGraphParams {
        FilterGraphParams {
            src_dims: Some((1920, 1080)),
            sar_num: 1,
            sar_den: 1,
            rotation_deg: 0,
            target_width: 64,
            target_height: 64,
            fit,
            expand: 0,
            autocrop: None,
        }
    }

    #[test]
    fn pad_chain_contains_pad_filter() {
        let s = build_filter_graph(&basic(Fit::Pad));
        assert!(s.contains("force_original_aspect_ratio=decrease"));
        assert!(s.contains("pad=64:64"));
    }

    #[test]
    fn cover_chain_contains_increase_and_crop() {
        let s = build_filter_graph(&basic(Fit::Cover));
        assert!(s.contains("force_original_aspect_ratio=increase"));
        assert!(s.contains("crop=64:64"));
    }

    #[test]
    fn auto_with_matching_ratio_is_direct_scale() {
        // 1:1 source → 64x64 target → ratios match → direct scale (no pad).
        let mut p = basic(Fit::Auto);
        p.src_dims = Some((1000, 1000));
        let s = build_filter_graph(&p);
        assert!(!s.contains("pad="));
        assert!(s.contains("scale=64:64"));
    }

    #[test]
    fn auto_without_src_dims_falls_through_to_pad() {
        let mut p = basic(Fit::Auto);
        p.src_dims = None;
        let s = build_filter_graph(&p);
        assert!(s.contains("pad=64:64"));
    }

    #[test]
    fn rotation_emits_transposes() {
        let mut p = basic(Fit::Pad);
        p.rotation_deg = 90;
        assert!(build_filter_graph(&p).contains("transpose=clock"));

        p.rotation_deg = 180;
        assert_eq!(build_filter_graph(&p).matches("transpose=clock").count(), 2);

        p.rotation_deg = 270;
        assert!(build_filter_graph(&p).contains("transpose=cclock"));
    }

    #[test]
    fn expand_force_tv_pc() {
        let mut p = basic(Fit::Pad);
        p.expand = 2;
        let s = build_filter_graph(&p);
        assert!(s.contains("in_range=tv:out_range=pc"));
    }

    #[test]
    fn autocrop_emits_crop_first() {
        let mut p = basic(Fit::Pad);
        p.autocrop = Some(AutocropRect {
            l: 10,
            r: 10,
            t: 5,
            b: 5,
        });
        let s = build_filter_graph(&p);
        assert!(s.starts_with("crop=1900:1070:10:5,"));
    }

    #[test]
    fn always_ends_rgb24() {
        assert!(build_filter_graph(&basic(Fit::Pad)).ends_with("format=rgb24"));
        assert!(build_filter_graph(&basic(Fit::Cover)).ends_with("format=rgb24"));
        assert!(build_filter_graph(&basic(Fit::Auto)).ends_with("format=rgb24"));
    }
}
