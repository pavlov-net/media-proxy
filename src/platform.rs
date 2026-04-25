//! OS-specific hooks. Currently just Windows timer resolution.

#[cfg(windows)]
pub struct WindowsTimerResolution {
    enabled: bool,
}

#[cfg(windows)]
impl WindowsTimerResolution {
    /// Calls `timeBeginPeriod(1)` on construction, `timeEndPeriod(1)` on drop.
    /// No-ops when `enable` is false.
    pub fn new(enable: bool) -> Self {
        if enable {
            // SAFETY: `timeBeginPeriod` is a stable Win32 API; `1` is a valid period.
            unsafe {
                windows_sys::Win32::Media::timeBeginPeriod(1);
            }
        }
        Self { enabled: enable }
    }
}

#[cfg(windows)]
impl Drop for WindowsTimerResolution {
    fn drop(&mut self) {
        if self.enabled {
            unsafe {
                windows_sys::Win32::Media::timeEndPeriod(1);
            }
        }
    }
}

#[cfg(not(windows))]
pub struct WindowsTimerResolution;

#[cfg(not(windows))]
impl WindowsTimerResolution {
    #[allow(dead_code)]
    pub fn new(_enable: bool) -> Self {
        Self
    }
}

/// Pick the best hardware-accel backend given a preference + `ffmpeg -hwaccels` output.
///
/// `prefer` is one of `"auto"`, `"none"`, or a specific backend name. When `"auto"`,
/// walks a platform-specific candidate list. Returns `None` if nothing matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwBackend {
    Cuda,
    Qsv,
    Vaapi,
    D3d11va,
    Videotoolbox,
}

impl HwBackend {
    pub fn as_ffmpeg_flag(self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::Qsv => "qsv",
            Self::Vaapi => "vaapi",
            Self::D3d11va => "d3d11va",
            Self::Videotoolbox => "videotoolbox",
        }
    }

    pub fn from_str_canon(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "cuda" => Some(Self::Cuda),
            "qsv" => Some(Self::Qsv),
            "vaapi" => Some(Self::Vaapi),
            "d3d11" | "d3d11va" => Some(Self::D3d11va),
            "videotoolbox" => Some(Self::Videotoolbox),
            _ => None,
        }
    }
}

/// Platform-appropriate candidate order when `prefer == "auto"`.
pub fn auto_candidates() -> &'static [HwBackend] {
    #[cfg(target_os = "windows")]
    {
        &[HwBackend::Cuda, HwBackend::D3d11va, HwBackend::Qsv]
    }
    #[cfg(target_os = "macos")]
    {
        &[HwBackend::Videotoolbox]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &[HwBackend::Vaapi, HwBackend::Qsv, HwBackend::Cuda]
    }
}

pub fn pick_hw_backend(prefer: &str, available: &[HwBackend]) -> Option<HwBackend> {
    match prefer.to_ascii_lowercase().as_str() {
        "" | "auto" => auto_candidates().iter().copied().find(|c| available.contains(c)),
        "none" => None,
        other => HwBackend::from_str_canon(other).filter(|b| available.contains(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_auto_returns_first_available() {
        let avail = [HwBackend::Qsv, HwBackend::Vaapi];
        let picked = pick_hw_backend("auto", &avail);
        // On Linux, auto tries vaapi first.
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(picked, Some(HwBackend::Vaapi));
        #[cfg(target_os = "windows")]
        assert_eq!(picked, Some(HwBackend::Qsv)); // cuda/d3d11 missing → qsv
        #[cfg(target_os = "macos")]
        assert_eq!(picked, None); // videotoolbox missing
    }

    #[test]
    fn pick_specific_honored() {
        let avail = [HwBackend::Cuda];
        assert_eq!(pick_hw_backend("cuda", &avail), Some(HwBackend::Cuda));
        assert_eq!(pick_hw_backend("d3d11", &avail), None);
    }

    #[test]
    fn pick_none() {
        let avail = [HwBackend::Cuda];
        assert_eq!(pick_hw_backend("none", &avail), None);
    }
}
