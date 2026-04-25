//! yt-dlp `-f` format-selector builder. Pure string assembly from target
//! height, hwaccel hint, and 60fps preference. Always prefers video-only
//! progressive HTTPS streams; falls through to combined / any protocol only
//! as last resort. Tiny displays are capped — pulling 1080p to render at
//! 64×64 wastes bandwidth.

use crate::platform::HwBackend;

#[derive(Debug, Clone, Copy)]
pub struct FormatParams {
    pub height: u32,
    pub hw: Option<HwBackend>,
    pub prefer_60fps: bool,
    pub video_only: bool,
}

fn codecs_for(hw: Option<HwBackend>) -> &'static [&'static str] {
    match hw {
        Some(HwBackend::Vaapi) => &[
            "av01", "vp9", "vp09", "h265", "hevc", "hev1", "h264", "avc1", "avc3",
        ],
        Some(HwBackend::Qsv) => &["h265", "hevc", "hev1", "h264", "avc1", "avc3", "av01", "vp9"],
        Some(HwBackend::Cuda) => &["av01", "h265", "hevc", "hev1", "h264", "avc1", "avc3", "vp9"],
        Some(HwBackend::Videotoolbox) => &["h264", "avc1", "avc3", "h265", "hevc", "hev1", "av01", "vp9"],
        Some(HwBackend::D3d11va) => &["h264", "avc1", "avc3", "h265", "hevc", "hev1", "av01", "vp9"],
        None => &[
            "h264", "avc1", "avc3", "vp9", "vp09", "h265", "hevc", "hev1", "av01",
        ],
    }
}

fn max_height_for(target: u32) -> u32 {
    let cap = (target.saturating_mul(4)).min(1080);
    if target <= 64 {
        cap.min(480)
    } else if target <= 128 {
        cap.min(720)
    } else {
        cap
    }
}

fn resolution_ladder(target: u32) -> &'static [u32] {
    if target <= 144 {
        &[144, 240, 360, 480]
    } else if target <= 240 {
        &[240, 144, 360, 480, 720]
    } else if target <= 360 {
        &[360, 240, 480, 720, 1080]
    } else if target <= 480 {
        &[480, 360, 240, 720, 1080]
    } else if target <= 720 {
        &[720, 1080, 480, 360, 240]
    } else {
        &[1080, 720, 480, 360, 240, 144]
    }
}

/// Append a `bv*[…]` arm and (when not video-only) the matching `b[…]` arm.
fn push_pair(comps: &mut Vec<String>, video_only: bool, body: &str) {
    comps.push(format!("bv*{body}"));
    if !video_only {
        comps.push(format!("b{body}"));
    }
}

