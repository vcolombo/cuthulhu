# SPDX-License-Identifier: GPL-3.0-or-later
"""Build the desktop shell's icon bundle from the vector sources beside this file.

Run from anywhere:

    python3 docs/branding/build-icons.py

Every raster is rendered from SVG at its native pixel size - nothing is ever
downscaled from a bitmap - and each size draws on the artwork suited to it:

    16 - 20 px      C mark, small tier    (widened slit)
    21 - 26 px      C mark, standard tier
    27 px and up    full mascot

The mascot carries a craft knife and eight tentacle curls that stop resolving
below about 27px, so a C mark stands in underneath. The `.ico` and `.icns`
containers are genuinely multi-resolution: each entry holds whichever artwork
suits its own size, and the OS picks.

Requires Pillow and CairoSVG, neither of which the Rust or Node toolchains
pull in. This is why the script is not wired into CI - the committed icons are
the build output, and this regenerates them on demand.

Install from `requirements.txt` rather than loose. The icons are committed as
binary, so an unpinned rasteriser bump produces a diff indistinguishable from
an intentional artwork change.
"""
import io
import os
import struct
import sys

try:
    import cairosvg
    from PIL import Image
except ImportError:
    sys.exit("needs Pillow and CairoSVG: "
             "pip install -r docs/branding/requirements.txt")

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
ICONS = os.path.join(ROOT, "apps", "desktop", "icons")

MASCOT = os.path.join(HERE, "cuthulhu-mascot.svg")
C_STD = os.path.join(HERE, "cuthulhu-c.svg")
C_SMALL = os.path.join(HERE, "cuthulhu-c-16.svg")

CREAM = (250, 236, 214, 255)
_cache = {}


def art(size):
    """Rounded artwork with transparent corners, rendered natively at `size`."""
    if size not in _cache:
        svg = C_SMALL if size <= 20 else C_STD if size <= 26 else MASCOT
        buf = io.BytesIO()
        cairosvg.svg2png(url=svg, write_to=buf, output_width=size, output_height=size)
        _cache[size] = Image.open(io.BytesIO(buf.getvalue())).convert("RGBA")
    return _cache[size]


def art_square(size):
    """Full-bleed variant: the artwork flattened onto an opaque cream square."""
    return Image.alpha_composite(Image.new("RGBA", (size, size), CREAM), art(size))


def save(im, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    im.save(path, "PNG", optimize=True)


# ----------------------------------------------------------------- .ico ----
def _dib(im):
    """32-bit BGRA DIB payload, bottom-up, with an empty AND mask."""
    w, h = im.size
    raw = im.tobytes("raw", "BGRA")
    rows = [raw[y * w * 4:(y + 1) * w * 4] for y in range(h)]
    xor = b"".join(reversed(rows))
    and_mask = b"\x00" * (((w + 31) // 32) * 4 * h)
    hdr = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, len(xor), 0, 0, 0, 0)
    return hdr + xor + and_mask


def write_ico(path, sizes):
    entries, blobs, off = [], [], 6 + 16 * len(sizes)
    for s in sizes:
        im = art_square(s)
        if s >= 256:                      # PNG payload at 256+, DIB below it
            buf = io.BytesIO()
            im.save(buf, "PNG", optimize=True)
            data = buf.getvalue()
        else:
            data = _dib(im)
        entries.append(struct.pack("<BBBBHHII", s & 0xFF, s & 0xFF, 0, 0,
                                   1, 32, len(data), off))
        blobs.append(data)
        off += len(data)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(struct.pack("<HHH", 0, 1, len(sizes)))
        f.write(b"".join(entries))
        f.write(b"".join(blobs))


# ---------------------------------------------------------------- .icns ----
def mac_tile(size):
    """Apple's macOS 11+ app-icon grid: content inset to 824/1024, centred.

    The inset is forced to even parity because an odd margin cannot be split
    evenly either side, which shifts the artwork half a pixel off centre.
    """
    inner = round(size * 824 / 1024)
    if (size - inner) % 2:
        inner -= 1
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    a = art(inner)
    o = (size - inner) // 2
    canvas.paste(a, (o, o), a)
    return canvas


def write_icns(path):
    types = [(b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128),
             (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024), (b"ic11", 32),
             (b"ic12", 64), (b"ic13", 256), (b"ic14", 512)]
    chunks = b""
    for tag, s in types:
        buf = io.BytesIO()
        mac_tile(s).save(buf, "PNG", optimize=True)
        d = buf.getvalue()
        # the chunk length counts its own 8-byte header, not just the payload
        chunks += tag + struct.pack(">I", 8 + len(d)) + d
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(b"icns" + struct.pack(">I", 8 + len(chunks)) + chunks)


# Tauri's default Store tile set. These are full-bleed rather than transparent:
# the cream tile is part of the mark, and a transparent tile would composite
# onto whatever accent colour the user has picked.
STORE_TILES = [(30, "Square30x30Logo"), (44, "Square44x44Logo"),
               (71, "Square71x71Logo"), (89, "Square89x89Logo"),
               (107, "Square107x107Logo"), (142, "Square142x142Logo"),
               (150, "Square150x150Logo"), (284, "Square284x284Logo"),
               (310, "Square310x310Logo"), (50, "StoreLogo")]

# Windows draws the app-list entry from Square44x44Logo, scaling it down as far
# as 16px for the taskbar. Left to itself it would shrink the 44px mascot and
# throw away the whole point of the C mark, so ship explicit target-size assets
# and let the tier rule pick the artwork at each one.
TARGET_SIZES = [16, 24, 32, 48, 256]


def main():
    save(art_square(32), os.path.join(ICONS, "32x32.png"))
    save(art(128), os.path.join(ICONS, "128x128.png"))
    save(art(256), os.path.join(ICONS, "128x128@2x.png"))
    save(art(1024), os.path.join(ICONS, "icon.png"))
    write_ico(os.path.join(ICONS, "icon.ico"),
              [16, 20, 24, 32, 48, 64, 128, 256])
    write_icns(os.path.join(ICONS, "icon.icns"))

    for size, name in STORE_TILES:
        save(art_square(size), os.path.join(ICONS, f"{name}.png"))
    for size in TARGET_SIZES:
        save(art_square(size),
             os.path.join(ICONS, f"Square44x44Logo.targetsize-{size}.png"))

    print("wrote", os.path.relpath(ICONS, ROOT))


if __name__ == "__main__":
    main()
