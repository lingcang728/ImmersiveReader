"""Regenerate Windows icons with true 32-bit BMP+AND alpha (no black corners)."""
from __future__ import annotations

import io
import struct
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "icon.png"
ICONS = ROOT / "apps" / "desktop" / "src-tauri" / "icons"
STATIC = ROOT / "apps" / "desktop" / "static"
WEBSITE = ROOT / "apps" / "desktop" / "website"


def resize_rgba(src: Image.Image, size: int) -> Image.Image:
    """Alpha-aware resize via premultiplication to avoid dark fringe bleed."""
    if src.size == (size, size):
        return src.copy()
    rgba = src.convert("RGBA")
    # Premultiply
    r, g, b, a = rgba.split()
    r = Image.composite(r, Image.new("L", rgba.size, 0), a)
    g = Image.composite(g, Image.new("L", rgba.size, 0), a)
    b = Image.composite(b, Image.new("L", rgba.size, 0), a)
    pre = Image.merge("RGBA", (r, g, b, a))
    pre = pre.resize((size, size), Image.Resampling.LANCZOS)
    # Un-premultiply
    pr, pg, pb, pa = pre.split()
    pr_data = list(pr.getdata())
    pg_data = list(pg.getdata())
    pb_data = list(pb.getdata())
    pa_data = list(pa.getdata())
    out_r, out_g, out_b, out_a = [], [], [], []
    for i, alpha in enumerate(pa_data):
        if alpha == 0:
            out_r.append(0)
            out_g.append(0)
            out_b.append(0)
            out_a.append(0)
        else:
            out_r.append(min(255, (pr_data[i] * 255) // alpha))
            out_g.append(min(255, (pg_data[i] * 255) // alpha))
            out_b.append(min(255, (pb_data[i] * 255) // alpha))
            out_a.append(alpha)
    out = Image.new("RGBA", (size, size))
    out.putdata(list(zip(out_r, out_g, out_b, out_a)))
    return out


def rgba_to_bmp_ico_image(im: Image.Image) -> bytes:
    """Encode one 32-bit BMP XOR + 1-bit AND mask for ICO (height doubled in header)."""
    im = im.convert("RGBA")
    w, h = im.size
    # XOR bitmap: BGRA bottom-up, 32bpp
    pixels = list(im.getdata())
    xor = bytearray()
    # rows bottom-up
    for y in range(h - 1, -1, -1):
        row_start = y * w
        for x in range(w):
            r, g, b, a = pixels[row_start + x]
            xor.extend((b, g, r, a))
    # AND mask: 1 bit per pixel, 1 = transparent, padded to 32-bit boundary per row
    row_bytes = ((w + 31) // 32) * 4
    and_mask = bytearray()
    for y in range(h - 1, -1, -1):
        row_start = y * w
        bits = 0
        bit_count = 0
        row = bytearray()
        for x in range(w):
            _r, _g, _b, a = pixels[row_start + x]
            bits = (bits << 1) | (1 if a == 0 else 0)
            bit_count += 1
            if bit_count == 8:
                row.append(bits & 0xFF)
                bits = 0
                bit_count = 0
        if bit_count:
            bits <<= 8 - bit_count
            row.append(bits & 0xFF)
        while len(row) < row_bytes:
            row.append(0)
        and_mask.extend(row)

    # BITMAPINFOHEADER
    header = struct.pack(
        "<IIIHHIIIIII",
        40,  # biSize
        w,
        h * 2,  # doubled height for XOR+AND
        1,  # planes
        32,  # bit count
        0,  # BI_RGB
        len(xor),
        0,
        0,
        0,
        0,
    )
    return header + bytes(xor) + bytes(and_mask)


def write_bmp_ico(path: Path, sizes: list[int], src: Image.Image) -> None:
    images = [resize_rgba(src, s) for s in sizes]
    payloads = [rgba_to_bmp_ico_image(im) for im in images]
    count = len(sizes)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    directory = b""
    for s, payload in zip(sizes, payloads):
        w = 0 if s >= 256 else s
        h = 0 if s >= 256 else s
        # planes=1, bitcount=32
        directory += struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(payload), offset)
        offset += len(payload)
    path.write_bytes(header + directory + b"".join(payloads))
    print(f"wrote {path} ({path.stat().st_size} bytes) sizes={sizes}")


def main() -> None:
    src = Image.open(SRC).convert("RGBA")
    # Ensure corner transparency
    c = src.getpixel((0, 0))
    print("source corner", c)
    if c[3] != 0:
        raise SystemExit("source still has opaque corners; fix transparency first")

    # Standard PNG set for Tauri bundle + favicons
    png_sizes = {
        "32x32.png": 32,
        "64x64.png": 64,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
    }
    for name, size in png_sizes.items():
        im = resize_rgba(src, size)
        im.save(ICONS / name, "PNG")
        print("png", name, im.getpixel((0, 0)))

    # Square logos used by store/appx leftovers
    for name, size in {
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }.items():
        resize_rgba(src, size).save(ICONS / name, "PNG")

    # Critical: BMP+AND multi-size ICO for Windows shell (avoids black corners)
    write_bmp_ico(ICONS / "icon.ico", [16, 24, 32, 48, 64, 128, 256], src)
    write_bmp_ico(ICONS / "tray-icon.ico", [16, 32], src)
    write_bmp_ico(WEBSITE / "favicon.ico", [16, 24, 32, 48], src)

    # Static UI assets
    resize_rgba(src, 32).save(STATIC / "favicon.png", "PNG")
    resize_rgba(src, 48).save(STATIC / "app-icon-48.png", "PNG")
    resize_rgba(src, 192).save(STATIC / "app-icon.png", "PNG")
    resize_rgba(src, 32).save(WEBSITE / "favicon.png", "PNG")

    # icns is mac-only; leave existing or regenerate roughly as PNG stack via pillow if needed
    # Keep icon.icns from prior tauri generation if present — optional skip

    # Verify ICO is BMP-based
    data = (ICONS / "icon.ico").read_bytes()
    _r, _t, count = struct.unpack_from("<HHH", data, 0)
    off = 6
    for i in range(count):
        w, h, _c, _res, planes, bpp, size, offset = struct.unpack_from("<BBBBHHII", data, off)
        ww = 256 if w == 0 else w
        magic = data[offset : offset + 4]
        print(f"ico entry {i}: {ww}px planes={planes} bpp={bpp} header={magic!r} (expect BI size 40)")
        off += 16

    # Round-trip load with Pillow
    ico = Image.open(ICONS / "icon.ico")
    ico = ico.convert("RGBA")
    print("pillow ico corner", ico.getpixel((0, 0)), "size", ico.size)


if __name__ == "__main__":
    main()
