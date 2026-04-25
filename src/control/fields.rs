//! Control protocol field definitions + derived `StreamOptions`.
//!
//! Serde handles type-level validation; cross-field constraints land in
//! [`StreamFields::from_start`].

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::error::ControlError;
use crate::output::sink::PixelFormat;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    Pad,
    Cover,
    #[default]
    Auto,
}

impl Fit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pad => "pad",
            Self::Cover => "cover",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HwPref {
    #[default]
    Auto,
    None,
    Cuda,
    Qsv,
    Vaapi,
    Videotoolbox,
    D3d11va,
}

impl HwPref {
    /// Canonical lowercase name accepted by `ffmpeg -hwaccel` and our own
    /// `HwBackend::from_str_canon`. `None` → `None`.
    pub fn as_canon(self) -> Option<&'static str> {
        Some(match self {
            Self::Auto => "auto",
            Self::None => return None,
            Self::Cuda => "cuda",
            Self::Qsv => "qsv",
            Self::Vaapi => "vaapi",
            Self::Videotoolbox => "videotoolbox",
            Self::D3d11va => "d3d11va",
        })
    }
}

/// `start_stream` request.
///
/// Required: `out`, `w`, `h`, `src`. Everything else has a default derived
/// from config.
#[derive(Debug, Clone, Deserialize)]
pub struct StartStream {
    pub out: i32,
    pub w: u32,
    pub h: u32,
    pub src: String,
    #[serde(default)]
    pub ddp_port: Option<u16>,
    #[serde(default)]
    pub ddp_host: Option<String>,
    #[serde(default)]
    pub r#loop: Option<bool>,
    #[serde(default)]
    pub expand: Option<u8>,
    #[serde(default)]
    pub hw: Option<HwPref>,
    #[serde(default)]
    pub fit: Option<Fit>,
    #[serde(default)]
    pub fmt: Option<String>,
    #[serde(default)]
    pub pace: Option<u32>,
    #[serde(default)]
    pub ema: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StopStream {
    pub out: i32,
    #[serde(default)]
    pub ddp_host: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateStream {
    pub out: i32,
    #[serde(default)]
    pub ddp_port: Option<u16>,
    #[serde(default)]
    pub ddp_host: Option<String>,
    #[serde(default)]
    pub w: Option<u32>,
    #[serde(default)]
    pub h: Option<u32>,
    #[serde(default)]
    pub src: Option<String>,
    #[serde(default)]
    pub r#loop: Option<bool>,
    #[serde(default)]
    pub expand: Option<u8>,
    #[serde(default)]
    pub hw: Option<HwPref>,
    #[serde(default)]
    pub fit: Option<Fit>,
    #[serde(default)]
    pub fmt: Option<String>,
    #[serde(default)]
    pub pace: Option<u32>,
    #[serde(default)]
    pub ema: Option<f32>,
}

/// `applied` map in `ack` responses — the canonical view of the resolved
/// stream parameters returned to the client.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AppliedParams {
    pub src: String,
    pub pace: u32,
    pub ema: f32,
    pub expand: u8,
    pub r#loop: bool,
    pub hw: HwPref,
    pub fmt: String,
    pub fit: Fit,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ddp_host: Option<String>,
}

/// Narrow the wire-level `out: i32` to the DDP header's `out_id: u8`.
/// Wire format: low 8 bits verbatim (`out_id & 0xFF`).
#[inline]
pub fn output_id_byte(out: i32) -> u8 {
    (out & 0xFF) as u8
}

/// Upper bound on per-stream width/height. Anything we'd reasonably drive
/// onto an LED display is far below this; the cap only exists to refuse
/// inputs that would blow the per-frame allocation budget.
pub const MAX_OUTPUT_DIM: u32 = 4096;

/// Validated & resolved stream configuration. Construct from a `StartStream`
/// (or an `UpdateStream` layered over the previous state) via
/// [`StreamFields::from_start`].
#[derive(Debug, Clone)]
pub struct StreamFields {
    pub output_id: i32,
    pub width: u32,
    pub height: u32,
    pub source: String,
    pub ddp_port: u16,
    pub ddp_host: IpAddr,
    pub r#loop: bool,
    pub expand: u8,
    pub hw: HwPref,
    pub fit: Fit,
    pub fmt: PixelFormat,
    pub pace: u32,
    pub ema: f32,
}

impl StreamFields {
    pub fn from_start(
        req: &StartStream,
        client_ip: IpAddr,
        server_host: &str,
        defaults: &crate::Config,
    ) -> Result<Self, ControlError> {
        if req.w == 0 || req.h == 0 {
            return Err(ControlError::BadRequest("w/h must be > 0".into()));
        }
        // Cap output dimensions so a malicious/misconfigured client can't
        // force giant per-frame allocations in the video or animated paths
        // (frame bytes scale as w*h*channels).
        if req.w > MAX_OUTPUT_DIM || req.h > MAX_OUTPUT_DIM {
            return Err(ControlError::BadRequest(format!(
                "w/h must be ≤ {MAX_OUTPUT_DIM}"
            )));
        }
        let source =
            crate::stream::url::normalize_source(&req.src, server_host).map_err(ControlError::BadRequest)?;

        if req.out < 0 {
            return Err(ControlError::BadRequest("out must be ≥ 0".into()));
        }
        if matches!(req.ddp_port, Some(0)) {
            return Err(ControlError::BadRequest("ddp_port must be > 0".into()));
        }
        if let Some(ema) = req.ema
            && !(0.0..=1.0).contains(&ema)
        {
            return Err(ControlError::BadRequest("ema must be in [0.0, 1.0]".into()));
        }

        let expand = req.expand.unwrap_or(defaults.video.expand_mode);
        if !(0..=2).contains(&expand) {
            return Err(ControlError::BadRequest("expand must be 0, 1, or 2".into()));
        }

        let fit = req.fit.unwrap_or(match defaults.video.fit.as_str() {
            "pad" => Fit::Pad,
            "cover" => Fit::Cover,
            _ => Fit::Auto,
        });

        let fmt_str = req.fmt.clone().unwrap_or_else(|| "rgb888".into());
        let fmt = PixelFormat::from_str_canon(&fmt_str)
            .ok_or_else(|| ControlError::BadRequest(format!("unknown fmt: {fmt_str}")))?;

        let ddp_host_str = req.ddp_host.clone();
        let ddp_host: IpAddr = match ddp_host_str.as_deref() {
            Some(s) => s
                .parse()
                .map_err(|e| ControlError::BadRequest(format!("ddp_host parse: {e}")))?,
            None => client_ip,
        };

        Ok(Self {
            output_id: req.out,
            width: req.w,
            height: req.h,
            source,
            ddp_port: req.ddp_port.unwrap_or(4048),
            ddp_host,
            r#loop: req.r#loop.unwrap_or(defaults.playback.r#loop),
            expand,
            hw: req.hw.unwrap_or(HwPref::Auto),
            fit,
            fmt,
            pace: req.pace.unwrap_or(0),
            ema: req.ema.unwrap_or(0.0).clamp(0.0, 1.0),
        })
    }

