//! Image resize with `fast_image_resize` 6.x.
//!
//! Implements LANCZOS/BICUBIC/BILINEAR/BOX/NEAREST plus AUTO (scale-dependent,
//! integer-ratio aware).

use fast_image_resize::PixelType;
use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};

use crate::control::fields::Fit;
use crate::error::ImageError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResampleMethod {
    Lanczos,
    Bicubic,
    Bilinear,
    Box,
    Nearest,
    Auto,
}

impl ResampleMethod {
    pub fn from_str_canon(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "bicubic" => Self::Bicubic,
            "bilinear" => Self::Bilinear,
            "box" => Self::Box,
            "nearest" => Self::Nearest,
            "auto" => Self::Auto,
            _ => Self::Lanczos,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lanczos => "lanczos",
            Self::Bicubic => "bicubic",
            Self::Bilinear => "bilinear",
            Self::Box => "box",
            Self::Nearest => "nearest",
            Self::Auto => "auto",
        }
    }
}

/// Pick the resize algorithm for AUTO based on scale + integer-ratio detection.
///
/// Upscales stay crisp via Nearest; modest downscales with a near-integer
/// pixel ratio also pick Nearest so pixel-art survives (LED-art friendly);
/// everything else uses Box for anti-aliased downscales.
fn auto_alg(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> ResizeAlg {
    if src_w == 0 || src_h == 0 {
        return ResizeAlg::Nearest;
    }
    let scale = (f64::from(dst_w) / f64::from(src_w)).min(f64::from(dst_h) / f64::from(src_h));
    if scale >= 1.0 {
        return ResizeAlg::Nearest;
    }
    if scale >= 0.5 {
        let x_ratio = f64::from(src_w) / f64::from(dst_w);
        let y_ratio = f64::from(src_h) / f64::from(dst_h);
        let avg = (x_ratio + y_ratio) / 2.0;
        if (avg - avg.round()).abs() < 0.1 {
            return ResizeAlg::Nearest;
        }
    }
    ResizeAlg::Convolution(FilterType::Box)
}

fn alg_for(method: ResampleMethod, src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> ResizeAlg {
    match method {
        ResampleMethod::Lanczos => ResizeAlg::Convolution(FilterType::Lanczos3),
        ResampleMethod::Bicubic => ResizeAlg::Convolution(FilterType::CatmullRom),
        ResampleMethod::Bilinear => ResizeAlg::Convolution(FilterType::Bilinear),
        ResampleMethod::Box => ResizeAlg::Convolution(FilterType::Box),
        ResampleMethod::Nearest => ResizeAlg::Nearest,
        ResampleMethod::Auto => auto_alg(src_w, src_h, dst_w, dst_h),
    }
}

/// Compute `(new_w, new_h)` for a given fit mode and target size.
///
/// `Pad` and `Cover` scale to fit / fill respectively. `Auto` scales directly
/// when aspect ratios match (within 1%), else falls back to `Pad`.
pub fn compute_fit_size(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32, fit: Fit) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (dst_w.max(1), dst_h.max(1));
    }
    let sr = src_w as f64 / src_h as f64;
    let tr = dst_w as f64 / dst_h as f64;
    let (sw, sh) = (src_w as f64, src_h as f64);
    let (tw, th) = (dst_w as f64, dst_h as f64);
    let scale = match fit {
        Fit::Cover => (tw / sw).max(th / sh),
        Fit::Pad => (tw / sw).min(th / sh),
        Fit::Auto => {
            if (sr - tr).abs() < 0.01 {
                return (dst_w, dst_h);
            }
            (tw / sw).min(th / sh)
        }
    };
    let new_w = (sw * scale).round().max(1.0) as u32;
    let new_h = (sh * scale).round().max(1.0) as u32;
    (new_w, new_h)
}

/// Resize an RGBA source into a new RGBA buffer at `(dst_w, dst_h)`.
pub fn resize_rgba(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    method: ResampleMethod,
) -> Result<Vec<u8>, ImageError> {
    let src_image =
        ImageRef::new(src_w, src_h, src, PixelType::U8x4).map_err(|e| ImageError::Resize(e.to_string()))?;
    let mut dst_image = Image::new(dst_w, dst_h, PixelType::U8x4);

    let alg = alg_for(method, src_w, src_h, dst_w, dst_h);
    let mut resizer = Resizer::new();
    resizer
        .resize(&src_image, &mut dst_image, &ResizeOptions::new().resize_alg(alg))
        .map_err(|e| ImageError::Resize(e.to_string()))?;
    Ok(dst_image.into_vec())
}

