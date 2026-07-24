# Protocol notes — sources & attribution

cuthulhu is **GPL-3.0-or-later**, so we may reuse existing GPL cutter drivers
directly. These notes reconstruct cutter USB/serial protocols from any of:

1. **GPL driver source** we port or adapt — `inkscape-silhouette`
   (GPL-2.0-or-later) and `robocut` (GPLv3-or-later). Any file that ports their
   code must keep the original copyright and license notice intact.
2. **USB captures** we recorded ourselves (`tools/capture/samples/`, git-ignored;
   trimmed `.hex` fixtures committed alongside decoder tests) — now used to
   *validate* the ported protocol, not to originate it.
3. **Public vendor manuals** and community protocol writeups (cited by URL).

Every documented command still cites its source — for attribution/GPL
compliance and traceability, not for clean-room isolation.

## Citation format

- GPL-source-derived: `[src: inkscape-silhouette silhouette/Graphtec.py L120-155 (GPL-2.0+)]`
- Capture-derived:     `[cap: cameo5-square-2026-07-21.pcapng #142-158]`
- Doc-derived:         `[doc: <title>, <url>, section/page]`

## Material presets

Builtin preset values for common materials are sourced from the Cameo/Puma driver
defaults in `fablabnbg/inkscape-silhouette` (GPL-2.0+): `[src: inkscape-silhouette Cameo/Puma defaults (GPL-2.0+)]`

## Files
- `silhouette-cameo5.md` — Cameo 5 Alpha command set.
- `gcc-hpgl.md` — GCC Puma IV HPGL command set (public-doc-derived).
