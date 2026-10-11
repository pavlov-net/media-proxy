# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Rust cutover
- Pin Rust 1.98.1 and retain the Python configuration defaults and ddp-esphome control/DDP formats.
- Fix animated disposal/blending, opaque WebP decoding, paced playback, and external resolver edge cases.
- Add independent disposal goldens and tests of the actual server and UDP output.
- Remove the animimg ZIP utility. Home Assistant entity/template drawing remains intentionally removed.
- Remove the mandatory AVX2 build baseline; standalone Linux binaries are prepared separately.

### Added
- winloop for improved async performance on Windows (~5x faster event loop vs default)
- Physical packet send tracking with microsecond-precision timing for accurate metrics during redundancy
- Enhanced DDP metrics: physical PPS, redundancy multiplier, and separate jitter tracking for unique vs physical packets

### Changed
- Simplified still frame redundancy configuration to single `playback_still.redundancy` setting (replaces `burst`, `spacing_ms`, `tail_s`, `tail_hz`)
- DDP still frame redundancy now sends duplicate packets immediately per DDP spec (no spacing delays)
- Metrics RateMeters now self-manage via sliding window (no manual clearing, prevents memory leaks)
- DDP metrics logging shows redundancy impact: `pps=45 (135phy, 3.0×)` when redundancy is active

### Fixed
- Fixed RateMeter memory leak from inconsistent clearing (only one of three meters was being cleared)
- Fixed duplicate timestamp pollution in packet jitter calculations during redundant sends
- Removed unused dead code: `last_frame_data`, `last_frame_seq`, `last_frame_was_still`

## [0.5.7] - 2025-10-15

### Added
- `internal:homeassistant` protocol for rendering Home Assistant entity states and templates to PNG
  - Entity mode: `internal:homeassistant/64x64.png?entity=sensor.name` (recommended)
  - Template mode: `internal:homeassistant/64x64.png?template={{ states('sensor.temp') }}` (fallback)
  - Development mode support via `HA_TOKEN` and `HA_URL` environment variables for testing outside addon
  - HEAD request optimization for efficient probing without rendering
- BBCode text rendering support for `internal:homeassistant` protocol
  - Word wrapping based on pixel width (not character count) for accurate text layout
  - Spleen bitmap fonts (5x8, 6x12, 8x16) for crisp LED display rendering
  - BBCode tags for rich text formatting:
    - `[color=red]...[/color]` or `[red]...[/red]` - Text colors (named or hex)
    - `[font=8x16]...[/font]` - Font size selection
    - `[left]`, `[center]`, `[right]` - Text alignment
    - `[b]...[/b]` - Bold text (simulated with double-draw)
  - URL query parameters: `?font=8x16&align=center` for default font and alignment
  - Plain text still works without any tags (backward compatible)
  - Material Design Icons webfont bundled (for future icon support)
- Third-party license documentation in `THIRD_PARTY_LICENSES.md` for bundled fonts

### Changed
- Switch to uv for dependency management (from pip)
- `internal:homeassistant` now uses Spleen 8x16 bitmap font by default (was PIL default font)
- `internal:placeholder` now defaults to black background instead of gray (better for LED displays)
- HEAD request optimization moved to top of handlers for maximum efficiency

## [0.5.1] - 2025-10-01

### Fixed
- YouTube URL matching now specifically requires https protocol to prevent playlist and other format mismatches

## [0.5.0] - 2025-10-01

### Added
- YouTube video caching for small looping videos (< 5MB) to reduce bandwidth and improve loop performance
- HTTP reconnect options for improved YouTube streaming reliability
- Enhanced error logging with codec, hardware acceleration, and frame count diagnostics

### Changed
- Video iterators now handle looping internally for better resource management

### Fixed
- YouTube streaming I/O errors and connection drops now automatically reconnect
- Video container resource leaks on exceptions
- Type checking errors for Windows-specific ctypes.windll on non-Windows platforms

## [0.4.1] - 2025-09-26

### Added
- Add auto fit mode and make it the default

### Changed
- Code cleanup
- Disable autocrop by default

### Fixed
- Correctly handle fit modes in image processing
- Fix animimg config example

## [0.4.0] - 2025-09-26

### Added
- Add new protocol handlers to create stream keys for avoiding duplicate streams
- Add image cache for animations to improve performance
- Add convert to animimg API functionality
- Add safety measures to prevent multiple DDP streams to the same IP/output combination

### Changed
- Switch from websockets library to aiohttp for WebSocket handling
- Use yt-dlp[default,curl-cffi] for enhanced YouTube downloading capabilities
- Use URLs internally for source handling instead of file paths
- Simplify video loading process
- Make DDP logging more consistent

## [0.3.5] - 2025-09-25

### Added
- Add `youtube.60fps` config option to control 60fps format preference
- Support `auto` option for image resizing method

