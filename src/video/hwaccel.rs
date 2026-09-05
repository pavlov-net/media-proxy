//! Detects usable ffmpeg hardware backends by initializing each compiled-in
//! device type. Listed backends can lack an accessible runtime device.

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

/// Tests device creation; compiled-in support alone does not guarantee access.
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

/// Selects an available backend, using platform order for Auto and CPU for None.
pub fn pick_for(pref: HwPref) -> Option<HwBackend> {
    platform::pick_hw_backend(pref.as_canon().unwrap_or("none"), available())
}
