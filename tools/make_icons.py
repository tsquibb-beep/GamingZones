"""Draws res/gamingzones.ico, res/ruler.ico and res/gz-editor.ico (plus res/gz-editor.rgba, a raw
64x64 RGBA copy the editor uses as its window icon).

Pure standard library (zlib + struct), 4x supersampled for anti-aliasing.
Run from anywhere: python3 tools/make_icons.py
"""
import pathlib
import struct
import zlib

SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
SS = 4
RES = pathlib.Path(__file__).resolve().parent.parent / "res"

SIDE = ((0x6C, 0x7E, 0xA4), (0x46, 0x53, 0x72))   # slate zones
GAME = ((0x4A, 0xE8, 0x8C), (0x1C, 0xA0, 0x58))   # green gaming zone
RULER = ((0xFF, 0xD3, 0x4E), (0xF0, 0xA2, 0x1C))  # yellow ruler body

# Gaming-Focus in miniature: two stacked zones each side, a big gaming zone in the middle.
GZ_SHAPES = [
    (0.02, 0.14, 0.24, 0.48, SIDE), (0.02, 0.52, 0.24, 0.86, SIDE),
    (0.28, 0.14, 0.72, 0.86, GAME),
    (0.76, 0.14, 0.98, 0.48, SIDE), (0.76, 0.52, 0.98, 0.86, SIDE),
]
# The editor uses the same icon as the tray app (Tom's choice).
EDITOR_SHAPES = GZ_SHAPES
# A ruler with tick marks cut out of its top edge.
RULER_SHAPES = [(0.02, 0.28, 0.98, 0.72, RULER)]
RULER_TICKS = [(0.10 + i * 0.1, 0.28, 0.10 + i * 0.1 + 0.035, 0.28 + (0.24 if i % 5 == 0 else 0.14)) for i in range(9)]


def inside_round_rect(x, y, x0, y0, x1, y1, r):
    if x < x0 or x > x1 or y < y0 or y > y1:
        return False
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def render(size, shapes, holes):
    n = size * SS
    rgba = bytearray(size * size * 4)
    for py in range(size):
        for px in range(size):
            acc = [0.0, 0.0, 0.0, 0.0]
            for sy in range(SS):
                for sx in range(SS):
                    x = (px * SS + sx + 0.5) / n
                    y = (py * SS + sy + 0.5) / n
                    if any(x0 <= x <= x1 and y0 <= y <= y1 for x0, y0, x1, y1 in holes):
                        continue
                    for x0, y0, x1, y1, (top, bot) in shapes:
                        if inside_round_rect(x, y, x0, y0, x1, y1, 0.05):
                            t = (y - y0) / (y1 - y0)
                            for c in range(3):
                                acc[c] += top[c] + (bot[c] - top[c]) * t
                            acc[3] += 1
                            break
            o = (py * size + px) * 4
            if acc[3]:
                rgba[o:o + 4] = bytes([round(acc[0] / acc[3]), round(acc[1] / acc[3]),
                                       round(acc[2] / acc[3]), round(acc[3] / (SS * SS) * 255)])
    return rgba


def png(size, rgba):
    raw = b"".join(b"\0" + bytes(rgba[y * size * 4:(y + 1) * size * 4]) for y in range(size))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def write_ico(name, shapes, holes=()):
    images = [png(s, render(s, shapes, holes)) for s in SIZES]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries = b""
    for s, data in zip(SIZES, images):
        entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    out = RES / name
    out.write_bytes(header + entries + b"".join(images))
    out.with_suffix(".png").write_bytes(images[-1])
    print(f"wrote {out}")


write_ico("gamingzones.ico", GZ_SHAPES)
write_ico("ruler.ico", RULER_SHAPES, RULER_TICKS)
write_ico("gz-editor.ico", EDITOR_SHAPES)
(RES / "gz-editor.rgba").write_bytes(bytes(render(64, EDITOR_SHAPES, ())))
print(f"wrote {RES / 'gz-editor.rgba'}")
