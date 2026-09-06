//! Static image processing from decoded RGBA to target-sized RGB888.

use crate::control::fields::Fit;
use crate::error::ImageError;
use crate::image::decode::DecodedImage;
use crate::image::resize::{self, ResampleMethod};
use crate::image::unsharp::{self, UnsharpParams};
use crate::image::{gamma, icc};

pub struct PipelineParams {
    pub target_w: u32,
    pub target_h: u32,
    pub fit: Fit,
    pub method: ResampleMethod,
    pub gamma_correct: bool,
    pub color_correction: bool,
    pub unsharp: UnsharpParams,
}

pub struct ImagePipeline;

impl ImagePipeline {
    /// Applies ICC conversion, resize, fit and unsharp masking, returning RGB888.
    pub fn run(mut image: DecodedImage, params: &PipelineParams) -> Result<Vec<u8>, ImageError> {
        if params.color_correction {
            icc::to_srgb_inplace_soft(&mut image.rgba, image.icc_profile.as_deref());
        }

        let (inner_w, inner_h) = resize::compute_fit_size(
            image.width,
            image.height,
            params.target_w,
            params.target_h,
            params.fit,
        );

        let use_gamma = params.gamma_correct && params.method != ResampleMethod::Nearest;
        let resized = if use_gamma {
            resize_rgba_gamma_aware(
                &image.rgba,
                image.width,
                image.height,
                inner_w,
                inner_h,
                params.method,
            )?
        } else {
            resize::resize_rgba(
                &image.rgba,
                image.width,
                image.height,
                inner_w,
                inner_h,
                params.method,
            )?
        };

        let mut rgb888 = match params.fit {
            Fit::Cover => {
                resize::cover_rgba_to_rgb888(&resized, inner_w, inner_h, params.target_w, params.target_h)
            }
            Fit::Pad | Fit::Auto => {
                resize::letterbox_rgba_to_rgb888(&resized, inner_w, inner_h, params.target_w, params.target_h)
            }
        };

        if params.unsharp.amount > 0.0 {
            unsharp::unsharp_rgb888(&mut rgb888, params.target_w, params.target_h, &params.unsharp);
        }

        Ok(rgb888)
    }
}

/// Resizes RGBA through one U16x4 buffer, with RGB in linear light and alpha
/// widened by 257. Returns sRGB-encoded RGBA8 with rounded alpha.
fn resize_rgba_gamma_aware(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    method: ResampleMethod,
) -> Result<Vec<u8>, ImageError> {
    let n = (src_w * src_h) as usize;
    let mut src_u16 = vec![0u16; n * 4];
    for i in 0..n {
        let px = &src[i * 4..i * 4 + 4];
        let q = &mut src_u16[i * 4..i * 4 + 4];
        q[0] = gamma::SRGB_TO_LINEAR_U16[px[0] as usize];
        q[1] = gamma::SRGB_TO_LINEAR_U16[px[1] as usize];
        q[2] = gamma::SRGB_TO_LINEAR_U16[px[2] as usize];
        q[3] = u16::from(px[3]) * 257; // [0..255] -> [0..65535]
    }

    let dst_u16 = resize::resize_u16x4(&src_u16, src_w, src_h, dst_w, dst_h, method)?;

    let dst_n = (dst_w * dst_h) as usize;
    let mut out = vec![0u8; dst_n * 4];
    for i in 0..dst_n {
        let p = &dst_u16[i * 4..i * 4 + 4];
        out[i * 4] = gamma::LINEAR_TO_SRGB_U8[p[0] as usize];
        out[i * 4 + 1] = gamma::LINEAR_TO_SRGB_U8[p[1] as usize];
        out[i * 4 + 2] = gamma::LINEAR_TO_SRGB_U8[p[2] as usize];
        // Rounded division by 257 preserves the alpha widening round trip.
        out[i * 4 + 3] = ((u32::from(p[3]) + 128) / 257) as u8;
    }
    Ok(out)
}
