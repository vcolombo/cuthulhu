# Manual checklist (per release)
- [ ] Switch machine Cameo 5 ⇄ Puma IV — artboard resizes on the canvas and in the status bar.
- [ ] Import a complex SVG via the TopBar Import button — paths render as real geometry; stays responsive (pan/zoom deferred — SP4 candidate).
- [ ] Drag/scale — drag via the canvas, scale via the properties panel W/H fields; one undo reverts the whole gesture. Rotate + on-canvas handles (deferred).
- [ ] Boolean union/subtract/intersect/exclude on two overlapping shapes — correct result rendered as a real path.
- [ ] Add text in a system font — glyph outlines render on the canvas and convert to a cut path on save.
- [ ] Save via the native file dialog to a chosen location; Open restores the project; Reload re-reads the last used path.
- [ ] Dark theme legible; tabular numerals align in the properties panel (light theme deferred).

## SP4 — Cut workflow

### Hardware: Cameo 5 (closes SP4)
- [ ] One prologue and one epilogue verified on-device across a 2-pass job.
- [ ] Moving→ready ENQ status polling observed between passes.
- [ ] Safe park sequence verified between passes (pen-up, no media movement).
- [ ] Registered two-color overlay cut aligns correctly on the device.
- [ ] Cancel mid-cut stops the machine (best-effort abort behavior).
- [ ] Unplug mid-cut results in device error plus graceful recovery.

### Hardware: Puma IV (non-blocking)
- [ ] Multi-color cut via operator-confirmed pass completion (manual "pass done" button).
- [ ] Host-queue drain ≠ cutter completion — explicitly verified that operator must confirm motion stopped before proceeding.

### GUI verification
- [ ] Open Cut dialog on a 2-color document — displays 2 passes with correct color swatches.
- [ ] Preview shows order badges at shape start points and dashed travel lines in configured order.
- [ ] Material preset selection works; per-pass override fields functional (subject to machine capabilities).
- [ ] Cut-by-color pass reorder and skip operations work via up/down and enable/disable toggles.
- [ ] "No device" empty state is graceful (no error, device list shows "no devices" message).
