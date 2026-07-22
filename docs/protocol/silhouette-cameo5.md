# Silhouette Cameo 5 (Alpha) — command set

Ported from the GPL driver `inkscape-silhouette`, file `silhouette/Graphtec.py`
(GPL-2.0-or-later). Cite lines as `[src: Graphtec.py L### (GPL-2.0+)]`. Silhouette
speaks a **GPGL-style** ASCII protocol (Graphtec Plotter Graphics Language), not
HPGL — different command letters and units from the GCC Puma. See `README.md`
for provenance rules. Line numbers reference `fablabnbg/inkscape-silhouette`
`main` as fetched 2026-07-22; re-verify if upstream shifts them.

The Cameo 5 shares the **Cameo 4 protocol family**; the driver groups all Cameo
5 variants with the Cameo 4 command dialect. `[src: Graphtec.py L151-163 (GPL-2.0+)]`

## Device identity (USB)

| Field | Value | Source |
|-------|-------|--------|
| Vendor ID (Cameo 5 **Alpha**) | `0x3844` | `[src: Graphtec.py L128 (GPL-2.0+)]` |
| Product ID — Cameo 5 Alpha | `0x0001` | `[src: Graphtec.py L142 (GPL-2.0+)]` |
| Product ID — Cameo 5 Alpha Plus | `0x0002` | `[src: Graphtec.py L143 (GPL-2.0+)]` |
| Bulk OUT endpoint (host→device) | `0x01` | `[src: Graphtec.py L577 (GPL-2.0+)]` |
| Bulk IN endpoint (device→host) | `0x82` | `[src: Graphtec.py L676 (GPL-2.0+)]` |
| Interface | `0`, bulk transfers | `[src: Graphtec.py L592-601 (GPL-2.0+)]` |

**Note the split vendor ID.** The Cameo 5 *Alpha* enumerates under vendor
`0x3844` (PID `0x0001`/`0x0002`), **not** the classic Graphtec vendor `0x0b4d`
used by earlier Cameo/Portrait models and even the non-Alpha Cameo 5 (`0x1140`).
`[src: Graphtec.py L127-141 (GPL-2.0+)]` Match the Alpha by `0x3844:0x0001`.

## Coordinate system & units

- **20 device units per millimetre** (`_mm_2_SU`: `int(round(mm * 20.0))`).
  `[src: Graphtec.py L296 (GPL-2.0+)]` So 1 unit = 0.05 mm; **508 units/inch**
  (`_inch_2_SU`, `inch * 508`) `[src: Graphtec.py L311 (GPL-2.0+)]`. This is the
  stated device resolution of 1/20 mm. `[src: Graphtec.py L915 (GPL-2.0+)]`
  (Contrast: the GCC Puma HPGL grid is 40 units/mm — do not mix them.)
- **Coordinates are `(y, x)`.** Move/draw encoders take `(mmy, mmx)` and emit
  `<letter><y_units>,<x_units>`. `[src: Graphtec.py L1160-1166 (GPL-2.0+)]`
  Origin is the media's loaded corner; +y feeds media through, +x across.

## Framing

- Every command is ASCII and **terminated by ETX (`0x03`)**. The driver joins
  commands as `command + \x03`. `[src: Graphtec.py L168, L325 (GPL-2.0+)]`
- Single-byte control operations are sent as **ESC (`0x1b`) + code**.
  `[src: Graphtec.py L170, L672 (GPL-2.0+)]`

## Control / status codes (ESC-prefixed)

| Bytes | Meaning | Source |
|-------|---------|--------|
| `1b 04` (ESC EOT) | Initialize / reset device | `[src: Graphtec.py L174, L784 (GPL-2.0+)]` |
| `1b 05` (ESC ENQ) | Query status | `[src: Graphtec.py L176, L729 (GPL-2.0+)]` |
| `1b 15` (ESC NAK) | Query tool setup | `[src: Graphtec.py L178 (GPL-2.0+)]` |

Status replies (single char + ETX): `0` = ready, `1` = moving, `2` = unloaded
(no media). `[src: Graphtec.py L184-189 (GPL-2.0+)]`

Firmware version query: send `FG` (ETX-terminated), read the version string back.
`[src: Graphtec.py L181, L843 (GPL-2.0+)]`

## Command reference (GPGL)

All ETX-terminated. `SU` = device units (20/mm). Tool = holder number (1 or 2;
Cameo 4/5 have two holders).

