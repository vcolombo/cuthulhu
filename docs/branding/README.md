<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Branding

Vector sources for the application icon, and the script that renders the
desktop shell's icon bundle from them.

```
cuthulhu-mascot.svg    vector master - kraken gripping a craft knife
cuthulhu-c.svg         C mark, standard tier (21-30px)
cuthulhu-c-16.svg      C mark, small tier (16-20px)
AppIcon.icon/          Icon Composer bundle for macOS 26+ (layer PNG written by the script)
build-icons.py         renders apps/desktop/icons/ from the three SVGs above
requirements.txt       pinned rasteriser versions for that script
preview-sizes.png      the full size ladder on light and dark, written by the script
social-preview.png     GitHub social preview card, written by the script; uploaded
                       manually in repo settings (GitHub has no API for it)
```

## Palette

| Role                | Hex       |
| ------------------- | --------- |
| Body teal (mascot)  | `#1E8A8C` |
| Teal (C mark)       | `#17767A` |
| Outline / shadow    | `#1A4F55` |
| Store-tile cream    | `#FAECD6` |
| Blade silver        | `#C9CDD2` |
| Dark outline        | `#14343A` |
| Eyes                | `#0E2E34` |

The artwork is a glyph on a transparent background — the sources draw no tile
of their own. macOS 26 composites any legacy `.icns` onto its own white
squircle plate, and Windows plates `targetsize` taskbar assets the same way,
so artwork carrying its own background renders tile-in-tile on both. Cream
survives only where the OS does *not* provide a plate: the Windows Store
tiles (see below). Because the glyph is transparent, judge every change on
both a light and a dark plate — `preview-sizes.png` shows both rows.

## Two artworks, one icon

The mascot has eight tentacle curls, two eyes and a craft knife. None of that
survives being shrunk to a favicon, so a **C mark** stands in below 31px. It
fuses the monogram with an eye: a ring cut open by a single straight blade
stroke forms the lids, its elliptical counter the sclera, and a vertical slit
the pupil.

| Rendered size | Artwork               |
| ------------- | --------------------- |
| 16 - 20       | `cuthulhu-c-16.svg`   |
| 21 - 30       | `cuthulhu-c.svg`      |
| 31 and up     | `cuthulhu-mascot.svg` |

`icon.ico` and `icon.icns` are genuinely multi-resolution — each entry holds
whichever artwork suits its own size, so the OS picks the right one rather
than scaling one bitmap.

The boundary sits at 31 because that is where the mascot's eyes and blade stop
reading. Rendered onto the Windows Store tiles it was clear that everything from
roughly 30 to 50px shows a recognisable creature with a soft grey smear for a
knife and no legible eyes; eyes only return cleanly around 71px. Thirty was the
last size the OS actually asks for in that band, so the boundary went above it.

### macOS 26: the .icon bundle

macOS 26 draws legacy `.icns` icons scaled down on a system grey squircle
plate. The way to control that backdrop is Apple's Icon Composer format:
`AppIcon.icon/` holds `icon.json` (solid cream fill, the mascot as a single
non-glass layer) and a script-rendered layer PNG. `build-icons.py` compiles it
into `apps/desktop/icons/Assets.car` with `actool` when Xcode is present, and
skips with a note when it is not — the committed `Assets.car` is the build
output, like everything else here. Tauri carries it into `Contents/Resources`
via `bundle.macOS.files`, and `apps/desktop/Info.plist` adds
`CFBundleIconName` so macOS 26+ finds it; older macOS keeps using
`CFBundleIconFile` and `icon.icns`. actool's auto-generated fallback icns is
discarded — ours carries the per-size artwork tiers, its version is
mascot-at-every-size.

The layer PNG keeps an 824/1024 inset: the squircle mask crops the canvas
corners, so a full-bleed glyph would lose its tentacle curls — and inside the
`.icon` the margin lands on our own cream rather than a system plate.

The `.icns` entries carry no 824/1024 inset. That grid is for tile artwork on
macOS 11-15; this is a free glyph, and macOS 26 rescales the whole canvas onto
its own squircle plate regardless, so an inset would only shrink the artwork
twice. The mascot's own viewBox is cropped to the glyph with a 1% margin for
the same reason — dead canvas is dead size on every platform.

## Two C tiers, differing in one value

Both C SVGs are the same geometry on the same 96-unit grid. The only difference
is the pupil slit: 8 units wide normally, 12 units below 21px.

That is not a style choice. Eight units is 1.3 physical pixels at 16px, which
antialiases away to nothing — the pupil disappears entirely at 16 and 20px, the
exact two sizes the mark exists to serve. Twelve units resolves as a distinct
2px slit with 3px of clearance from the surrounding lid.