/// Resize a single u8 grayscale plane.
pub fn resize_u8(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    method: ResampleMethod,
) -> Result<Vec<u8>, ImageError> {
    let src_image =
        ImageRef::new(src_w, src_h, src, PixelType::U8).map_err(|e| ImageError::Resize(e.to_string()))?;
    let mut dst_image = Image::new(dst_w, dst_h, PixelType::U8);
    let alg = alg_for(method, src_w, src_h, dst_w, dst_h);
    Resizer::new()
        .resize(&src_image, &mut dst_image, &ResizeOptions::new().resize_alg(alg))
        .map_err(|e| ImageError::Resize(e.to_string()))?;
    Ok(dst_image.into_vec())
}

/// Resize a single u16 grayscale plane (used by the gamma-aware path).
pub fn resize_u16(
    src: &[u16],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    method: ResampleMethod,
) -> Result<Vec<u16>, ImageError> {
    let bytes: &[u8] = bytemuck::cast_slice(src);
    let src_image =
        ImageRef::new(src_w, src_h, bytes, PixelType::U16).map_err(|e| ImageError::Resize(e.to_string()))?;
    let mut dst_image = Image::new(dst_w, dst_h, PixelType::U16);
    let alg = alg_for(method, src_w, src_h, dst_w, dst_h);
    Resizer::new()
        .resize(&src_image, &mut dst_image, &ResizeOptions::new().resize_alg(alg))
        .map_err(|e| ImageError::Resize(e.to_string()))?;
    let raw = dst_image.into_vec();
    let out: Vec<u16> = bytemuck::cast_slice(&raw).to_vec();
    Ok(out)
}

/// Blend an RGBA pixel against a black background → RGB888.
#[inline]
fn blend_over_black(rgba: &[u8]) -> [u8; 3] {
    let (r, g, b, a) = (rgba[0], rgba[1], rgba[2], rgba[3]);
    if a == 255 {
        return [r, g, b];
    }
    let af = u16::from(a);
    [
        ((u16::from(r) * af + 127) / 255) as u8,
        ((u16::from(g) * af + 127) / 255) as u8,
        ((u16::from(b) * af + 127) / 255) as u8,
    ]
}

/// Placement plan for compositing an RGBA source onto a black RGB888 canvas.
pub struct CompositePlan {
    pub src_w: u32,
    pub src_off: (u32, u32),
    pub dst_off: (u32, u32),
    pub copy: (u32, u32),
    pub target: (u32, u32),
}

/// Composite an RGBA source onto a black `target` canvas. `src_off` picks the
/// top-left corner in the source (positive for center-crop / `Cover`);
/// `dst_off` picks where in the target canvas the region lands (positive for
/// letterbox / `Pad`).
pub fn composite_rgba_to_rgb888(rgba: &[u8], plan: &CompositePlan) -> Vec<u8> {
    let (target_w, target_h) = plan.target;
    let (copy_w, copy_h) = plan.copy;
    let (src_off_x, src_off_y) = plan.src_off;
    let (dst_off_x, dst_off_y) = plan.dst_off;

    let mut out = vec![0u8; (target_w * target_h * 3) as usize];
    for y in 0..copy_h {
        for x in 0..copy_w {
            let si = (((y + src_off_y) * plan.src_w) + (x + src_off_x)) as usize * 4;
            let pixel = blend_over_black(&rgba[si..si + 4]);
            let oi = (((y + dst_off_y) * target_w) + (x + dst_off_x)) as usize * 3;
            out[oi..oi + 3].copy_from_slice(&pixel);
        }
    }
    out
}

/// Letterbox a smaller RGBA image onto a `target_w × target_h` black canvas.
pub fn letterbox_rgba_to_rgb888(
    rgba: &[u8],
    inner_w: u32,
    inner_h: u32,
    target_w: u32,
    target_h: u32,
) -> Vec<u8> {
    let off_x = (target_w.saturating_sub(inner_w)) / 2;
    let off_y = (target_h.saturating_sub(inner_h)) / 2;
    let copy_w = inner_w.min(target_w);
    let copy_h = inner_h.min(target_h);
    composite_rgba_to_rgb888(
        rgba,
        &CompositePlan {
            src_w: inner_w,
            src_off: (0, 0),
            dst_off: (off_x, off_y),
            copy: (copy_w, copy_h),
            target: (target_w, target_h),
        },
    )
}

/// Center-crop a larger RGBA image into a `target_w × target_h` canvas.
pub fn cover_rgba_to_rgb888(
    rgba: &[u8],
    inner_w: u32,
    inner_h: u32,
    target_w: u32,
    target_h: u32,
) -> Vec<u8> {
    let off_x = (inner_w.saturating_sub(target_w)) / 2;
    let off_y = (inner_h.saturating_sub(target_h)) / 2;
    let copy_w = target_w.min(inner_w);
    let copy_h = target_h.min(inner_h);
    composite_rgba_to_rgb888(
        rgba,
        &CompositePlan {
            src_w: inner_w,
            src_off: (off_x, off_y),
            dst_off: (0, 0),
            copy: (copy_w, copy_h),
            target: (target_w, target_h),
        },
    )
}
