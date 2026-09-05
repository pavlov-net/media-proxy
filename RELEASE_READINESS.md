# Rust release readiness

Scope confirmed during the cutover review: retain media streaming and its usual
Python defaults. Home Assistant entity/template drawing and the animimg ZIP
utility are intentionally removed. The add-on supplies the standalone program's
runtime dependencies and Home Assistant lifecycle/configuration.

## Compatibility findings and fixes

| Area | Finding / resulting behavior |
| --- | --- |
| Defaults | Every retained default is compared with a captured copy of `src/config.py` from `2a88de2^`. Nested legacy YAML and uppercase log levels are covered. |
| Startup | Rust accepts the add-on's uppercase log levels. `hw.prefer` now applies when a stream leaves `hw` unset; explicit `none` also reaches the YouTube format selector. |
| Animated media | Fixed opaque WebP output buffer sizing, APNG offset-frame debug assertions, indexed PNG expansion, excluded fallback images, and straight-alpha OVER compositing. |
| Disposal | 32 independently generated reference frames exercise GIF/APNG/WebP disposal and blending, two decoder passes, default image processing and warm-cache playback. See `tests/fixtures/animated/README.md` for legacy differences. |
| DDP client | Reviewed `pavlov-net/ddp-esphome` at `89a28553df5214b007e6b1ad83d6b93fa4966c84`: `proto`, optional `pixcfg`, start/update/stop fields, RGB888/RGB565 headers and byte order. Preserve Python's RGB888 fallback for the client's `rgbw` option. |
| Streaming URLs | RTSP/RTMP/UDP/TCP and extensionless direct media bypass yt-dlp. FFmpeg receives the configured default user agent for HTTP inputs. |
| yt-dlp | External executable already supported. Isolated command arguments, headers, failures and deadlines are tested; yt-dlp and Deno are separate runtime dependencies. |
| Playback | Retained default looping/native cadence; paced producers now respect source timing and rewind animated sources instead of draining them immediately. |
| Text placeholders | Closing named-color tags and inline style boundaries no longer insert unwanted lines. This does not restore Home Assistant drawing. |
| CPU support | Removed mandatory x86-64-v3 and mold flags; release binaries use baseline x86-64. Rust is pinned to 1.98.1. |

## Automated validation

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked --bins
python3 tests/smoke.py --binary target/debug/media-proxy
```

The smoke test uses actual HTTP/WebSocket connections and UDP packets. It checks
still/GIF/APNG/WebP/video playback with default stream parameters, updates,
stop, a paced animation, pixel formats, extensionless HTTP video, and SIGTERM.
When yt-dlp is on PATH, it also extracts a local HTML video page and verifies DDP
output. The test needs FFmpeg, but no Python packages or external network.

The format selector also retains its pre-existing Python golden corpus. Most
original tests are unit tests; the new binary smoke and disposal corpus cover
important boundaries that were previously untested. This is not a claim of full
coverage or pixel-identical resizing across Pillow and Rust.

## Release / add-on order

1. Merge the core compatibility changes and the stacked release-packaging PR.
   The first Rust release is **v1.0.0**; mark it as a major release in the release
   drafter. Its tag must match `Cargo.toml` (the packaging workflow checks this).
2. Require both Linux musl builds and their binary smoke tests. Assets contain
   `media-proxy`, `ddp-view`, license notices and README, with per-archive SHA-256.
3. Publish the core release. Only after both archives are uploaded does the
   workflow notify the add-on. Prereleases do not notify it.
4. Merge the add-on migration once its pinned release exists and both amd64 and
   aarch64 image checks pass. It installs the verified binary, FFmpeg/ffprobe,
   `yt-dlp[default]` (including matching EJS), and stable Alpine Deno.
5. Before publishing the add-on, take a Home Assistant add-on backup, test the
   default paths on a real ddp-esphome display, and verify upgrade/reconnect and
   rollback. Keep the same slug, port, host networking, options and config path.
   Dependency dispatch creates a PR without enabling automatic merge.

## Checks still requiring the deployment environment

- Real YouTube extraction and playback. This sandbox's outbound network policy
  blocks YouTube; local yt-dlp extraction is not a substitute for its EJS challenge.
- Actual Home Assistant Supervisor upgrade/restart/backup restore and a display
  running ddp-esphome. No live HA instance or display is attached here.
- Hardware decoding on the target GPU/driver, sustained playback, frame pacing
  and visual quality on representative animations. The default software path
  is tested locally; synthetic goldens do not prove every file decodes correctly.
- The add-on Docker image must build and run on both architectures. This workspace
  has a Docker client but no daemon; the PR adds executable image checks to CI.

The current add-on migration targets `v1.0.0` and must remain unmerged until that
release's assets exist. No release or add-on upgrade is published by this review.
