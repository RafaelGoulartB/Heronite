#!/usr/bin/env python3
"""Makes packaging/windows/heronite.ico (the program's icon on Windows) from
packaging/linux/app.heronite.Heronite.png. Run it again when the PNG changes and
commit the .ico; builds do not run it.

Sizes 16, 24, 32, 48, 64, 128 and 256: each one resized from the 256 px PNG
with Lanczos (Pillow resizes with premultiplied alpha, so the rounded corners
get no dark fringe). 256 is stored as PNG, the others as 32-bit bitmaps,
which every part of Windows reads.

    usage: python scripts/windows/make-icon.py      (needs Pillow)
"""

import io
import struct
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "packaging" / "linux" / "app.heronite.Heronite.png"
TARGET = ROOT / "packaging" / "windows" / "heronite.ico"
SIZES = [16, 24, 32, 48, 64, 128, 256]


def bitmap(img: Image.Image) -> bytes:
    """A 32-bit icon bitmap: header, BGRA rows bottom-up, then the 1-bit AND
    mask (set where the pixel is fully transparent)."""
    w, h = img.size
    header = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    px = img.load()
    color = bytearray()
    for y in reversed(range(h)):
        for x in range(w):
            r, g, b, a = px[x, y]
            color += bytes((b, g, r, a))
    stride = ((w + 31) // 32) * 4
    mask = bytearray()
    for y in reversed(range(h)):
        row = bytearray(stride)
        for x in range(w):
            if px[x, y][3] == 0:
                row[x // 8] |= 0x80 >> (x % 8)
        mask += row
    return header + bytes(color) + bytes(mask)


def main() -> int:
    src = Image.open(SOURCE).convert("RGBA")
    if src.size != (256, 256):
        print(f"{SOURCE} is {src.size}, expected 256x256", file=sys.stderr)
        return 1
    images = []
    for size in SIZES:
        img = src if size == 256 else src.resize((size, size), Image.Resampling.LANCZOS)
        if size == 256:
            buf = io.BytesIO()
            img.save(buf, format="PNG", optimize=True)
            images.append((size, buf.getvalue()))
        else:
            images.append((size, bitmap(img)))

    out = bytearray(struct.pack("<HHH", 0, 1, len(images)))
    offset = 6 + 16 * len(images)
    for size, data in images:
        dim = 0 if size == 256 else size
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    for _, data in images:
        out += data
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_bytes(bytes(out))
    print(f"wrote {TARGET} ({len(out)} bytes, sizes {SIZES})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
