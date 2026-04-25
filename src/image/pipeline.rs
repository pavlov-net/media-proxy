//! End-to-end static image pipeline: decoded RGBA → target RGB888.

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
    /// Run the pipeline: ICC → resize (in linear light when gamma-aware) →
    /// fit → unsharp → RGB888 output.
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

/// Gamma-aware resize: sRGB u8 → linear u16 per channel, resize each RGB
/// channel in linear light (alpha stays in sRGB space as a mask), linear u16
/// → sRGB u8. Approximates Pillow's `gamma_correct` path.
fn resize_rgba_gamma_aware(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    method: ResampleMethod,
) -> Result<Vec<u8>, ImageError> {
    // Split the RGBA source into four grayscale planes. We resize RGB in
    // linear light and alpha in sRGB space (matching PIL's behavior — alpha
    // isn't a color channel).
    let n = (src_w * src_h) as usize;
    let mut r_lin = vec![0u16; n];
    let mut g_lin = vec![0u16; n];
    let mut b_lin = vec![0u16; n];
    let mut a_src = vec![0u8; n];
    for i in 0..n {
        let px = &src[i * 4..i * 4 + 4];
        r_lin[i] = gamma::SRGB_TO_LINEAR_U16[px[0] as usize];
        g_lin[i] = gamma::SRGB_TO_LINEAR_U16[px[1] as usize];
        b_lin[i] = gamma::SRGB_TO_LINEAR_U16[px[2] as usize];
        a_src[i] = px[3];
    }

    let r_resized = resize::resize_u16(&r_lin, src_w, src_h, dst_w, dst_h, method)?;
    let g_resized = resize::resize_u16(&g_lin, src_w, src_h, dst_w, dst_h, method)?;
    let b_resized = resize::resize_u16(&b_lin, src_w, src_h, dst_w, dst_h, method)?;
    let a_resized = resize::resize_u8(&a_src, src_w, src_h, dst_w, dst_h, method)?;

    let dst_n = (dst_w * dst_h) as usize;
    let mut out = vec![0u8; dst_n * 4];
    for i in 0..dst_n {
        out[i * 4] = gamma::LINEAR_TO_SRGB_U8[r_resized[i] as usize];
        out[i * 4 + 1] = gamma::LINEAR_TO_SRGB_U8[g_resized[i] as usize];
        out[i * 4 + 2] = gamma::LINEAR_TO_SRGB_U8[b_resized[i] as usize];
        out[i * 4 + 3] = a_resized[i];
    }
    Ok(out)
}
