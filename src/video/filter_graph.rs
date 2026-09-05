//! Builds ffmpeg filters in dependency order: autocrop, square-pixel conversion,
//! rotation, target scaling with range expansion, fitting, and RGB24 conversion.

use std::fmt::Write;

use crate::control::fields::Fit;

pub struct AutocropRect {
    pub l: u32,
    pub r: u32,
    pub t: u32,
    pub b: u32,
}

pub struct FilterGraphParams {
    /// Source dimensions after rotation. `None` makes Auto fit use padding.
    pub src_dims: Option<(u32, u32)>,
    pub sar_num: u32,
    pub sar_den: u32,
    pub rotation_deg: u32, // 0/90/180/270
    pub target_width: u32,
    pub target_height: u32,
    pub fit: Fit,
    pub expand: u8, // 0=ffmpeg defaults, 1=auto input to full, 2=limited input to full
    pub autocrop: Option<AutocropRect>,
}

/// Returns a comma-separated filter chain for ffmpeg `-vf`.
pub fn build_filter_graph(p: &FilterGraphParams) -> String {
    let mut s = String::new();

    // Crop edges use source pixels, before SAR correction and target scaling.
    if let (Some(ac), Some((src_w, src_h))) = (&p.autocrop, p.src_dims) {
        let cw = src_w.saturating_sub(ac.l + ac.r).max(1);
        let ch = src_h.saturating_sub(ac.t + ac.b).max(1);
        let _ = write!(s, "crop={cw}:{ch}:{}:{},", ac.l, ac.t);
    }

    // Normalize sample aspect ratio before target scaling.
    s.push_str("scale=iw*sar:ih,setsar=1,");

    // Two clockwise transposes produce a half-turn.
    match p.rotation_deg {
        90 => s.push_str("transpose=clock,"),
        180 => s.push_str("transpose=clock,transpose=clock,"),
        270 => s.push_str("transpose=cclock,"),
        _ => {}
    }

    // Apply range expansion during target scaling.
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
            // Unknown source dimensions prevent aspect-ratio comparison; use padding.
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
