"""Regenerate small disposal fixtures and reference frames (Pillow 12.3.0).

Containers are assembled directly to guarantee subrects and disposal flags;
FFmpeg is not used. Expected compositing uses Pillow on the raw subframes.
The legacy_rgb capture reproduces Python media-proxy's extra compositing step.
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


def black_rgb(image):
    bg = rgba(W, H, (0, 0, 0, 255))
    bg.alpha_composite(image)
    return bg.convert('RGB').tobytes()


def capture_legacy(data, skip_default=False):
    result = []
    canvas = rgba(W, H, (0, 0, 0, 255))
    with Image.open(io.BytesIO(data)) as image:
        for i in range(int(skip_default), image.n_frames):
            image.seek(i)
            image.load()
            disposal = getattr(image, 'disposal_method', None)
            disposal = {0: 0, 1: 0, 2: 1, 3: 2}.get(disposal, image.info.get('disposal', 0))
            before = canvas.copy()
            patch = image.convert('RGBA')
            if getattr(image, 'blend_op', 1) == 0:
                canvas.paste(patch, (0, 0))
            else:
                canvas.paste(patch, (0, 0), patch)
            result.append(black_rgb(canvas).hex())
            if disposal == 1:
                canvas = rgba(W, H, (0, 0, 0, 255))
            elif disposal == 2:
                canvas = before
    return result


def png_chunk(kind, data):
    return struct.pack('!I', len(data)) + kind + data + struct.pack('!I', zlib.crc32(kind + data))


def png_data(image):
    rows = image.tobytes()
    return zlib.compress(b''.join(b'\0' + rows[i:i + image.width * 4] for i in range(0, len(rows), image.width * 4)))


def apng(frames, separate=False):
    out = b'\x89PNG\r\n\x1a\n' + png_chunk(b'IHDR', struct.pack('!II5B', W, H, 8, 6, 0, 0, 0))
    out += png_chunk(b'acTL', struct.pack('!II', len(frames), 0))
    if separate:
        out += png_chunk(b'IDAT', png_data(rgba(W, H, BLUE)))
    seq = 0
    for i, f in enumerate(frames):
        im = f['image']
        ctl = struct.pack('!IIIIIHHBB', seq, im.width, im.height, f['x'], f['y'], f['delay'], 1000,
                          ['none', 'background', 'previous'].index(f['dispose']), int(f['blend'] == 'over'))
        out += png_chunk(b'fcTL', ctl)
        seq += 1
        if i == 0 and not separate:
            out += png_chunk(b'IDAT', png_data(im))
        else:
            out += png_chunk(b'fdAT', struct.pack('!I', seq) + png_data(im))
            seq += 1
    return out + png_chunk(b'IEND', b'')


def gif(frames):
    palette = bytes([0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255])
    out = b'GIF89a' + struct.pack('<HHBBB', W, H, 0x81, 0, 0) + palette
    for f in frames:
        im = f['image']
        disposal = {'none': 1, 'background': 2, 'previous': 3}[f['dispose']]
        out += b'!\xf9\x04' + struct.pack('<BHB', disposal * 4 + 1, f['delay'] // 10, 0) + b'\0'
        out += b',' + struct.pack('<HHHHB', f['x'], f['y'], im.width, im.height, 0)
        codes = []
        for pixel in im.get_flattened_data():
            codes += [4, [CLEAR, RED, GREEN, BLUE].index(pixel)]
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
        im.save(buf, format='WEBP', lossless=True, exact=True)
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
fixtures = [('disposal.gif', gif(opaque), opaque, False),
            ('disposal.apng', apng(opaque), opaque, False),
            ('alpha.apng', apng(alpha), alpha, False),
            ('default-image.apng', apng(opaque, separate=True), opaque, True),
            ('disposal.webp', webp(webp_frames), webp_frames, False),
            ('alpha.webp', webp(alpha), alpha, False)]
for name, data, frames, separate in fixtures:
    (ROOT / name).write_bytes(data)
    expected = reference(frames)
    legacy = capture_legacy(data, separate)
    for f, old in zip(expected, legacy):
        f['legacy_rgb'] = old
    (ROOT / (name + '.json')).write_text(json.dumps(dict(width=W, height=H, frames=expected), indent=2) + '\n')
    matches = sum(black_rgb(Image.frombytes('RGBA', (W, H), bytes.fromhex(f['rgba']))).hex() == old
                  for f, old in zip(expected, legacy))
    print(name, 'legacy matches', matches, '/', len(frames))
