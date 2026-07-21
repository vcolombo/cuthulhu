# Protocol notes — clean-room provenance

These notes describe cutter USB/serial protocols reconstructed **only** from:
1. USB captures we recorded ourselves (files under `tools/capture/samples/`, git-ignored; trimmed `.hex` fixtures committed alongside decoder tests).
2. Public vendor manuals and community protocol writeups (cited by URL).

We do **not** read, translate, or paraphrase GPL-licensed drivers
(`inkscape-silhouette`, `robocut`, potrace). Anyone contributing protocol
facts must state the source for each claim.

## Citation format

Every documented command references its evidence:
- Capture-derived: `[cap: cameo5-square-2026-07-21.pcapng #142-158]`
- Doc-derived: `[doc: <title>, <url>, section/page]`

## Files
- `silhouette-cameo5.md` — Cameo 5 Alpha command set (capture-derived).
- `gcc-hpgl.md` — GCC Puma IV HPGL command set (public-doc-derived).
