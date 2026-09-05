# media-proxy — Cutover Smoke Test

Checklist for validating the Rust port against a real LED device before publishing the Rust add-on. The retained default media paths are the
release gate; Home Assistant entity/template drawing was intentionally removed.

## Build

- [ ] `cargo build --release` — produces `target/release/media-proxy`.
- [ ] `cargo test --locked --all-targets` — all unit tests green.
- [ ] `python3 tests/smoke.py --binary target/release/media-proxy` — retained APIs and DDP pass.
- [ ] `cargo clippy --all-targets -- -D warnings` — clean.
- [ ] `cargo fmt --all --check` — clean.

## Startup

- [ ] `media-proxy --help` shows expected CLI flags.
- [ ] `media-proxy --host 127.0.0.1 --port 8788` binds and logs `server listening`.
- [ ] `curl http://127.0.0.1:8788/api/system/health` → `{"status":"ok","service":"media-proxy"}`.
- [ ] Ctrl-C → process drains, logs `shutdown signal received`, exits 0.
- [ ] `kill -TERM <pid>` (Unix) → same shutdown path.
- [ ] `MEDIA_PROXY__LOG__LEVEL=debug media-proxy` enables debug logs.

## Static-image path

Pick a local PNG and a 64×64 LED panel at `192.168.1.50:4048`, output `1`.
Websocket client sends (via `websocat` or similar):

```json
{"type":"hello","device_id":"smoke"}
{"type":"start_stream","out":1,"w":64,"h":64,"src":"file:///tmp/test.png","ddp_host":"192.168.1.50"}
```

- [ ] Device shows the image within 1 s.
- [ ] Server logs include `DDP reservation granted` + `DDP sender bound`.
- [ ] `{"type":"stop_stream","out":1}` → device freezes last frame; server
      logs `stream finished` and the reservation releases.
- [ ] Starting a second stream on the same `out=1` cancels the first
      (server log: `displacing conflicting DDP stream`).

## Animated path

- [ ] Start a looping GIF: server logs `animated cache hit` on the second
      start of the same URL+size+fit combo within the cache's MB budget.
- [ ] APNG with all three disposal methods composites visibly correctly
      (no ghosting on dispose-to-background, no gaps on dispose-to-previous).
- [ ] Animated WebP composites correctly (regression guard for
      `image-webp` #178/#179 fixes).
- [ ] Non-looping GIF stops after one pass (no repeat).

## REST endpoints

- [ ] `GET /api/internal/placeholder/64x64.png?text=HI` — PNG with "HI"
      rendered in the default Spleen font.
- [ ] `GET /api/internal/placeholder/128x32/red/white.png?text=ALERT` — red
      background, white text.
- [ ] `GET /api/internal/homeassistant/anything.png` — 501 (intentionally removed).

## Performance sanity

- [ ] Single 1080p video → 64×64 DDP at native cadence: CPU per stream
      reasonable (compare to Python baseline on same machine).
- [ ] Four concurrent streams: no single-thread bottleneck (should scale
      across cores; Python was GIL-bound here).
- [ ] `net.spread_packets=true` + `spread_max_fps=60`: `pps` log line
      shows consistent packet cadence, `pkt_jit` < half the frame interval.
- [ ] Paced mode (`pace=30`, `ema=0.3`): displays smooth motion, log
      shows `pace=30Hz fps≈30`.

## Hardware-accel smoke

Loop a 720p H.264 file and watch `ffmpeg` CPU vs. GPU usage:

- [ ] Linux: `-hwaccel vaapi` when available.
- [ ] Windows: `-hwaccel cuda` (NVIDIA) or `d3d11va` (any GPU).
- [ ] macOS: `-hwaccel videotoolbox`.

## Config

- [ ] `media-proxy --config path/to/config.yaml` loads and overrides
      defaults.
- [ ] `MEDIA_PROXY__IMAGE__GAMMA_CORRECT=true` toggles gamma-aware resize
      at runtime.
- [ ] Invalid file extension → error message, non-zero exit.

## Cutover

- [ ] Record the last Python core tag (`v0.5.11`) and take a Home Assistant add-on backup.
- [ ] Bump Rust crate version, cut release.
- [ ] Addon repo: bundle the Rust binary, update the entrypoint.
- [ ] Confirm release notes explicitly mention removal of Home Assistant entity/template drawing.