    pub fn to_applied(&self) -> AppliedParams {
        AppliedParams {
            src: self.source.clone(),
            pace: self.pace,
            ema: self.ema,
            expand: self.expand,
            r#loop: self.r#loop,
            hw: self.hw,
            fmt: match self.fmt {
                PixelFormat::Rgb888 => "rgb888".into(),
                PixelFormat::Rgb565Le => "rgb565le".into(),
                PixelFormat::Rgb565Be => "rgb565be".into(),
            },
            fit: self.fit,
            ddp_host: Some(self.ddp_host.to_string()),
        }
    }
}

/// Overlay `update` fields onto a prior stream's resolved state. Any field
/// left `None` in the update keeps the prior value. The `src` field, if
/// present, runs through the same [`normalize_source`] pipeline as the
/// original `from_start`.
pub fn merge_update(
    prior: &StreamFields,
    upd: &UpdateStream,
    server_host: &str,
) -> Result<StreamFields, ControlError> {
    let mut out = prior.clone();
    if let Some(v) = upd.w {
        out.width = v;
    }
    if let Some(v) = upd.h {
        out.height = v;
    }
    if let Some(v) = upd.ddp_port {
        out.ddp_port = v;
    }
    if let Some(v) = upd.r#loop {
        out.r#loop = v;
    }
    if let Some(v) = upd.expand {
        out.expand = v;
    }
    if let Some(v) = upd.hw {
        out.hw = v;
    }
    if let Some(v) = upd.fit {
        out.fit = v;
    }
    if let Some(v) = upd.pace {
        out.pace = v;
    }
    if let Some(v) = upd.ema {
        out.ema = v.clamp(0.0, 1.0);
    }
    if let Some(ref s) = upd.src {
        out.source =
            crate::stream::url::normalize_source(s, server_host).map_err(ControlError::BadRequest)?;
    }
    if let Some(ref s) = upd.fmt
        && let Some(fmt) = PixelFormat::from_str_canon(s)
    {
        out.fmt = fmt;
    }
    if let Some(ref s) = upd.ddp_host
        && let Ok(ip) = s.parse::<IpAddr>()
    {
        out.ddp_host = ip;
    }
    Ok(out)
}
