#!/usr/bin/env python3
"""
Structural golden check for the headless --dump render.

Does NOT do exact pixel comparison (CoreText antialiasing and shell-prompt
content vary across macOS versions and machines). Instead asserts:
  - dimensions are 1600x1000
  - dominant pixel color is the Mineral dark background #0b0d0e (>= 90%)
  - title bar region contains the bar color #161a1c

Run: python3 tools/check-render.py /tmp/anvil-ci.png
"""

import sys
import struct
import zlib
from collections import Counter


def decode_png(path):
    with open(path, "rb") as f:
        data = f.read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    pos = 8
    idat = []
    width = height = bpp = 0
    while pos < len(data):
        length = struct.unpack(">I", data[pos : pos + 4])[0]
        kind = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            width, height = struct.unpack(">II", chunk[:8])
            color_type = chunk[9]
            bpp = 3 if color_type == 2 else 4
        if kind == b"IDAT":
            idat.append(chunk)
        pos += 12 + length
    raw = zlib.decompress(b"".join(idat))
    stride = 1 + width * bpp
    rows = []
    prev = None
    for r in range(height):
        filt = raw[r * stride]
        row_in = list(raw[r * stride + 1 : (r + 1) * stride])
        row = bytearray(len(row_in))
        for i in range(len(row_in)):
            a = row[i - bpp] if i >= bpp else 0
            b = prev[i] if prev else 0
            raw_b = row_in[i]
            if filt == 0:
                row[i] = raw_b
            elif filt == 1:
                row[i] = (raw_b + a) & 0xFF
            elif filt == 2:
                row[i] = (raw_b + b) & 0xFF
            elif filt == 3:
                row[i] = (raw_b + (a + b) // 2) & 0xFF
            elif filt == 4:
                c = prev[i - bpp] if (prev and i >= bpp) else 0
                pa = abs(b - c)
                pb = abs(a - c)
                pc = abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                row[i] = (raw_b + pr) & 0xFF
        rows.append(row)
        prev = row
    return width, height, bpp, rows


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/anvil-ci.png"
    width, height, bpp, rows = decode_png(path)

    # 1. Dimensions
    assert width == 1600 and height == 1000, f"unexpected dimensions {width}x{height}"

    # 2. Dominant background color = Mineral dark bg #0b0d0e
    total = width * height
    counts = Counter()
    for row in rows:
        for i in range(0, len(row), bpp):
            counts[tuple(row[i : i + bpp])] += 1
    bg = (0x0B, 0x0D, 0x0E)
    bg_frac = counts[bg] / total
    assert bg_frac >= 0.90, f"background #{bg[0]:02x}{bg[1]:02x}{bg[2]:02x} covers only {bg_frac:.1%} (expected >= 90%)"

    # 3. Title bar strip (rows 0-79, 2x scale) contains bar color #161a1c
    bar = (0x16, 0x1A, 0x1C)
    bar_counts = Counter()
    for row in rows[:80]:
        for i in range(0, len(row), bpp):
            bar_counts[tuple(row[i : i + bpp])] += 1
    bar_frac = bar_counts[bar] / (80 * width)
    assert bar_frac >= 0.30, f"title bar color #{bar[0]:02x}{bar[1]:02x}{bar[2]:02x} covers only {bar_frac:.1%} of bar rows (expected >= 30%)"

    print(f"ok: {width}x{height}, bg={bg_frac:.1%}, bar={bar_frac:.1%}")


if __name__ == "__main__":
    main()
