"""Draws the placeholder menu bar icons (36x36 RGBA PNG, shown at 18pt) with no third-party modules.

Both are macOS template images: black with an alpha channel, so the system
tints them for light and dark menu bars. They differ in shape, not color:

  tray-idle.png       five waveform bars, as on the app icon
  tray-recording.png  a solid disc inside a ring, the usual "record" mark

Run from this folder: python3 make_tray_icons.py
"""
import math, os, struct, zlib

SIZE = 36
SS = 4  # supersamples per axis
HERE = os.path.dirname(os.path.abspath(__file__))


def in_round_rect(x, y, x0, y0, x1, y1, r):
    if x < x0 or x > x1 or y < y0 or y > y1:
        return False
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    dx, dy = x - cx, y - cy
    return dx * dx + dy * dy <= r * r


BAR_W = 4.0
BAR_GAP = 3.0
BAR_HEIGHTS = [10.0, 20.0, 28.0, 16.0, 8.0]
_total = len(BAR_HEIGHTS) * BAR_W + (len(BAR_HEIGHTS) - 1) * BAR_GAP
BARS = []
for i, h in enumerate(BAR_HEIGHTS):
    x0 = (SIZE - _total) / 2 + i * (BAR_W + BAR_GAP)
    BARS.append((x0, SIZE / 2 - h / 2, x0 + BAR_W, SIZE / 2 + h / 2, BAR_W / 2))


def idle(x, y):
    return any(in_round_rect(x, y, *bar) for bar in BARS)


def recording(x, y):
    d = math.hypot(x - SIZE / 2, y - SIZE / 2)
    return d <= 9.0 or 13.0 <= d <= 16.0


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


def write(name, inside):
    rows = []
    for py in range(SIZE):
        row = bytearray([0])
        for px in range(SIZE):
            hits = sum(
                inside(px + (sx + 0.5) / SS, py + (sy + 0.5) / SS)
                for sy in range(SS)
                for sx in range(SS)
            )
            row += bytes([0, 0, 0, round(255 * hits / (SS * SS))])
        rows.append(bytes(row))
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(b"".join(rows), 9))
    png += chunk(b"IEND", b"")
    with open(os.path.join(HERE, name), "wb") as f:
        f.write(png)


write("tray-idle.png", idle)
write("tray-recording.png", recording)
