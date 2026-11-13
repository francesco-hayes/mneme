"""Draws the placeholder Mneme app icon (1024x1024 RGBA PNG) with no third-party modules.

A rounded square in the macOS icon grid (824px body inside a 1024px canvas)
with five white waveform bars. Anti-aliased by 3x3 supersampling.
"""
import struct, sys, zlib

SIZE = 1024
SS = 3
BODY = 824
MARGIN = (SIZE - BODY) / 2
RADIUS = 185.0
TOP = (88, 86, 214)      # indigo
BOTTOM = (48, 44, 140)   # darker indigo
BAR_W = 64.0
BAR_GAP = 52.0
BAR_HEIGHTS = [200.0, 380.0, 520.0, 300.0, 160.0]


def in_round_rect(x, y, x0, y0, x1, y1, r):
    if x < x0 or x > x1 or y < y0 or y > y1:
        return False
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    dx, dy = x - cx, y - cy
    return dx * dx + dy * dy <= r * r


total_w = len(BAR_HEIGHTS) * BAR_W + (len(BAR_HEIGHTS) - 1) * BAR_GAP
bar_x0 = (SIZE - total_w) / 2
bars = []
for i, h in enumerate(BAR_HEIGHTS):
    x0 = bar_x0 + i * (BAR_W + BAR_GAP)
    bars.append((x0, SIZE / 2 - h / 2, x0 + BAR_W, SIZE / 2 + h / 2, BAR_W / 2))

rows = []
for py in range(SIZE):
    row = bytearray([0])
    t = min(max((py - MARGIN) / BODY, 0.0), 1.0)
    bg = [TOP[c] + (BOTTOM[c] - TOP[c]) * t for c in range(3)]
    for px in range(SIZE):
        body = 0
        bar = 0
        for sy in range(SS):
            y = py + (sy + 0.5) / SS
            for sx in range(SS):
                x = px + (sx + 0.5) / SS
                if in_round_rect(x, y, MARGIN, MARGIN, SIZE - MARGIN, SIZE - MARGIN, RADIUS):
                    body += 1
                    if any(in_round_rect(x, y, *b) for b in bars):
                        bar += 1
        n = SS * SS
        if body == 0:
            row += b"\x00\x00\x00\x00"
            continue
        f = bar / body
        rgb = [round(bg[c] * (1 - f) + 255 * f) for c in range(3)]
        row += bytes(rgb + [round(255 * body / n)])
    rows.append(bytes(row))


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


png = b"\x89PNG\r\n\x1a\n"
png += chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(b"".join(rows), 9))
png += chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
