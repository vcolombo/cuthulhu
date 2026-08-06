# SPDX-License-Identifier: GPL-3.0-or-later
"""Build the desktop shell's icon bundle from the vector sources beside this file.

Run from anywhere:

    python3 docs/branding/build-icons.py

Every raster is rendered from SVG at its native pixel size - nothing is ever
downscaled from a bitmap - and each size draws on the artwork suited to it:

    16 - 20 px      C mark, small tier    (widened slit)
    21 - 30 px      C mark, standard tier
    31 px and up    full mascot

The mascot carries a craft knife and eight tentacle curls. Its eyes and blade
go soft below roughly 31px - visible on the Windows 30px Start tile, which is
what set this boundary - so a C mark stands in underneath. The `.ico` and
`.icns` containers are genuinely multi-resolution: each entry holds whichever
artwork suits its own size, and the OS picks.

Requires Pillow and CairoSVG, neither of which the Rust or Node toolchains
pull in. This is why the script is not wired into CI - the committed icons are
the build output, and this regenerates them on demand.

Install from `requirements.txt` rather than loose. The icons are committed as
binary, so an unpinned rasteriser bump produces a diff indistinguishable from
an intentional artwork change.

Those pins do not make this byte-reproducible. libcairo does the rasterising and
pip cannot pin it, so a host with a different libcairo rewrites most of the
output with no SVG edit. Judge a regeneration by preview-sizes.png, not by the
size of the diff. See "Regeneration is visual, not byte-exact" in README.md.
"""
import io
import os
import shutil
import struct
import subprocess
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
    """The glyph on a transparent background, rendered natively at `size`.

    The sources draw no background tile of their own: macOS 26 composites any
    legacy `.icns` onto its own white squircle plate, so artwork carrying its
    own tile renders tile-in-tile. Surfaces that need an opaque background get
    it from `art_square` instead.
    """
    if size not in _cache:
        svg = C_SMALL if size <= 20 else C_STD if size <= 30 else MASCOT
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
        im = art(s)
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
# No 824/1024 inset here. That grid is for tile artwork on macOS 11-15; this is
# a free glyph, and macOS 26 rescales the whole canvas onto its own squircle
# plate regardless, so an inset would only shrink the artwork twice.
def write_icns(path):
    types = [(b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128),
             (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024), (b"ic11", 32),
             (b"ic12", 64), (b"ic13", 256), (b"ic14", 512)]
    chunks = b""
    for tag, s in types:
        buf = io.BytesIO()
        art(s).save(buf, "PNG", optimize=True)
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
# and let the tier rule pick the artwork at each one. 32 is included because it
# is the first size above the boundary, so it is the mascot's weakest showing.
# These are transparent: Windows plates targetsize assets itself, so an opaque
# square here would tile-in-tile exactly like the macOS case.
TARGET_SIZES = [16, 24, 32, 48, 256]


# ------------------------------------------------- macOS 26 .icon ----
# Tahoe draws legacy .icns on a system grey plate. The Icon Composer bundle in
# AppIcon.icon is the sanctioned way to control that backdrop: cream fill, the
# mascot as a non-glass layer. The layer keeps the old 824/1024 inset because
# the squircle mask crops the canvas corners — a full-bleed glyph would lose
# its tentacle curls to the mask, and here the margin lands on our own cream
# rather than a system plate.
ICON_BUNDLE = os.path.join(HERE, "AppIcon.icon")


def write_icon_layer():
    canvas = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
    a = art(824)
    canvas.paste(a, (100, 100), a)
    save(canvas, os.path.join(ICON_BUNDLE, "Assets", "mascot.png"))


def compile_assets_car():
    """Compile AppIcon.icon into Assets.car beside the other bundle icons.

    actool ships with full Xcode only, so this step is macOS-plus-Xcode; on
    any other host the committed Assets.car simply stays as it is, same as
    every other committed output here.
    """
    if not shutil.which("actool"):
        print("actool not found - skipped Assets.car (needs Xcode on macOS)")
        return
    plist = os.path.join(ICONS, "_actool_partial.plist")
    subprocess.run(
        ["actool", ICON_BUNDLE, "--compile", ICONS,
         "--output-format", "human-readable-text", "--notices", "--warnings",
         "--errors", "--output-partial-info-plist", plist,
         "--app-icon", "AppIcon", "--include-all-app-icons",
         "--enable-on-demand-resources", "NO", "--development-region", "en",
         "--target-device", "mac", "--minimum-deployment-target", "26.0",
         "--platform", "macosx"],
        check=True)
    os.remove(plist)
    # actool also emits its own fallback icns, mascot-only at every size.
    # Ours is better - per-size artwork tiers - so the fallback stays out.
    os.remove(os.path.join(ICONS, "AppIcon.icns"))


def write_preview(path, sizes=(16, 20, 24, 32, 48, 64, 128, 256)):
    """The size ladder the README says to judge regenerations by.

    Two rows, light and dark, because the glyph is transparent and has to be
    checked against both plate colours it will actually sit on.
    """
    pad = 8
    w = sum(s + pad for s in sizes) + pad
    h = max(sizes) + 2 * pad
    sheet = Image.new("RGBA", (w, 2 * h))
    for bg, top in (((255, 255, 255, 255), 0), ((30, 30, 30, 255), h)):
        row = Image.new("RGBA", (w, h), bg)
        x = pad
        for s in sizes:
            a = art(s)
            row.paste(a, (x, (h - s) // 2), a)
            x += s + pad
        sheet.paste(row, (0, top))
    save(sheet, path)


def main():
    save(art(32), os.path.join(ICONS, "32x32.png"))
    save(art(128), os.path.join(ICONS, "128x128.png"))
    save(art(256), os.path.join(ICONS, "128x128@2x.png"))
    save(art(1024), os.path.join(ICONS, "icon.png"))
    write_ico(os.path.join(ICONS, "icon.ico"),
              [16, 20, 24, 32, 48, 64, 128, 256])
    write_icns(os.path.join(ICONS, "icon.icns"))

    for size, name in STORE_TILES:
        save(art_square(size), os.path.join(ICONS, f"{name}.png"))
    for size in TARGET_SIZES:
        save(art(size),
             os.path.join(ICONS, f"Square44x44Logo.targetsize-{size}.png"))

    write_icon_layer()
    compile_assets_car()
    write_preview(os.path.join(HERE, "preview-sizes.png"))

    print("wrote", os.path.relpath(ICONS, ROOT))


if __name__ == "__main__":
    main()