pub fn build_format(p: &FormatParams) -> String {
    let codecs = codecs_for(p.hw);
    let vcodec_regex = format!("^({})$", codecs.join("|"));

    let max_h = max_height_for(p.height);
    let ladder = resolution_ladder(p.height);
    let resolutions: Vec<u32> = ladder.iter().copied().filter(|r| *r <= max_h).collect();
    // Tiny targets can fully filter the ladder; fall back to one tier so the
    // selector always has at least one resolution-pinned arm.
    let resolutions = if resolutions.is_empty() {
        vec![240]
    } else {
        resolutions
    };

    let mut comps: Vec<String> = Vec::new();

    if p.prefer_60fps {
        // 60fps streams only exist at ≥720p on YouTube — pin to 720 regardless
        // of the target height.
        for codec in codecs {
            push_pair(
                &mut comps,
                p.video_only,
                &format!("[fps>=60][vcodec*={codec}][height>=720][height<=720][protocol=https]"),
            );
        }
    }

    for &res in &resolutions {
        for codec in codecs {
            push_pair(
                &mut comps,
                p.video_only,
                &format!("[vcodec*={codec}][height>={res}][height<={res}][protocol=https]"),
            );
        }
    }

    for &res in &resolutions {
        push_pair(
            &mut comps,
            p.video_only,
            &format!("[height>={res}][height<={res}][protocol=https]"),
        );
    }

    comps.push(format!(
        "bv*[vcodec~=\"{vcodec_regex}\"][height>={}][protocol=https]",
        p.height
    ));
    comps.push(format!("bv*[height>={}][protocol=https]", p.height));
    comps.push(format!("bv*[vcodec~=\"{vcodec_regex}\"][protocol=https]"));
    comps.push("bv*[protocol=https]".to_string());

    if !p.video_only {
        comps.push(format!(
            "b[vcodec~=\"{vcodec_regex}\"][height>={}][protocol=https]",
            p.height
        ));
        comps.push(format!("b[height>={}][protocol=https]", p.height));
        comps.push(format!("b[vcodec~=\"{vcodec_regex}\"][protocol=https]"));
        comps.push("b[protocol=https]".to_string());
    }

    // Final any-protocol fallbacks — for live streams and DASH/HLS edge cases
    // where no https progressive format exists.
    comps.push("bv*".to_string());
    if !p.video_only {
        comps.push("b".to_string());
    }

    comps.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Fixture {
        height: u32,
        hw: Option<String>,
        prefer_60fps: bool,
        video_only: bool,
        expr: String,
    }

    fn parse_hw(s: Option<&str>) -> Option<HwBackend> {
        s.and_then(HwBackend::from_str_canon)
    }

    #[test]
    fn matches_python_golden_fixtures() {
        let raw = include_str!("../../tests/fixtures/yt_dlp_format/fixtures.json");
        let fixtures: Vec<Fixture> = serde_json::from_str(raw).expect("parse fixtures");
        assert!(!fixtures.is_empty(), "no fixtures loaded");

        let mut failures = Vec::new();
        for fx in &fixtures {
            let got = build_format(&FormatParams {
                height: fx.height,
                hw: parse_hw(fx.hw.as_deref()),
                prefer_60fps: fx.prefer_60fps,
                video_only: fx.video_only,
            });
            if got != fx.expr {
                failures.push(format!(
                    "MISMATCH height={} hw={:?} 60fps={} video_only={}\n  expected: {}\n  got:      {}",
                    fx.height, fx.hw, fx.prefer_60fps, fx.video_only, fx.expr, got
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "{} fixture mismatches:\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }

    #[test]
    fn small_target_falls_back_to_240() {
        let s = build_format(&FormatParams {
            height: 32,
            hw: None,
            prefer_60fps: false,
            video_only: true,
        });
        assert!(s.contains("[height>=240][height<=240]"));
        assert!(!s.contains("[height>=144]"));
    }

    #[test]
    fn prefer_60fps_emits_720p_preamble() {
        let with = build_format(&FormatParams {
            height: 64,
            hw: None,
            prefer_60fps: true,
            video_only: true,
        });
        let without = build_format(&FormatParams {
            height: 64,
            hw: None,
            prefer_60fps: false,
            video_only: true,
        });
        assert!(with.starts_with("bv*[fps>=60]"));
        assert!(!without.contains("[fps>=60]"));
    }

    #[test]
    fn cuda_prefers_av1_first() {
        let s = build_format(&FormatParams {
            height: 720,
            hw: Some(HwBackend::Cuda),
            prefer_60fps: false,
            video_only: true,
        });
        let first_arm = s.split('/').next().expect("at least one arm");
        assert!(
            first_arm.contains("vcodec*=av01"),
            "cuda first arm should be av01, got: {first_arm}"
        );
    }

    #[test]
    fn videotoolbox_prefers_h264_first() {
        let s = build_format(&FormatParams {
            height: 720,
            hw: Some(HwBackend::Videotoolbox),
            prefer_60fps: false,
            video_only: true,
        });
        let first_arm = s.split('/').next().expect("at least one arm");
        assert!(
            first_arm.contains("vcodec*=h264"),
            "videotoolbox first arm should be h264, got: {first_arm}"
        );
    }

    #[test]
    fn always_ends_with_bv_star() {
        let s = build_format(&FormatParams {
            height: 720,
            hw: None,
            prefer_60fps: true,
            video_only: true,
        });
        assert!(s.ends_with("/bv*"));
    }

    #[test]
    fn video_only_has_no_b_arms() {
        let s = build_format(&FormatParams {
            height: 720,
            hw: None,
            prefer_60fps: true,
            video_only: true,
        });
        for arm in s.split('/') {
            assert!(!arm.starts_with("b["), "leaked combined arm: {arm}");
            assert_ne!(arm, "b", "trailing `b` should be absent for video_only");
        }
    }
}
