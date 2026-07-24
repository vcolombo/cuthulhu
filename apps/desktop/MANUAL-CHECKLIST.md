# Manual checklist (per release)
- [ ] Switch machine Cameo 5 ⇄ Puma IV — artboard resizes on the canvas and in the status bar.
- [ ] Import a complex SVG via the TopBar Import button — paths render as real geometry; stays responsive (pan/zoom deferred — SP4 candidate).
- [ ] Drag/scale — drag via the canvas, scale via the properties panel W/H fields; one undo reverts the whole gesture. Rotate + on-canvas handles (deferred).
- [ ] Boolean union/subtract/intersect/exclude on two overlapping shapes — correct result rendered as a real path.
- [ ] Add text in a system font — glyph outlines render on the canvas and convert to a cut path on save.
- [ ] Save via the native file dialog to a chosen location; Open restores the project; Reload re-reads the last used path.
- [ ] Dark theme legible; tabular numerals align in the properties panel (light theme deferred).
