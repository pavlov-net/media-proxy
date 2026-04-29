//! Map `HwBackend` to ffmpeg CLI `-hwaccel` flag + surface detection.
//!
//! `ffmpeg -hwaccels` lists *compiled-in* support, not what works at runtime
//! — distros ship binaries with `vaapi` enabled even on systems where
//! `/dev/dri/renderD*` isn't accessible. We probe each listed backend by
//! actually creating the hwdevice; only working ones are returned.

use std::process::Command;
use std::sync::OnceLock;

use tracing::{debug, info};

use crate::control::fields::HwPref;
use crate::platform::{self, HwBackend};

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
    let mut working = Vec::new();
    for line in text.lines().map(str::trim) {
        let Some(b) = HwBackend::from_str_canon(line) else {
            continue;
        };
        if probe(b) {
            debug!(backend = b.as_ffmpeg_flag(), "hwaccel probe ok");
            working.push(b);
        } else {
            info!(
                backend = b.as_ffmpeg_flag(),
                "hwaccel listed but device init failed; ignoring"
            );
        }
    }
    working
}

/// `-init_hw_device` calls `av_hwdevice_ctx_create` eagerly, so a failure
/// here mirrors what the real decode would hit.
fn probe(backend: HwBackend) -> bool {
    let device_arg = format!("{}=probe", backend.as_ffmpeg_flag());
    Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-init_hw_device",
            &device_arg,
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=2x2:r=1",
            "-frames:v",
            "1",
            "-f",
            "null",
            "-",
        ])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Resolve a stream's hardware-accel preference against the system's
/// available backends. `HwPref::None` opts out explicitly; `Auto` walks the
/// platform candidate list; a specific backend is honored if available.
pub fn pick_for(pref: HwPref) -> Option<HwBackend> {
    platform::pick_hw_backend(pref.as_canon().unwrap_or("none"), available())
}