The general rule for editing these: **any feature must clear roughly 2 physical
pixels at 16px, so nothing thinner than about 12 units on the 96-unit grid.**
That single constraint broke every earlier revision of this mark.

The 96-unit grid divides evenly by 6, 4, 3 and 2, so 16, 24, 32 and 48px
rasters all land on whole pixels.

## Do not use an SVG mask here

The cut ring is an explicit computed path. An earlier version expressed the same
shape as a full annulus behind an SVG `<mask>`, which is the obvious way to draw
it and renders correctly in a browser.

CairoSVG rasterises masks at a resolution tied to the output size. The masked
version measured 26% teal coverage at 96px and **0% at 16px** — a blank tile,
silently, in the one size that matters most. The explicit path measures 16.4% at
16px against 19.0% at 96px, which is ordinary antialiasing.

A consequence worth knowing if you retune the shape: a straight cut only severs
the ring when it passes closer to the centre than the inner radius, so a thick
ring forces a wide aperture. The two cannot be tuned independently. Thinning the
ring to close the aperture puts the stroke under 2.5px at 16px, where it turns
to mush.

## Regenerating

```sh
pip install -r docs/branding/requirements.txt
python3 docs/branding/build-icons.py
```

This rewrites `apps/desktop/icons/`. The outputs are committed, and the script
is deliberately not wired into CI: it needs a Python imaging stack that neither
the Rust nor the Node toolchain pulls in.

Use the pinned requirements rather than installing loose. Because the icons are
committed as binary, an unpinned rasteriser bump produces a diff that looks
exactly like an intentional artwork change.

### Regeneration is visual, not byte-exact

The pins do not make this reproducible, and it is worth being blunt about that
rather than leaving it as a footnote. Running the script on a host with a
different system libcairo rewrites most of `apps/desktop/icons/` without any SVG
edit: byte-different across the set, and pixel-different on the larger mascot
renders, where there is enough curve detail for antialiasing to disagree.

The cause is structural. CairoSVG is a thin layer over the system libcairo via
cairocffi, and libcairo does the rasterising. pip can pin CairoSVG; it cannot
pin the library actually drawing the pixels. What the pins buy is narrower than
reproducibility: they remove the Python layer as a variable, so when output does
shift there is exactly one place it can have come from.

The committed assets were produced in this environment:

| | |
| --- | --- |
| Platform | macOS 26.6 (arm64) |
| libcairo | 1.18.4 (Homebrew) |
| Python | 3.14.6 |
| cairocffi | 1.7.1 |
| CairoSVG | 2.9.0 |
| Pillow | 12.3.0 |

So: do not read an empty diff as confirmation, and do not read a large diff as
breakage. **Review `preview-sizes.png` and the 16px tile, and judge the artwork.**
If you need regeneration to be byte-reproducible — for a release process that
asserts the committed icons match their sources — pin libcairo too, by running
the script in a container built from a fixed base image. That is deliberately
not done here: it is a lot of machinery for an icon set that changes rarely and
is verified by eye anyway.

Edit the SVGs, never the PNGs, and re-check the result at 16px magnified before
committing — small-size artwork cannot be judged at 100%.

## Windows Store tiles

`apps/desktop/icons/` also carries the MSIX tile set — `Square30x30Logo.png`
through `Square310x310Logo.png`, plus `StoreLogo.png`.

These are **full-bleed and opaque**, not transparent. The cream tile is part of
the mark; a transparent tile would composite onto whatever accent colour the
user has chosen and lose it.

Windows draws the app-list and taskbar entry from `Square44x44Logo`, scaling it
down as far as 16px. Left to itself it would shrink the 44px mascot and discard
the whole point of the C mark, so explicit target-size assets ship alongside it:

```
Square44x44Logo.targetsize-16.png    C mark, small tier
Square44x44Logo.targetsize-24.png    C mark, standard tier
Square44x44Logo.targetsize-32.png    mascot
Square44x44Logo.targetsize-48.png    mascot
Square44x44Logo.targetsize-256.png   mascot
```

Unlike the Store tiles, the target-size assets are transparent: Windows draws
its own backplate behind them, so an opaque square here would render
tile-in-tile just like a legacy `.icns` does on macOS 26.

Nothing in `tauri.conf.json` references these yet — Tauri has no MSIX bundle
target, so they sit ready rather than wired up. They are generated by the same
script and the same tier rule as everything else, so they cannot drift out of
sync with the app icon.

`Square30x30Logo` is what set the 31px boundary: at 30px the mascot's eyes and
knife were too soft to read, so that tile now carries the C mark.

That leaves a deliberate 30-to-32 seam, where a two-pixel step swaps the artwork
entirely. Both sides are the best available rendering at their own size, and no
Windows surface shows them side by side. `targetsize-32` is kept in the set
precisely because it is the mascot's weakest showing — if it ever looks wrong in
practice, that is the file to look at first.
