# Supported machines — roadmap

Target: **every non-Cricut cutter, in one app.** Two driver backends cover the
field — **GPGL** (Silhouette/Graphtec family) and **HPGL** (almost everyone
else) — plus a per-model profile table (cut width, speed/force range, serial
params). That takes us from 2 machines to hundreds without a new protocol per
brand.

Cricut is out of scope (closed, cloud-tethered platform).

## Tier 1 — near-free: same GPGL driver we already port

The GPL driver we port for the Cameo 5 (`inkscape-silhouette`) already handles a
whole family with the **same GPGL protocol** — adding them is a device-table
entry (USB VID/PID + width), not new protocol work.

| Model(s) | Notes |
|----------|-------|
| Silhouette Portrait 1–4 | GPGL |
| Silhouette Cameo 1–5 (+ Plus / Pro / Alpha / Alpha Plus) | GPGL; Cameo 5 Alpha is our reference machine |
| Silhouette Cameo Pro MK-II | GPGL, 24" |
| Silhouette SD 1/2 | older GPGL |
| Graphtec Craft Robo CC200-20 / CC300-20 | GPGL |

**Effort: a data table.** Source: `inkscape-silhouette` `silhouette/Graphtec.py`
DEVICE list; see `docs/protocol/silhouette-cameo5.md`.

## Tier 2 — cheap: our HPGL path + a per-brand profile

HPGL is the budget-cutter lingua franca. Our `driver-hpgl` (built for the GCC
Puma IV) generalizes directly to the huge "same machine, many names" class — all
plain HPGL, differing mainly in width and speed/force ranges.

- **Generic budget cutters:** USCutter (MH, LaserPoint, SC, TC, Refine, Copam),
  Creation PCut, Vinyl Express, Redsail, GoldCut, Vevor, Liyu, Rabbit, and
  unbranded units. Mainstream cutting software supports [800+](https://www.easycutstudio.com/supported-vinyl-cutters.html)
  of these precisely because they share HPGL.
- **Other GCC:** Expert, Jaguar, i-Craft — native HPGL; we already target GCC.

**Effort: one HPGL driver + a profile per model.** Hundreds of machines.

## Tier 3 — low-moderate: HPGL plus a dialect wrinkle

- **Roland CAMM-1** (GX-24, CM, PNC): [CAMM-GL is HPGL-derived](https://downloadcenter.rolanddg.com/contents/manuals/CAMM-GL2_PRO_EN_R1.pdf)
  plus `!`-prefixed extensions (`!ST`, and `VS`/`FS` for speed/force). HPGL core
  cuts immediately; extensions add pressure/speed control.
- **Summa** (SummaCut, S-Class): [supports HP/GL + HP/GL/2 and native DM/PL](https://help.cadlink.com/website/engravelab_productionspooler/en/tech_support/hpgl_dmpl_gpgl_meanings.htm).
  HP/GL mode works day one; native DM/PL is a later dialect for extra features
  (polling, knife-specific commands).
- **Pro Graphtec** (FC/CE series): native GP-GL (we already have GPGL) or HPGL.

## Out of scope

- **Cricut** — closed platform.
- **Brother ScanNCut** — proprietary / cloud + USB-mass-storage workflow.

## Reference sources to mine (all reusable under GPLv3)

- [`inkscape-silhouette`](https://github.com/fablabnbg/inkscape-silhouette) — GPGL, Silhouette/Graphtec device table (already porting).
- [`InkCut`](https://github.com/inkcut/inkcut) — GPL; HPGL device profiles + serial handling worth porting.
- [Graphtec GPGL reference](https://www.ohthehugemanatee.net/2011/07/gpgl-reference-courtesy-of-graphtec/) — native GP-GL for pro Graphtec.
- [Roland CAMM-GL II Programmer's Manual](https://downloadcenter.rolanddg.com/contents/manuals/CAMM-GL2_PRO_EN_R1.pdf) — CAMM-GL extensions.

## Implication for sub-project 2 (drivers + CLI)

Design `driver-core` around two protocol backends (GPGL, HPGL) + a model-profile
table, not one-driver-per-machine. Ship the Cameo 5 Alpha (GPGL) and GCC Puma IV
(HPGL) first as the two backends' reference machines; every Tier 1/2 model after
that is a table entry, not a new driver.
