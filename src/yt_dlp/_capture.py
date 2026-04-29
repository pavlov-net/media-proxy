# Re-run via:  python3 src/yt_dlp/_capture.py
#
# Mirrors the pre-rewrite Python build_yt_dlp_format() exactly. Re-extracts
# `try_60fps` as a parameter (Python read it from the global Config singleton)
# and writes one fixture file per (height, hw, prefer_60fps, video_only) combo
# so the Rust port can golden-test against them.

import json
import pathlib
import sys


def build_yt_dlp_format(width, height, mode=None, video_only=True, try_60fps=True):
    codec_preferences = {
        "vaapi": ["av01", "vp9", "vp09", "h265", "hevc", "hev1", "h264", "avc1", "avc3"],
        "qsv": ["h265", "hevc", "hev1", "h264", "avc1", "avc3", "av01", "vp9"],
        "cuda": ["av01", "h265", "hevc", "hev1", "h264", "avc1", "avc3", "vp9"],
        "videotoolbox": ["h264", "avc1", "avc3", "h265", "hevc", "hev1", "av01", "vp9"],
        "d3d11va": ["h264", "avc1", "avc3", "h265", "hevc", "hev1", "av01", "vp9"],
        None: ["h264", "avc1", "avc3", "vp9", "vp09", "h265", "hevc", "hev1", "av01"],
    }
    codecs = codec_preferences.get(mode, codec_preferences[None])
    vcodec_regex = "^(" + "|".join(codecs) + ")$"

    max_height = min(height * 4, 1080)
    if height <= 64:
        max_height = min(max_height, 480)
    elif height <= 128:
        max_height = min(max_height, 720)

    if height <= 144:
        resolutions = [144, 240, 360, 480]
    elif height <= 240:
        resolutions = [240, 144, 360, 480, 720]
    elif height <= 360:
        resolutions = [360, 240, 480, 720, 1080]
    elif height <= 480:
        resolutions = [480, 360, 240, 720, 1080]
    elif height <= 720:
        resolutions = [720, 1080, 480, 360, 240]
    else:
        resolutions = [1080, 720, 480, 360, 240, 144]

    resolutions = [r for r in resolutions if r <= max_height]
    if not resolutions:
        resolutions = [240]

    components = []

    if try_60fps:
        for codec in codecs:
            components.append(f"bv*[fps>=60][vcodec*={codec}][height>=720][height<=720][protocol=https]")
            if not video_only:
                components.append(f"b[fps>=60][vcodec*={codec}][height>=720][height<=720][protocol=https]")

    for res in resolutions:
        for codec in codecs:
            components.append(f"bv*[vcodec*={codec}][height>={res}][height<={res}][protocol=https]")
            if not video_only:
                components.append(f"b[vcodec*={codec}][height>={res}][height<={res}][protocol=https]")

    for res in resolutions:
        components.append(f"bv*[height>={res}][height<={res}][protocol=https]")
        if not video_only:
            components.append(f"b[height>={res}][height<={res}][protocol=https]")

    components.append(f'bv*[vcodec~="{vcodec_regex}"][height>={height}][protocol=https]')
    components.append(f"bv*[height>={height}][protocol=https]")
    components.append(f'bv*[vcodec~="{vcodec_regex}"][protocol=https]')
    components.append("bv*[protocol=https]")

    if not video_only:
        components.append(f'b[vcodec~="{vcodec_regex}"][height>={height}][protocol=https]')
        components.append(f"b[height>={height}][protocol=https]")
        components.append(f'b[vcodec~="{vcodec_regex}"][protocol=https]')
        components.append("b[protocol=https]")

    components.append("bv*")
    if not video_only:
        components.append("b")

    return "/".join(components)


HW = [None, "vaapi", "qsv", "cuda", "videotoolbox", "d3d11va"]
HEIGHTS = [32, 64, 100, 128, 144, 240, 360, 480, 720, 1080]
SIXTYFPS = [True, False]
VIDEO_ONLY = [True, False]


def main():
    out_dir = pathlib.Path(__file__).parent
    rows = []
    for h in HEIGHTS:
        for hw in HW:
            for sixty in SIXTYFPS:
                for vo in VIDEO_ONLY:
                    expr = build_yt_dlp_format(width=h, height=h, mode=hw,
                                               video_only=vo, try_60fps=sixty)
                    rows.append({
                        "height": h,
                        "hw": hw,
                        "prefer_60fps": sixty,
                        "video_only": vo,
                        "expr": expr,
                    })
    (out_dir / "fixtures.json").write_text(json.dumps(rows, indent=2) + "\n")
    print(f"wrote {len(rows)} fixtures to {out_dir / 'fixtures.json'}", file=sys.stderr)


if __name__ == "__main__":
    main()
