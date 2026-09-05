# Vendored image-webp

Source: https://github.com/image-rs/image-webp

Revision: `f4d80bd965df2c81e65b6f43c1f70e0750bd4b0f` (upstream main,
version 0.2.4). The upstream source, manifest, README and MIT/Apache-2.0
licenses are preserved. This snapshot includes the post-release upstream
canvas-corruption fixes previously consumed through the git dependency.

Two local disposal corrections:

- `src/decoder.rs`: apply BACKGROUND disposal before opaque VP8 successors too;
  selecting the clear color must not depend on the next frame's alpha.
- `src/extended.rs`: clear only the previous frame's rectangle, including when
  the next frame is a full-canvas transparent OVER frame.

Both are covered independently by `tests/fixtures/animated/disposal-edges.webp`
and `tests/animated_disposal.rs` in media-proxy. The generator builds raw
subframes and reference compositing with Pillow, independently of image-webp.

Only media-proxy's direct animated decoder uses this path dependency. The
ordinary crates.io image-webp dependency used by the `image` crate is unchanged.
Replace this snapshot with an upstream release once both corrections and the
existing post-release fixes are available there.
