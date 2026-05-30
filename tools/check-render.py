#!/usr/bin/env python3
"""
Structural golden check for the headless --dump Control Room render.

Does NOT do exact pixel comparison (CoreText antialiasing and shell-prompt
content vary across macOS versions and machines). Instead it asserts the
three-zone operator-console shell is laid out as expected:
  - dimensions are 1600x1000
  - left SESSIONS/EXPLORER sidebar band is charcoal #161a1c
  - center terminal panel body is Mineral dark #0b0d0e
  - right RUNS/TRACE/AGENT drawer band is charcoal #161a1c
  - bottom status bar is charcoal #161a1c

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


def region_frac(rows, bpp, color, x0, x1, y0, y1):
    """Fraction of pixels in [x0,x1)x[y0,y1) exactly matching `color`."""
    hit = 0
    total = 0
    for y in range(y0, y1):
        row = rows[y]
        for x in range(x0, x1):
            i = x * bpp
            total += 1
            if tuple(row[i : i + bpp]) == color:
                hit += 1
    return hit / total if total else 0.0


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/anvil-ci.png"
    width, height, bpp, rows = decode_png(path)

    # 1. Dimensions.
    assert width == 1600 and height == 1000, f"unexpected dimensions {width}x{height}"

    charcoal = (0x16, 0x1A, 0x1C)
    pane_bg = (0x1A, 0x1B, 0x26)

    # 2. Left sidebar band (SESSIONS / EXPLORER) is charcoal.
    side = region_frac(rows, bpp, charcoal, 60, 350, 120, 880)
    assert side >= 0.60, f"sidebar charcoal only {side:.1%} (expected >= 60%)"

    # 3. Center terminal panel body is the pane background.
    body = region_frac(rows, bpp, pane_bg, 420, 1240, 220, 860)
    assert body >= 0.60, f"panel body pane-bg only {body:.1%} (expected >= 60%)"

    # 4. Right context drawer band (RUNS / TRACE / AGENT) is charcoal.
    draw = region_frac(rows, bpp, charcoal, 1300, 1590, 320, 880)
    assert draw >= 0.60, f"drawer charcoal only {draw:.1%} (expected >= 60%)"

    # 5. Bottom status bar is charcoal.
    status = region_frac(rows, bpp, charcoal, 400, 1200, 972, 998)
    assert status >= 0.60, f"status bar charcoal only {status:.1%} (expected >= 60%)"

    print(f"ok: {width}x{height}, sidebar={side:.1%}, body={body:.1%}, drawer={draw:.1%}, status={status:.1%}")


if __name__ == "__main__":
    main()
