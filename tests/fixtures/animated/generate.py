"""Regenerate small disposal fixtures and reference frames (Pillow 12.3.0).

Containers are assembled directly to guarantee subrects and disposal flags;
FFmpeg is not used. Expected compositing uses Pillow on the raw subframes.
"""
import io
import json
from pathlib import Path
import struct
import zlib
from PIL import Image

ROOT = Path(__file__).parent
W, H = 6, 4
CLEAR = (0, 0, 0, 0)
RED, GREEN, BLUE = (255, 0, 0, 255), (0, 255, 0, 255), (0, 0, 255, 255)


def rgba(w, h, color):
    return Image.new('RGBA', (w, h), color)


def frame(x, y, image, dispose='none', blend='over', delay=70):
    return dict(x=x, y=y, image=image, dispose=dispose, blend=blend, delay=delay)


def reference(frames):
    canvas = rgba(W, H, CLEAR)
    output = []
    for f in frames:
        before = canvas.copy()
        patch = f['image']
        box = (f['x'], f['y'])
        if f['blend'] == 'source':
            canvas.paste(patch, box)
        else:
            canvas.alpha_composite(patch, box)
        output.append(dict(rgba=canvas.tobytes().hex(), delay_ms=f['delay']))
        if f['dispose'] == 'background':
            canvas.paste(CLEAR, (f['x'], f['y'], f['x'] + patch.width, f['y'] + patch.height))
        elif f['dispose'] == 'previous':
            canvas = before
    return output


def png_chunk(kind, data):
    return struct.pack('!I', len(data)) + kind + data + struct.pack('!I', zlib.crc32(kind + data))


