# Animated disposal references

Regenerate with Pillow 12.3.0:

```sh
python generate.py
```

The generator constructs each container directly so encoders cannot optimize
away the cases being tested. There is no FFmpeg dependency. Expected RGBA frames
come from compositing the *raw subframes* with Pillow `alpha_composite` or SOURCE
replacement, applying disposal only to the prior frame's rectangle.

- `disposal.gif`: KEEP, BACKGROUND and PREVIOUS, with offset subframes.
- `disposal.apng`: NONE, BACKGROUND and PREVIOUS, with offset subframes.
- `default-image.apng`: same animation after an excluded fallback IDAT image.
- `alpha.apng`: transparent SOURCE replacement, OVER onto a transparent canvas,
  overlapping half-transparent frames, and background disposal.
- `disposal.webp`: offset frames and clearing the previous rectangle before a
  new frame at a different position.
- `alpha.webp`: the same alpha/SOURCE/BACKGROUND cases as APNG. WebP does not have
  a PREVIOUS disposal mode.

`legacy_rgb` captures the extra Pillow compositing loop from Python media-proxy
at `2a88de2^` (before resizing). GIF and opaque APNG fixtures match every legacy
frame. Some transparent APNG frames and WebP frames differ because the Python
loop composites Pillow's already-composited frames a second time. This can leave
ghost pixels or apply alpha twice. The Rust test uses the raw-subframe reference,
not those artifacts. Pillow/libwebp's direct WebP output matches all reference
frames; Pillow's APNG OVER path itself differs on some translucent cases.

The Rust integration test checks all 32 frames, frame delays, two fresh decoder
passes, black-background RGB output with the default image pipeline, and reuse
of the warm frame cache. Only one unit of alpha-rounding difference is tolerated;
RGB behind completely transparent pixels is ignored. These synthetic fixtures
complement, rather than replace, a check of real animations on an LED display.