| Command | Form | Meaning | Source |
|---------|------|---------|--------|
| Move (pen up) | `M<y>,<x>` | Rapid move to (y,x) in SU | `[src: Graphtec.py L1162 (GPL-2.0+)]` |
| Draw (pen down) | `D<y>,<x>` | Cut/draw line to (y,x) in SU | `[src: Graphtec.py L1166 (GPL-2.0+)]` |
| Upper-left bound | `\<y>,<x>` | Set clip box top-left | `[src: Graphtec.py L1170, L847 (GPL-2.0+)]` |
| Lower-right bound | `Z<y>,<x>` | Set clip box bottom-right | `[src: Graphtec.py L1174, L847 (GPL-2.0+)]` |
| Select tool | `J<tool>` | Choose tool holder | `[src: Graphtec.py L337 (GPL-2.0+)]` |
| Speed | `!<speed>,<tool>` | Cut speed | `[src: Graphtec.py L345 (GPL-2.0+)]` |
| Pressure | `FX<pressure>,<tool>` | Cut force | `[src: Graphtec.py L341 (GPL-2.0+)]` |
| Acceleration | `TJ<accel>` | Carriage acceleration | `[src: Graphtec.py L1158 (GPL-2.0+)]` |
| Blade offset | `FC<x_su>,<y_su>,<tool>` | Cutter offset (from mm) | `[src: Graphtec.py L353 (GPL-2.0+)]` |
| Depth | `TF<depth>,<tool>` | Auto-blade depth (Cameo 4/5) | `[src: Graphtec.py L349 (GPL-2.0+)]` |
| Lift | `FE1,<tool>` / `FE0,<tool>` | Lift blade on corners on/off | `[src: Graphtec.py L358-360 (GPL-2.0+)]` |
| Sharpen corners | `FF<start>,<end>,<tool>` | Overcut corners | `[src: Graphtec.py L363-365 (GPL-2.0+)]` |
| Media | `FW<media>` | Select media preset | `[src: Graphtec.py L946 (GPL-2.0+)]` |
| Cutting mat | `TG<n>` | Select mat (Cameo 3+) | `[src: Graphtec.py L863-866 (GPL-2.0+)]` |
| Orientation | `TB50,0` / `TB50,1` | Portrait / landscape | `[src: Graphtec.py L871 (GPL-2.0+)]` |
| Set origin / feed | `SO0`, `FN0` | End-of-job home + feed out | `[src: Graphtec.py L1476-1478, L1482-1488 (GPL-2.0+)]` |

## Minimal cut job sequence

Order the driver emits for a plain cut (no registration): `[src: Graphtec.py L784, L845-882, L946-1122, L1219-1290, L1476-1488 (GPL-2.0+)]`

1. **Init** — `ESC EOT` (`1b 04`); read firmware via `FG`.
2. **Cameo 3+ probes** — `TB71` (regmark-sensor cal), `FA` (carriage/roller cal). `[src: Graphtec.py L821-832 (GPL-2.0+)]`
3. **Mat / orientation / bounds** — `TG<mat>`, `FN0`, `TB50,0`, then `\0,0` + `Z<bottom>,<right>`.
4. **Media** — `FW<media>`.
5. **Tool params** — `J<tool>`, `FX<pressure>,<tool>`, `!<speed>,<tool>`, `TJ<accel>`, `FC<off>,<off>,<tool>`, `FE0,<tool>`.
6. **Geometry** — `M<y>,<x>` to start each subpath (pen up), `D<y>,<x>` for each following point (pen down).
7. **Finish** — `M<feed>,0`, `SO0`, `FN0` (feed media out, set new origin).

### Worked example — 20 mm square

20 mm = **400 SU**. Square path in (x,y) mm: (0,0)→(20,0)→(20,20)→(0,20)→(0,0).
Encoded `(y,x)` in SU, ETX shown as `·`:

```
M0,0·      move (pen up) to origin
D0,400·    cut to x=20,y=0
D400,400·  cut to x=20,y=20
D400,0·    cut to x=0,y=20
D0,0·      cut back to origin
```

This is the geometry core; a real job wraps it in the init/tool/finish steps
above. This byte stream is the correct-dialect replacement for the placeholder
`tools/capture/samples/cameo5-square.SYNTHETIC.hex`.

## Cameo 5 Alpha specifics

- Distinct USB vendor `0x3844` (see Device identity).
- **Quad registration marks**: the Alpha uses `TB124,<h>,<w>,<t>,<l>` for the
  automatic four-mark regmark search, versus `TB123,...` (dual marks) on older
  models. `[src: Graphtec.py L1176-1183 (GPL-2.0+)]` Relevant to Print & Cut
  (sub-project 6), not basic cutting.

## Registration marks (Print & Cut — reference for sub-project 6)

Framing before a regmark search: `TB50,0`, `TB99`, `TB52,2` (mark type:
Cameo/Portrait), `TB51,400` (mark length), `TB53,10` (mark width), `TB55,1`;
then `TB123`/`TB124` search. `[src: Graphtec.py L1371-1387 (GPL-2.0+)]` Success
reply is `    0\x03`; timeout/other means marks not found.
`[src: Graphtec.py L1393-1395 (GPL-2.0+)]`

## To validate on hardware (Cameo 5 Alpha)

- Confirm `0x3844:0x0001` enumeration on the physical Alpha (`lsusb` / capture).
- Confirm the cut of the 20 mm square above lands at 20 mm (verifies the 20 SU/mm
  scale and the `(y,x)` order end-to-end).
- Confirm the two-tool holder numbering (`J1`/`J2`) matches physical holders.
- Confirm `TB124` quad-regmark behaviour when we reach Print & Cut.

USB captures (`tools/capture/`) now serve to **validate** this ported protocol,
not to originate it.