def png_data(image, depth=8, color_type=6):
    samples = []
    for r, g, b, a in image.get_flattened_data():
        if color_type == 4:
            assert r == g == b
            samples.extend((r, a))
        else:
            samples.extend((r, g, b, a))
    if depth == 16:
        # Non-repeated bytes catch byte-order/STRIP_16 mistakes. The reference
        # uses each sample's high byte, matching conversion to RGBA8.
        rows = b''.join(struct.pack('!H', (v << 8) | (17 if 0 < v < 255 else v)) for v in samples)
    else:
        rows = bytes(samples)
    stride = image.width * (2 if color_type == 4 else 4) * (depth // 8)
    return zlib.compress(b''.join(b'\0' + rows[i:i + stride] for i in range(0, len(rows), stride)))


def apng(frames, separate=False, depth=8, color_type=6):
    out = b'\x89PNG\r\n\x1a\n' + png_chunk(b'IHDR', struct.pack('!II5B', W, H, depth, color_type, 0, 0, 0))
    out += png_chunk(b'acTL', struct.pack('!II', len(frames), 0))
    if separate:
        out += png_chunk(b'IDAT', png_data(rgba(W, H, BLUE), depth, color_type))
    seq = 0
    for i, f in enumerate(frames):
        im = f['image']
        ctl = struct.pack('!IIIIIHHBB', seq, im.width, im.height, f['x'], f['y'], f['delay'], 1000,
                          ['none', 'background', 'previous'].index(f['dispose']), int(f['blend'] == 'over'))
        out += png_chunk(b'fcTL', ctl)
        seq += 1
        if i == 0 and not separate:
            out += png_chunk(b'IDAT', png_data(im, depth, color_type))
        else:
            out += png_chunk(b'fdAT', struct.pack('!I', seq) + png_data(im, depth, color_type))
            seq += 1
    return out + png_chunk(b'IEND', b'')


def gif(frames):
    palette = bytes([0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255])
    out = b'GIF89a' + struct.pack('<HHBBB', W, H, 0x81, 0, 0) + palette
    for f in frames:
        im = f['image']
        colors = f.get('palette', [CLEAR, RED, GREEN, BLUE])
        local_palette = 'palette' in f
        disposal = {'none': 1, 'background': 2, 'previous': 3}[f['dispose']]
        out += b'!\xf9\x04' + struct.pack('<BHB', disposal * 4 + 1, f['delay'] // 10, colors.index(CLEAR)) + b'\0'
        out += b',' + struct.pack('<HHHHB', f['x'], f['y'], im.width, im.height, 0x81 if local_palette else 0)
        if local_palette:
            out += bytes(c for color in colors for c in color[:3])
        codes = []
        for pixel in im.get_flattened_data():
            codes += [4, colors.index(pixel)]
        codes += [5]
        bits = sum(code << (i * 3) for i, code in enumerate(codes))
        encoded = bits.to_bytes((len(codes) * 3 + 7) // 8, 'little')
        out += b'\x02' + bytes([len(encoded)]) + encoded + b'\0'
    return out + b';'


def riff_chunk(kind, data):
    return kind + struct.pack('<I', len(data)) + data + (b'\0' if len(data) % 2 else b'')


def u24(n):
    return n.to_bytes(3, 'little')


def webp(frames):
    out = riff_chunk(b'VP8X', bytes([0x12, 0, 0, 0]) + u24(W - 1) + u24(H - 1))
    out += riff_chunk(b'ANIM', bytes(6))
    for f in frames:
        im = f['image']
        buf = io.BytesIO()
        im.save(buf, format='WEBP', lossless=not f.get('lossy', False), exact=True)
        chunks = buf.getvalue()[12:]
        flags = int(f['dispose'] == 'background') | (int(f['blend'] == 'source') << 1)
        header = (u24(f['x'] // 2) + u24(f['y'] // 2) + u24(im.width - 1) + u24(im.height - 1)
                  + u24(f['delay']) + bytes([flags]))
        out += riff_chunk(b'ANMF', header + chunks)
    return b'RIFF' + struct.pack('<I', len(out) + 4) + b'WEBP' + out


opaque = [frame(0, 0, rgba(W, H, RED), blend='source', delay=40),
          frame(2, 2, rgba(2, 2, GREEN), dispose='previous', delay=70),
          frame(4, 0, rgba(2, 2, BLUE), dispose='background', delay=110),
          frame(0, 0, rgba(2, 2, GREEN), delay=130),
          frame(2, 2, rgba(2, 2, BLUE), delay=170)]
alpha = [frame(0, 0, rgba(W, H, CLEAR), blend='source', delay=40),
         frame(2, 2, rgba(2, 2, (255, 0, 0, 128)), dispose='background'),
         frame(4, 0, rgba(2, 2, BLUE), blend='source'),
         frame(4, 0, rgba(2, 2, CLEAR), blend='source'),
         frame(0, 0, rgba(2, 2, (0, 255, 0, 128))),
         frame(0, 0, rgba(2, 2, (0, 0, 255, 128)))]
webp_frames = [dict(f, dispose='none' if f['dispose'] == 'previous' else f['dispose']) for f in opaque]
webp_edges = [frame(0, 0, rgba(W, H, RED), blend='source'),
              frame(2, 2, rgba(2, 2, BLUE), dispose='background'),
              # A full-canvas OVER frame must only clear the previous rect.
              frame(0, 0, rgba(W, H, CLEAR)),
              frame(2, 2, rgba(2, 2, BLUE), dispose='background'),
              # An opaque VP8 subframe must still dispose the previous rect.
              dict(frame(4, 0, rgba(2, 2, (0, 0, 0, 255))), lossy=True)]


def holes(color):
    patch = rgba(2, 2, CLEAR)
    patch.putpixel((0, 0), color)
    patch.putpixel((1, 1), color)
    return patch


local_palette = [GREEN, BLUE, CLEAR, RED]  # Transparent index 2, not global index 0.
gif_holes = [frame(0, 0, rgba(W, H, RED), blend='source'),
             dict(frame(2, 2, holes(BLUE), dispose='previous'), palette=local_palette),
             frame(4, 0, holes(GREEN), dispose='background'),
             dict(frame(0, 0, holes(GREEN)), palette=local_palette),
             frame(0, 0, rgba(W, H, CLEAR))]
gray = [frame(0, 0, rgba(W, H, (64, 64, 64, 255)), blend='source'),
        frame(2, 2, rgba(2, 2, (128, 128, 128, 128)), dispose='previous'),
        frame(4, 0, rgba(2, 2, (192, 192, 192, 255)), dispose='background'),
        frame(0, 0, rgba(W, H, CLEAR)),
        frame(2, 2, rgba(2, 2, (32, 32, 32, 128)), blend='source')]
rgba16 = [frame(0, 0, rgba(W, H, (64, 96, 192, 255)), blend='source'),
          frame(2, 2, rgba(2, 2, (192, 64, 32, 128)), dispose='previous'),
          frame(4, 0, rgba(2, 2, (32, 192, 64, 255)), dispose='background'),
          frame(0, 0, rgba(W, H, CLEAR)),
          frame(2, 2, rgba(2, 2, (96, 32, 192, 128)), blend='source')]
fixtures = [('disposal.gif', gif(opaque), opaque),
            ('disposal.apng', apng(opaque), opaque),
            ('alpha.apng', apng(alpha), alpha),
            ('default-image.apng', apng(opaque, separate=True), opaque),
            ('disposal.webp', webp(webp_frames), webp_frames),
            ('alpha.webp', webp(alpha), alpha),
            ('disposal-edges.webp', webp(webp_edges), webp_edges),
            ('palette-holes.gif', gif(gif_holes), gif_holes),
            ('grayscale-alpha.apng', apng(gray, color_type=4), gray),
            ('rgba16.apng', apng(rgba16, depth=16), rgba16)]
for name, data, frames in fixtures:
    (ROOT / name).write_bytes(data)
    expected = reference(frames)
    (ROOT / (name + '.json')).write_text(json.dumps(dict(width=W, height=H, frames=expected), indent=2) + '\n')
