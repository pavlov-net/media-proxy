# Animated disposal fixtures

The generator assembles containers directly to preserve subframe rectangles and
disposal flags. Expected RGBA frames use Pillow compositing on raw subframes,
with disposal applied to the preceding rectangle. FFmpeg is not an oracle or a
generator dependency.

| Fixture | Cases |
| --- | --- |
| `disposal.gif` | KEEP, BACKGROUND, PREVIOUS, offset frames. |
| `palette-holes.gif` | Transparent holes, local/global palettes with different transparent indices, BACKGROUND, PREVIOUS. |
| `disposal.apng` | NONE, BACKGROUND, PREVIOUS, offset frames. |
| `default-image.apng` | Fallback IDAT image excluded from the animation. |
| `alpha.apng` | Transparent SOURCE, OVER onto transparency, overlapping alpha, BACKGROUND. |
| `grayscale-alpha.apng` | Grayscale/alpha expansion with blending and disposal. |
| `rgba16.apng` | Distinct high/low sample bytes, blending and disposal; expected RGBA8 uses the high bytes before compositing. |
| `disposal.webp` | Offset frames and clearing the preceding rectangle. |
| `alpha.webp` | SOURCE, OVER and BACKGROUND with transparency. |
| `disposal-edges.webp` | Subrect disposal before full-canvas OVER and opaque VP8 successors. The lossy patch is solid black for exact reference pixels. |

[`animated_disposal.rs`](../../animated_disposal.rs) checks frames and delays,
black-background RGB output, and warm-cache reuse. It tolerates
one unit of alpha rounding and ignores RGB beneath zero alpha.

The three WebP tests are ignored because `image-webp` 0.2.4 retains pixels in
disposed rectangles. Run them explicitly when evaluating dependency updates:

```sh
cargo test --test animated_disposal -- --ignored
```

To regenerate, install Pillow 12.3.0 and run from the repository root:

```sh
python3 tests/fixtures/animated/generate.py
```
