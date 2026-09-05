#![allow(clippy::unwrap_used)]
//! Independent raw-subframe/Pillow goldens. No FFmpeg or Python at test time.
use media_proxy::image::animated::{AnimatedDecoder, apng::ApngDecoder, gif::GifDecoder, webp::WebpDecoder};
use serde::Deserialize;

#[derive(Deserialize)]
struct Golden {
    width: u32,
    height: u32,
    frames: Vec<ExpectedFrame>,
}
#[derive(Deserialize)]
struct ExpectedFrame {
    rgba: String,
    delay_ms: f32,
}

#[test]
fn disposal_and_blending_match_independent_goldens() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/animated");
    for name in [
        "disposal.gif",
        "disposal.apng",
        "alpha.apng",
        "default-image.apng",
        "disposal.webp",
        "alpha.webp",
        "disposal-edges.webp",
        "palette-holes.gif",
        "grayscale-alpha.apng",
        "rgba16.apng",
    ] {
        let data = std::fs::read(root.join(name)).unwrap();
        let golden: Golden =
            serde_json::from_slice(&std::fs::read(root.join(format!("{name}.json"))).unwrap()).unwrap();
        // Also verify the production pipeline and warm frame cache preserve
        // compositing, alpha flattening and durations across playback loops.
        let cache = media_proxy::image::animated::cache::FrameCache::new(32, 5);
        let params = media_proxy::image::animated::dispatch::AnimatedDispatchParams {
            target_w: golden.width,
            target_h: golden.height,
            fit: media_proxy::control::fields::Fit::Auto,
            method: media_proxy::image::resize::ResampleMethod::Lanczos,
            gamma_correct: false,
            color_correction: true,
            unsharp: media_proxy::image::unsharp::UnsharpParams {
                radius: 0.6,
                amount: 0.0,
                threshold: 2,
            },
            source_url: name,
            r#loop: true,
        };
        let cold = media_proxy::image::animated::dispatch::dispatch(data.clone(), &params, &cache).unwrap();
        let warm = media_proxy::image::animated::dispatch::dispatch(data.clone(), &params, &cache).unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&cold, &warm),
            "{name} should use the frame cache"
        );
        assert_eq!(cold.frames.len(), golden.frames.len());
        for ((rgb, delay), expected) in cold.frames.iter().zip(&golden.frames) {
            assert!((*delay - expected.delay_ms).abs() < 0.01);
            let rgba = decode_hex(&expected.rgba);
            for (actual, expected) in rgb.as_chunks::<3>().0.iter().zip(rgba.as_chunks::<4>().0.iter()) {
                for c in 0..3 {
                    let reference = ((u16::from(expected[c]) * u16::from(expected[3]) + 127) / 255) as u8;
                    assert!(
                        actual[c].abs_diff(reference) <= 1,
                        "{name}: cached RGB {actual:?} vs RGBA {expected:?}"
                    );
                }
            }
        }

        // Decode twice to check disposal state resets on a new playback loop.
        for _ in 0..2 {
            let mut decoder = match name.rsplit('.').next().unwrap() {
                "gif" => AnimatedDecoder::Gif(Box::new(GifDecoder::new(data.clone(), name).unwrap())),
                "apng" => AnimatedDecoder::Apng(Box::new(ApngDecoder::new(data.clone(), name).unwrap())),
                "webp" => AnimatedDecoder::Webp(Box::new(WebpDecoder::new(data.clone(), name).unwrap())),
                _ => unreachable!(),
            };
            for (index, expected) in golden.frames.iter().enumerate() {
                let frame = decoder
                    .next_frame()
                    .unwrap_or_else(|e| panic!("{name} frame {index}: {e}"))
                    .unwrap_or_else(|| panic!("{name} frame {index} missing"));
                assert_eq!(
                    (frame.width, frame.height),
                    (golden.width, golden.height),
                    "{name}"
                );
                assert!(
                    (frame.delay_ms - expected.delay_ms).abs() < 0.01,
                    "{name} frame {index} timing"
                );
                let bytes = decode_hex(&expected.rgba);
                assert_eq!(frame.rgba.len(), bytes.len());
                for (pixel, (actual, expected)) in frame
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(bytes.as_chunks::<4>().0.iter())
                    .enumerate()
                {
                    // Transparent RGB is undefined. Allow one rounding unit
                    // for integer alpha compositors; never tolerate ghost pixels.
                    assert!(
                        actual[3].abs_diff(expected[3]) <= 1,
                        "{name} frame {index} pixel {pixel}: {actual:?} != {expected:?}"
                    );
                    if expected[3] > 0 {
                        for channel in 0..3 {
                            assert!(
                                actual[channel].abs_diff(expected[channel]) <= 1,
                                "{name} frame {index} pixel {pixel}: {actual:?} != {expected:?}"
                            );
                        }
                    }
                }
            }
            assert!(decoder.next_frame().unwrap().is_none(), "{name} has extra frames");
        }
    }
}

fn decode_hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks(2)
        .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .collect()
}
