# Manual checklist (per release)
- [ ] Switch machine Cameo 5 ⇄ Puma IV — artboard resizes on the canvas and in the status bar.
- [ ] Import a complex SVG via the TopBar Import button — paths render as real geometry; stays responsive (pan/zoom deferred — SP4 candidate).
- [ ] Drag/scale — drag via the canvas, scale via the properties panel W/H fields; one undo reverts the whole gesture. Rotate + on-canvas handles (deferred).
- [ ] Boolean union/subtract/intersect/exclude on two overlapping shapes — correct result rendered as a real path.
- [ ] Add text in a system font — glyph outlines render on the canvas and convert to a cut path on save.
- [ ] Save via the native file dialog to a chosen location; Open restores the project; Reload re-reads the last used path.
- [ ] Dark theme legible; tabular numerals align in the properties panel (light theme deferred).

## SP4 — Cut workflow

### Hardware: Cameo 5 (closes SP4) — verified 2026-07-24 on Cameo 5 Alpha Plus (0x3844:0x0002)
- [x] One prologue and one epilogue verified on-device across a 2-pass job.
      (Dry-run bytes: ESC EOT + J1 only on pass 1; SO0/FN0 only after pass 2. On-device 2-pass job completed through swap/resume.)
- [x] Moving→ready ENQ status polling observed between passes.
      (Required a fix found by this checklist: the query must be ESC ENQ `1b 05`, not bare `0x05` — bare ENQ gets no reply and times out the 60s poll cap.)
- [x] Safe park sequence verified between passes (pen-up, no media movement).
- [x] Registered two-color overlay cut aligns correctly on the device.
- [x] Cancel mid-cut stops the machine (best-effort abort behavior).
      (Host lands on Cancelled promptly; Silhouette has no abort command, so the machine finishes its already-buffered moves before stopping.)
- [x] Unplug mid-cut results in device error plus graceful recovery.
      (Typed `Io("Unknown")` error, no panic/hang; replug + fresh cut succeeded. nusb's disconnect error string is opaque — polish candidate.)

### Hardware: Puma IV (non-blocking)
- [ ] Multi-color cut via operator-confirmed pass completion (manual "pass done" button).
- [ ] Host-queue drain ≠ cutter completion — explicitly verified that operator must confirm motion stopped before proceeding.

### GUI verification — verified 2026-07-24 in the real app (imported 2-color SVG, Cameo 5 connected)
- [x] Open Cut dialog on a 2-color document — displays 2 passes with correct color swatches.
- [x] Preview shows order badges at shape start points and dashed travel lines in planned order (reordering passes does not yet update the travel lines — deferred).
      (Badges/travel render, but the preview scales to the full 330×3000mm artboard so small docs are tiny — polish candidate: fit-to-content zoom.)
- [x] Material preset selection works; per-pass override fields functional (subject to machine capabilities).
      (Builtin presets appear after connect; picking Cardstock auto-fills force from the preset while a typed speed override wins.)
- [x] Cut-by-color pass reorder and skip operations work via up/down and enable/disable toggles.
- [ ] "No device" empty state is graceful (no error, device list shows "no devices" message).
      (Not reachable on a real Mac: the OS always exposes serial ports, so unverified puma candidates always populate the list — every /dev/cu.* shows as its own "puma (unverified serial device)" row, which is itself a polish candidate. Empty-state render covered by the e2e mock test.)

## SP5 Trace

- [ ] Trace a real logo PNG in binary mode — preview updates live as sliders move; Insert lands correctly sized paths on the canvas; one undo removes them all.
- [ ] Trace a multi-color image in color mode — one path per color; colors match; cut-by-color in the Cut dialog lists the traced colors as passes.
- [ ] Trace a photo larger than 2048 px — "reduced to 2048 px" note appears; app stays responsive.
- [ ] Max speckle filter on a small image — "Nothing traced" hint (not an error banner); lowering the slider recovers the preview.
- [ ] Pick a non-image file via the picker filter bypass (rename a .txt to .png) — error banner in dialog, dialog still usable.
- [ ] **In a packaged build (`tauri build`, not `tauri dev`)** — both the source thumbnail and the traced preview actually render. The e2e suite runs against Vite and never applies the Tauri CSP, so a missing `img-src 'self' data:` blocks both images with green tests. Check the webview console for CSP violations.
- [ ] Trace a fine-detail image (dithered scan, halftone, or a one-pixel checkerboard ≥512 px) — a tracer failure shows as an error banner; the app does not crash or hang.
