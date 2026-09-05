//! Layered configuration via `figment`: defaults → file (yaml/toml/json) → env → runtime.

use std::path::Path;

use figment::{
    Figment,
    providers::{Env, Format, Json, Serialized, Toml, Yaml},
};
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::telemetry::LogLevel;

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub hw: HwConfig,
    #[serde(default)]
    pub video: VideoConfig,
    #[serde(default)]
    pub playback: PlaybackConfig,
    #[serde(default)]
    pub youtube: YoutubeConfig,
    #[serde(default)]
    pub image: ImageConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub net: NetConfig,
    #[serde(default)]
    pub playback_still: PlaybackStillConfig,
    #[serde(default)]
    pub resolver: ResolverConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HwConfig {
    #[serde(default = "default_hw_prefer")]
    pub prefer: String,
}

fn default_hw_prefer() -> String {
    "auto".to_string()
}

impl Default for HwConfig {
    fn default() -> Self {
        Self {
            prefer: default_hw_prefer(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VideoConfig {
    /// 0=never, 1=auto(limited→full), 2=force
    #[serde(default = "default_expand_mode")]
    pub expand_mode: u8,
    #[serde(default = "default_fit")]
    pub fit: String,
    #[serde(default)]
    pub autocrop: AutocropConfig,
}

fn default_expand_mode() -> u8 {
    2
}
fn default_fit() -> String {
    "auto".to_string()
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            expand_mode: default_expand_mode(),
            fit: default_fit(),
            autocrop: AutocropConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AutocropConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_probe_frames")]
    pub probe_frames: u32,
    #[serde(default = "default_luma_thresh")]
    pub luma_thresh: u8,
    #[serde(default = "default_max_bar_ratio")]
    pub max_bar_ratio: f32,
    #[serde(default = "default_min_bar_px")]
    pub min_bar_px: u32,
}

fn default_probe_frames() -> u32 {
    24
}
fn default_luma_thresh() -> u8 {
    16
}
fn default_max_bar_ratio() -> f32 {
    0.15
}
fn default_min_bar_px() -> u32 {
    2
}

impl Default for AutocropConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            probe_frames: default_probe_frames(),
            luma_thresh: default_luma_thresh(),
            max_bar_ratio: default_max_bar_ratio(),
            min_bar_px: default_min_bar_px(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaybackConfig {
    #[serde(default = "default_true")]
    pub r#loop: bool,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self { r#loop: true }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct YoutubeConfig {
    #[serde(default = "default_true", rename = "60fps")]
    pub prefer_60fps: bool,
    #[serde(default)]
    pub cache: YoutubeCacheConfig,
}

impl Default for YoutubeConfig {
    fn default() -> Self {
        Self {
            prefer_60fps: true,
            cache: YoutubeCacheConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct YoutubeCacheConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_yt_cache_max")]
    pub max_size: u64,
}

fn default_yt_cache_max() -> u64 {
    5 * 1024 * 1024
}

impl Default for YoutubeCacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_size: default_yt_cache_max(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageConfig {
    #[serde(default = "default_image_method")]
    pub method: String,
    #[serde(default)]
    pub gamma_correct: bool,
    #[serde(default = "default_true")]
    pub color_correction: bool,
    #[serde(default)]
    pub unsharp: UnsharpConfig,
    /// Max memory for cached frames (MB, 0 = disabled)
    #[serde(default = "default_frame_cache_mb")]
    pub frame_cache_mb: u32,
    /// Only cache if animation has ≥ N frames
    #[serde(default = "default_frame_cache_min_frames")]
    pub frame_cache_min_frames: u32,
}

fn default_image_method() -> String {
    "lanczos".to_string()
}
fn default_frame_cache_mb() -> u32 {
    32
}
fn default_frame_cache_min_frames() -> u32 {
    5
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            method: default_image_method(),
            gamma_correct: false,
            color_correction: true,
            unsharp: UnsharpConfig::default(),
            frame_cache_mb: default_frame_cache_mb(),
            frame_cache_min_frames: default_frame_cache_min_frames(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UnsharpConfig {
    #[serde(default)]
    pub amount: f32,
    #[serde(default = "default_unsharp_radius")]
    pub radius: f32,
    #[serde(default = "default_unsharp_threshold")]
    pub threshold: i32,
}

fn default_unsharp_radius() -> f32 {
    0.6
}
fn default_unsharp_threshold() -> i32 {
    2
}

impl Default for UnsharpConfig {
    fn default() -> Self {
        Self {
            amount: 0.0,
            radius: default_unsharp_radius(),
            threshold: default_unsharp_threshold(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogConfig {
    #[serde(default)]
    pub send_ms: bool,
    #[serde(default = "default_log_rate_ms")]
    pub rate_ms: u64,
    #[serde(default)]
    pub level: LogLevel,
    #[serde(default = "default_true")]
    pub metrics: bool,
}

fn default_log_rate_ms() -> u64 {
    5000
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            send_ms: false,
            rate_ms: default_log_rate_ms(),
            level: LogLevel::default(),
            metrics: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetConfig {
    #[serde(default = "default_true")]
    pub win_timer_res: bool,
    #[serde(default = "default_true")]
    pub spread_packets: bool,
    #[serde(default = "default_spread_max_fps")]
    pub spread_max_fps: u32,
    #[serde(default = "default_spread_min_ms")]
    pub spread_min_ms: f32,
    #[serde(default)]
    pub spread_max_sleeps: u32,
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
}

fn default_spread_max_fps() -> u32 {
    60
}
fn default_spread_min_ms() -> f32 {
    3.0
}
fn default_user_agent() -> String {
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/140.0.0.0 Safari/537.36"
        .to_string()
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            win_timer_res: true,
            spread_packets: true,
            spread_max_fps: default_spread_max_fps(),
            spread_min_ms: default_spread_min_ms(),
            spread_max_sleeps: 0,
            user_agent: default_user_agent(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaybackStillConfig {
    /// Send each packet this many times for reliability on still frames.
    #[serde(default = "default_redundancy")]
    pub redundancy: u32,
}

fn default_redundancy() -> u32 {
    3
}

impl Default for PlaybackStillConfig {
    fn default() -> Self {
        Self {
            redundancy: default_redundancy(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResolverConfig {
    /// Optional external resolver URL. When unset, detect yt-dlp on PATH;
    /// direct media works even when neither resolver is available.
    pub url: Option<String>,
    #[serde(default = "default_resolver_timeout")]
    pub timeout_ms: u64,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            url: None,
            timeout_ms: default_resolver_timeout(),
        }
    }
}

fn default_resolver_timeout() -> u64 {
    // Cold-start budget: Python interpreter (~300ms) + yt-dlp imports
    // (~500ms) + Deno spawn (~1s) + youtubei API + EJS solve (~2-4s).
    // Warm runs land near 2s; first call after install can be 5-10s.
    30_000
}

fn default_true() -> bool {
    true
}

impl Config {
    /// Load configuration with defaults → optional file → env (`MEDIA_PROXY__*`) overlay.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut fig = Figment::from(Serialized::defaults(Self::default()));

        if let Some(p) = path {
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            fig = match ext.as_str() {
                "yaml" | "yml" => fig.merge(Yaml::file(p)),
                "toml" => fig.merge(Toml::file(p)),
                "json" => fig.merge(Json::file(p)),
                other => {
                    return Err(ConfigError::Invalid(format!("unknown config extension: {other}")));
                }
            };
        }

        fig = fig.merge(Env::prefixed("MEDIA_PROXY__").split("__"));
        let cfg: Self = fig.extract()?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_final_python_config() {
        // Captured from src/config.py at the parent of rewrite commit 2a88de2.
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../tests/python-config-defaults.json")).unwrap();
        // JSON serialization uses the shortest f32 representation, avoiding
        // artificial f64 widening differences such as 0.6 -> 0.600000024.
        let mut actual: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&Config::default()).unwrap()).unwrap();
        actual.as_object_mut().unwrap().remove("resolver");
        assert_eq!(actual, expected);
    }

    #[test]
    fn legacy_yaml_merges_nested_defaults_and_uppercase_logging() {
        let mut file = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        use std::io::Write;
        write!(file, "hw:\n  prefer: none\nyoutube:\n  60fps: false\nimage:\n  unsharp:\n    amount: 0.5\nlog:\n  level: WARNING\n").unwrap();
        let config = Config::load(Some(file.path())).unwrap();
        assert!(!config.youtube.prefer_60fps);
        assert_eq!(config.log.level, LogLevel::Warning);
        assert_eq!(config.image.unsharp.amount, 0.5);
        assert_eq!(config.image.unsharp.radius, 0.6);
        assert_eq!(config.hw.prefer, "none");
    }

    #[test]
    fn defaults_are_stable() {
        let c = Config::default();
        assert_eq!(c.hw.prefer, "auto");
        assert_eq!(c.video.expand_mode, 2);
        assert_eq!(c.video.fit, "auto");
        assert!(!c.video.autocrop.enabled);
        assert!(c.playback.r#loop);
        assert!(c.youtube.prefer_60fps);
        assert!(c.youtube.cache.enabled);
        assert_eq!(c.youtube.cache.max_size, 5 * 1024 * 1024);
        assert_eq!(c.image.method, "lanczos");
        assert!(!c.image.gamma_correct);
        assert!(c.image.color_correction);
        assert_eq!(c.image.frame_cache_mb, 32);
        assert_eq!(c.image.frame_cache_min_frames, 5);
        assert_eq!(c.log.rate_ms, 5000);
        assert!(c.net.win_timer_res);
        assert!(c.net.spread_packets);
        assert_eq!(c.net.spread_max_fps, 60);
        assert_eq!(c.playback_still.redundancy, 3);
        assert!(c.resolver.url.is_none());
        assert_eq!(c.resolver.timeout_ms, 30_000);
    }

    #[test]
    fn defaults_survive_figment_round_trip() {
        let fig = Figment::from(Serialized::defaults(Config::default()));
        let cfg: Config = fig.extract().expect("extract");
        assert_eq!(cfg.resolver.timeout_ms, 30_000);
        assert_eq!(cfg.hw.prefer, "auto");
        assert!(cfg.youtube.cache.enabled);
    }
}
