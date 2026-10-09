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
- [ ] Speed and force grey out when the Puma connects and stay editable for the Cameo.
      (Caps now come from the Driver over `machine_caps` rather than a table in `CutDialog.tsx`. Everything either side of the link is pinned — wire casing by a Rust test, per-machine values by each Driver's own caps test, the disable rule by four vitest cases — but nothing automated asserts that the fetched value reaches the field, because the e2e fake deliberately answers with one constant rather than restating the per-machine mapping it exists to remove.)

### GUI verification — verified 2026-07-24 in the real app (imported 2-color SVG, Cameo 5 connected)
- [x] Open Cut dialog on a 2-color document — displays 2 passes with correct color swatches.
- [x] Preview shows order badges at shape start points and dashed travel lines in planned order (reordering passes does not yet update the travel lines — deferred).
      (Badges/travel render, but the preview scales to the full 330×3000mm artboard so small docs are tiny — polish candidate: fit-to-content zoom.)
- [x] Material preset selection works; per-pass override fields functional (subject to machine capabilities).
      (Builtin presets appear after connect; picking Cardstock auto-fills force from the preset while a typed speed override wins.)
- [x] Cut-by-color pass reorder and skip operations work via up/down and enable/disable toggles.
- [ ] "No device" empty state is graceful (no error, device list shows "no devices" message).
      (Was unreachable on a real Mac — the OS always exposes serial ports, so every /dev/cu.* and its /dev/tty. twin showed as its own "puma (unverified serial device)" row. Fixed for #10: `list_ports` now drops dial-in duplicates and the macOS system pseudo-ports, so the list is empty on a Mac with no cutter and no paired Bluetooth serial device. The empty-state message itself is new and unverified — no automated test covers it, so this item needs a real run.)

## SP5 Trace — verified 2026-07-25 against a packaged `cargo tauri build` bundle

- [x] Trace a real logo PNG in binary mode — preview updates live as sliders move; Insert lands correctly sized paths on the canvas; one undo removes them all.
      (Re-verified after the alpha fix in #14. The first pass measured a 512 px logo at W=H=135.4667 mm and read that as proof of correct sizing — but that path was the transparent-background artifact spanning the whole canvas, so the number was right about the px→mm scale and wrong about the geometry. With alpha composited, the same logo traces to one path, the glyph, inserted at W=73.522 H=83.218 mm at X=29.232 Y=26.365 — matching the traced SVG's own extent and origin exactly. Moving Detail re-traced live; Binary→Color went 2→37 paths. One undo empties the layer list, including the 4-path colour insert.)
- [x] Trace a multi-color image in color mode — one path per color; colors match; cut-by-color in the Cut dialog lists the traced colors as passes.
      (A 4-color image gives exactly 4 paths and the fills round-trip exactly: #FFD600/#CC0000/#0066CC/#009944. Initially failed the cut-by-color clause — vtracer emits `fill`-only paths while `plan_passes` groups by *stroke*, so the Cut dialog reported "Not cut: 4 shapes" and listed zero passes. Fixed in #15 by mirroring the traced fill onto the stroke; re-verified in a fresh bundle, where the dialog now lists 4 passes with the matching swatches and "Not cut: 0 shapes". Since #144 that mirroring is deleted and the same outcome holds for a different reason: a traced path is cut because import defaults its `CutLineType` to `Cut`, and its pass is keyed on the visible stroke, else the visible fill. Anyone diagnosing a future failure of this item should look there, not for a mirroring function that no longer exists.)
- [x] Trace a photo larger than 2048 px — "reduced to 2048 px" note appears; app stays responsive.
      (7952×5304 photo → "926 paths — large image reduced to 2048 px for tracing", matching the CLI. Slider moves stayed responsive and re-traced, 926→208 paths.)
- [x] Max speckle filter on a small image — "Nothing traced" hint (not an error banner); lowering the slider recovers the preview.
      (Shown as muted hint text, not the red error style. Lowering speckle to 1 recovered 36 paths, matching the CLI.)
- [x] Pick a non-image file via the picker filter bypass (rename a .txt to .png) — error banner in dialog, dialog still usable.
      (Banner: "could not read image: The image format could not be determined". Controls still responded afterwards — the mode radio toggled fine.)
- [x] **In a packaged build (`tauri build`, not `tauri dev`)** — both the source thumbnail and the traced preview actually render. The e2e suite runs against Vite and never applies the Tauri CSP, so a missing `img-src 'self' data:` blocks both images with green tests. Check the webview console for CSP violations.
      (Both images render in the bundled .app; the configured `img-src 'self' data:` is sufficient for the base64 thumbnail and the SVG data URL. No image was blocked.)
- [x] Trace a fine-detail image (dithered scan, halftone, or a one-pixel checkerboard ≥512 px) — a tracer failure shows as an error banner; the app does not crash or hang.
      (512×512 one-pixel checkerboard: "trace failed: tracer failed on this image; try a lower detail setting". The `catch_unwind` around `vtracer::convert` holds in a release bundle — process stayed alive and the UI stayed responsive.)

## Device layer — pre-SP6 fixes (unverified)

Everything here is hardware behavior that no automated test reaches. All of it wants a run
before SP6 leans on device connection and mid-job disconnect detection.

- [ ] Connect to a real Puma IV over serial — the connect-time probe is answered and the device reaches Idle with no visible delay.
- [ ] Connect to a serial device that is not a cutter (a paired Bluetooth device is the easy case) — refused with a message naming the port, and the dialog stays usable afterwards. Confirm the device itself was not disturbed by the one status byte the probe writes.
- [ ] Unplug the Cameo mid-cut — the job fails as a disconnect, not `Io("Unknown")`.
- [ ] Unplug a USB-serial adapter mid-cut — the job fails promptly as a disconnect rather than waiting out the completion-poll deadline.
- [ ] With no cutter attached and no paired Bluetooth serial device, the cut dialog shows the "No devices found" empty state.

Driving note: WebKit range inputs ignore both synthetic `click at` positioning and AX `set value`. The
reliable way to move a slider is AX `set focused to true` on it, then arrow-key key codes.

## Cut Host resolver (#126, unverified)

The `.local` path now resolves over mdns-sd rather than the OS resolver; only a real Pi on a
real network can confirm the swap end to end.

- [ ] Pair a Cut Host by `cuthulhu-pi.local` and list its cutters — the lookup succeeds and the
      paired host cuts.
- [ ] Wedge the network (drop the Pi's Wi-Fi mid-poll) — the desktop's connect attempt fails
      within ~5s, no `resolve …` threads accumulate in Activity Monitor, and the host is
      reachable again once the network returns.
- [ ] Dual-stack Pi answering A and AAAA as separate packets — the second family lands within
      the 150ms grace window and the connect loop gets both families, v4 first.
- [ ] A v6 link-local answer (`fe80::…`) actually connects — the scope id survives from the
      mDNS answer to the dial (v6-only or v4-blocked network needed to force the path).

## CLI plain cut path (architecture review candidate 3)

- [ ] `cuthulhu cut fill-only.svg --device cameo5 --dry-run` — a fill-only SVG still produces bytes (one pass).
- [ ] `cuthulhu cut off-bed.svg --device cameo5` — refused with the out-of-bounds message, nothing sent.
- [ ] `cuthulhu cut off-bed.svg --device cameo5 --allow-out-of-bounds` — sends.
- [ ] `cuthulhu cut a.svg --skip-pass color:FF0000FF` — refused, naming `--group-by single`.
- [ ] On hardware: a plain cut on the Cameo 5 completes, and Ctrl-C mid-cut stops it.
- [ ] Scripted (stdin redirected from /dev/null): `cuthulhu cut a.svg --device puma --port …` completes without blocking, and prints the completion-not-verified note.

## CutStatus (architecture review candidate 1)

- [ ] Cut dialog buttons match what the machine allows at each stage — nothing enabled that errors when pressed.
- [ ] Progress advances during a pass; pass n of m is correct across a colour swap.
- [ ] Cancel mid-cut reports the cancelled ending, the dialog shows "Cancelled", and a second cut can be started straight after.
- [ ] Unplug mid-cut shows the failure with its reason, and the dialog offers no dead buttons.
- [ ] Closing the window mid-cut still prompts.
- [ ] `cuthulhu cut --group-by color` on the Puma still prompts per pass and completes.

## Pass grouping (#148)

- [ ] Cut dialog → Group passes by: each mode replans and the rows rename themselves; Cut is
      unavailable until the new plan lands.
- [ ] Assign a material to a Layer in the properties panel — every shape under it reads
      `Inherited — <name>`, and one shape set to "No preset" leaves that Layer's pass.
- [ ] On hardware: `--group-by preset` on a document with two materials cuts one pass per
      material, **each with that preset's own speed and force**, prompting between them. Nothing
      but real material settles that the settings followed the key.

## Viewport and handles (spec 2026-10-06, unverified)

- [ ] Launch on a Retina Mac — strokes and handles are crisp, not blurred; the artboard fits the window.
- [ ] Trackpad: pinch zooms about the fingers; two-finger scroll pans; ⌘0 refits; ⌘1 is CSS reference
      size — record how far a 100 mm rect is from 100 mm on a ruler, with the display and its OS
      scaling (it is exact only at the reference density; per-monitor calibration is a ponytail).
- [ ] Mouse (Windows/Linux): the wheel pans; Ctrl-wheel zooms; middle-drag pans; Space-drag pans.
- [ ] Switch machine Cameo 5 ⇄ Puma IV — the view refits to the new artboard. Resizing the window
      does not refit.
- [ ] Marquee a word of imported text; Shift-drag a corner — it scales proportionally; Alt-drag
      scales from the centre; one undo reverts each gesture.
- [ ] Pinch-zoom or scroll-pan while dragging a handle — the shape keeps following the pointer
      instead of snapping back, and the release saves what is on screen. (Canvas pixels are
      unreadable from e2e, so this one is only checkable by eye.)
- [ ] Rotate a rect 30° with Shift held (snaps in 15° steps), cut it on the Cameo 5, and measure —
      the cut matches the screen.
- [ ] `node apps/desktop/ui/scripts/perf-svg.mjs > /tmp/perf.svg`, Import it, pan and zoom with the
      WebKit inspector's frame timeline open — record the frame rate here with the machine and date
      (target 60 fps).

## Snapping (spec 2026-10-07, unverified)

- [ ] Guides read clearly against the artboard and the workspace, distinct from the cyan selection.
- [ ] Snapping feels sticky but not grabby at fit and at 400%; small deliberate offsets are still
      reachable with ⌘ (Ctrl) held.
- [ ] Pressing ⌘ (Ctrl) mid-drag releases a snap; releasing it snaps again on the next move.
- [ ] A decal snapped to the artboard's left edge cuts flush with the mat's left limit on the Cameo 5.

## Align and distribute (spec 2026-10-07, unverified)

- [ ] Six names aligned left cut in a straight column on the Cameo 5: every left edge on one line.
- [ ] A single decal centred with "Align horizontal centres" cuts centred on the mat.
- [ ] A row of letters distributed horizontally weeds with even gaps between neighbours.
- [ ] Letters distributed with their weed border selected sit inside it with even margins, and the
      border cuts where it was.
- [ ] One ⌘Z (Ctrl+Z) after an align puts every moved piece back at once.
- [ ] The eight icons read as what they do at the panel's size, in the dark theme (light is deferred).

## Weed lines (spec 2026-10-09, unverified)

- [ ] A word cut on the Cameo 5 with Border on (3 mm): the border is cut last, after every letter,
      and the sheet lifts off the roll in one piece.
- [ ] The same word with Lines: Both, spacing 25 mm, clearance 1.5 mm: no line nicks a letter, and
      the waste weeds off strip by strip, including from inside the counters (O, A, e).
- [ ] A two-colour design grouped by colour: each colour's sheet gets a border sized to its own
      shapes, cut in that colour's pass.
- [ ] A design within 3 mm of the mat edge with Border on: the cut is refused, and the message says
      to shrink the weed margin or move the design.
- [ ] `cuthulhu cut --group-by color --weed-margin 3 --weed-lines h` on the Puma IV: each pass ends
      with its lines and then its border, and the strips weed.
- [ ] Tune the defaults (margin 3, spacing 25, clearance 1.5 mm) on real vinyl and HTV; record the
      values that weed cleanly here.
- [ ] Lines: Both at 25 mm on the densest real job at hand (a full sheet of small lettering, a
      trace): it is planned without "too large or detailed", and typing in a weed field does not
      lag. The work budget (`cutplan::weed::MAX_WEED_WORK`, 50M tests) measured about 2.9M for a
      synthetic trace of 1000 closed 100-sided blobs over 300 mm at the defaults (test
      `a_dense_trace_at_the_default_spacing_is_weeded_not_refused`); raise it, or take the
      sorted-crossing upgrade, if a real job hits it.