### Changed
- Major refactor: Replace dictionary-based configuration with strongly-typed StreamOptions
- Rework YouTube format selection algorithm to favor lower resolution when appropriate for target display size
- Update README documentation

### Fixed
- Fix 60fps YouTube format selection logic

## [0.3.4] - 2025-09-24

### Added
- Add ha-addon/build.yaml file to specify container build platforms

### Changed
- Remove armv7 architecture from container builds

### Fixed
- Rework YouTube format selection to favor lower resolution when possible

## [0.3.3] - 2025-09-24

### Added
- Add ability to set 'fit' from websocket control
- Explicitly include zlib for armv7 builds

### Changed
- Switch prints to logging for better control

### Security
- Bump tj-actions/changed-files from 45 to 47

## [0.3.2] - 2025-09-23

### Fixed
- Fix race condition in non-looping image packet transmission where tasks could exit before all UDP packets (including still frame resends) were fully sent

## [0.3.1] - 2025-09-23

### Added
- Add AppArmor security profile for Home Assistant add-on

### Fixed
- Fix exception handling

## [0.3.0] - 2025-09-23

### Changed
- Changed from sleeping a fixed delay after each frame to tracking absolute target frame times, preventing timing drift

## [0.2.9] - 2025-09-21

### Added
- Add color correction for files with ICC profiles in them
- Optimize background color fetching to only do once

### Changed
- For smaller images, load them in to memory rather than saving them to a tempfile

## [0.2.8] - 2025-09-21

### Added
- Add proper disposal handling for GIFs and APNGs

## [0.2.7] - 2025-09-21

### Fixed
- Force lines to flush after each newline
- Fix can_handle calls to look at the path but not query params
- Add a user agent to avoid some bot blockers and better handle 403s and other errors

### Changed
- Blend all transparent images against a black background

## [0.2.6] - 2025-09-21

### Fixed
- Fix webp framerate issue

## [0.2.5] - 2025-09-21

### Changed
- Improve how image resampling works

### Fixed
- Fix animated GIF performance issue

## [0.2.4] - 2025-09-17

### Changed
- Only include intel driver on x86
- Switch from abandoned jitterbit/get-changed-files to tj-actions/changed-files
- Bump home-assistant/builder from 2025.03.0 to 2025.09.0

### Fixed
- Fix linter issues

## [0.2.3] - 2025-09-17

### Added
- Add github workflows to build container packages

### Changed
- Remove unnecessary defaults on config.get calls since they already have a default value set

### Fixed
- Restore missing log configuration options from legacy server

## [0.2.0] - 2025-09-17

### Added
- Refactor media processing to protocol-based architecture
- Enable color range expansion by default
- Always prefer hardware acceleration when available
- Add development file exclusions to gitignore

### Changed
- Update README to reflect refactoring & bugfixes
- Reduce metrics logging frequency to minimize console spam
- Refactor control field management
- Migrate from imageio to PIL for image processing
- Refactor server.py in to separate files
- Handle youtube url resolution async
- Disable gamma correction by default

## [0.1.0] - 2025-09-16

### Added
- Change default config path to /config/media-proxy/config.yaml
- Use uvloop on non-Windows platforms
- Add README
- Add home assistant add-on files

### Changed
- Use rounding instead of flooring when converting from rgb888 to rgb565 and add slight shadow lift

### Fixed
- Initial release from https://github.com/stuartparmenter/lvgl-ddp-stream

[unreleased]: https://github.com/stuartparmenter/media-proxy/compare/v0.5.1...HEAD
[0.5.1]: https://github.com/stuartparmenter/media-proxy/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/stuartparmenter/media-proxy/compare/v0.4.1...v0.5.0
[0.3.5]: https://github.com/stuartparmenter/media-proxy/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/stuartparmenter/media-proxy/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/stuartparmenter/media-proxy/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/stuartparmenter/media-proxy/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/stuartparmenter/media-proxy/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/stuartparmenter/media-proxy/compare/v0.2.9...v0.3.0
[0.2.9]: https://github.com/stuartparmenter/media-proxy/compare/v0.1.0...v0.2.9
[0.2.8]: https://github.com/stuartparmenter/media-proxy/compare/v0.1.0...v0.2.8
[0.2.7]: https://github.com/stuartparmenter/media-proxy/compare/v0.1.0...v0.2.7
[0.2.6]: https://github.com/stuartparmenter/media-proxy/compare/v0.1.0...v0.2.6
[0.2.5]: https://github.com/stuartparmenter/media-proxy/compare/v0.1.0...v0.2.5
[0.2.4]: https://github.com/stuartparmenter/media-proxy/compare/v0.1.0...v0.2.4
[0.2.3]: https://github.com/stuartparmenter/media-proxy/compare/v0.1.0...v0.2.3
[0.2.0]: https://github.com/stuartparmenter/media-proxy/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/stuartparmenter/media-proxy/releases/tag/v0.1.0
