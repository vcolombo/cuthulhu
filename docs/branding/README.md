<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Branding

Vector sources for the application icon, and the script that renders the
desktop shell's icon bundle from them.

```
cuthulhu-mascot.svg    vector master - kraken gripping a craft knife
cuthulhu-c.svg         C mark, standard tier (21-26px)
cuthulhu-c-16.svg      C mark, small tier (16-20px)
build-icons.py         renders apps/desktop/icons/ from the three SVGs above
requirements.txt       pinned rasteriser versions for that script
preview-sizes.png      the full size ladder, for eyeballing changes
```

## Palette

| Role                | Hex       |
| ------------------- | --------- |
| Body teal (mascot)  | `#1E8A8C` |
| Teal (C mark)       | `#17767A` |
| Outline / shadow    | `#1A4F55` |
| Background cream    | `#FAECD6` |
| Blade silver        | `#C9CDD2` |
| Dark outline        | `#14343A` |
| Eyes                | `#0E2E34` |

Corner radius on the rounded tile is 17.5% of the icon width.

## Two artworks, one icon

The mascot has eight tentacle curls, two eyes and a craft knife. None of that
survives being shrunk to a favicon, so a **C mark** stands in below 27px. It
fuses the monogram with an eye: a ring cut open by a single straight blade
stroke forms the lids, its elliptical counter the sclera, and a vertical slit
the pupil.

| Rendered size | Artwork               |
| ------------- | --------------------- |
| 16 - 20       | `cuthulhu-c-16.svg`   |
| 21 - 26       | `cuthulhu-c.svg`      |
| 27 and up     | `cuthulhu-mascot.svg` |

`icon.ico` and `icon.icns` are genuinely multi-resolution — each entry holds
whichever artwork suits its own size, so the OS picks the right one rather
than scaling one bitmap.

The threshold sits at 27 rather than 32 because of the macOS grid. Apple insets
app-icon content to 824/1024 of its canvas, so the 32px `.icns` tile actually
draws 26px of artwork; a threshold of 32 would have put a squeezed mascot in it.

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
exactly like an intentional artwork change. The committed set was produced with
Pillow 12.3.0 and CairoSVG 2.9.0 against cairocffi 1.7.1. The pin only covers
the Python layer — CairoSVG draws through the system libcairo, so a different
libcairo can still shift antialiasing by a pixel.

Edit the SVGs, never the PNGs, and re-check the result at 16px magnified before
committing — small-size artwork cannot be judged at 100%.

If a Windows Store target is ever added, `tauri icon` can generate the
`Square*Logo.png` set from `apps/desktop/icons/icon.png`.
