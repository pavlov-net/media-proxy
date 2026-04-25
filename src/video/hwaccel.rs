//! Map `HwBackend` to ffmpeg CLI `-hwaccel` flag + surface detection.

use std::process::Command;
use std::sync::OnceLock;

use crate::control::fields::HwPref;
use crate::platform::{self, HwBackend};

/// Query `ffmpeg -hwaccels` once and cache the result.
pub fn available() -> &'static [HwBackend] {
    static CACHE: OnceLock<Vec<HwBackend>> = OnceLock::new();
    CACHE.get_or_init(detect).as_slice()
}

fn detect() -> Vec<HwBackend> {
    let Ok(out) = Command::new("ffmpeg")
        .args(["-hide_banner", "-hwaccels"])
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut v = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(b) = HwBackend::from_str_canon(line) {
            v.push(b);
        }
    }
    v
}

/// Build the `-hwaccel <name>` args. Returns an empty slice for "none".
pub fn cli_args(backend: Option<HwBackend>) -> Vec<&'static str> {
    match backend {
        None => Vec::new(),
        Some(b) => vec!["-hwaccel", b.as_ffmpeg_flag()],
    }
}

/// Resolve a stream's hardware-accel preference against the system's
/// available backends. `HwPref::None` opts out explicitly; `Auto` walks the
/// platform candidate list; a specific backend is honored if available.
pub fn pick_for(pref: HwPref) -> Option<HwBackend> {
    platform::pick_hw_backend(pref.as_canon().unwrap_or("none"), available())
}
