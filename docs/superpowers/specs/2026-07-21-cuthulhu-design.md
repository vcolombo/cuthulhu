# Cuthulhu — cross-platform cutting software: design spec

Date: 2026-07-21
Status: approved (brainstorming complete)

## Purpose

Free, open-source (GPLv3), cross-platform (Windows/macOS/Linux) cutting software that beats the proprietary incumbents on four fronts:

1. **Stability / performance** — no crashes or laggy canvas on complex designs.
2. **Open file formats** — no lock-in; documented project format, faithful SVG import/export.
3. **Modern UX** — fast, keyboard-friendly, non-modal workflows.
4. **Multi-machine support in one app** — drives Silhouette and generic HPGL cutters equally well.

Primary reference hardware: **Silhouette Cameo 5 Alpha** (USB) and **GCC Puma IV** (HPGL over serial/USB). Other common non-Cricut cutters follow. Cricut is out of scope: its platform is cloud-tethered and closed to third-party software.

## Licensing (decided)

- Whole project licensed **GPL-3.0-or-later**. GPLv3 is the umbrella that lets us reuse the existing GPL cutter drivers: `inkscape-silhouette` (GPL-2.0-or-later, upgrades into v3) and `robocut` (GPLv3-or-later).
- We **may** vendor, link, and port those drivers' protocol code, keeping attribution and license notices intact. Trace may use GPL potrace or MIT vtracer (both GPL-compatible).
- Fully open source and community-developed: every feature ships in this GPLv3 codebase.
- Every ported protocol fact still cites its source (`docs/protocol/README.md`) — now for attribution/compliance, not clean-room isolation.

## Architecture

Tauri application over a Rust engine. The engine is the open-source product; the UI is a thin shell.

```
cuthulhu/
├── crates/                    # Rust workspace = the open-source engine
│   ├── geometry/              # kurbo/lyon: booleans, offsets (weed lines), transforms, text-to-path
│   ├── driver-core/           # device abstraction: enumeration, job model, settings (force/speed/passes)
│   ├── driver-silhouette/     # USB driver (ported from inkscape-silhouette), Cameo 5 Alpha first
│   ├── driver-hpgl/           # HPGL/DMPL over serial+USB, GCC Puma IV first
│   ├── trace/                 # vtracer bitmap→vector
│   ├── fileio/                # usvg SVG import, DXF, PDF; open project format
│   └── cli/                   # `cuthulhu cut file.svg --device cameo5`
├── apps/desktop/              # Tauri shell
│   └── ui/                    # TypeScript + React, Canvas2D custom renderer (WebGL only if perf demands)
```

Rules:

- The UI reaches geometry and devices only through Tauri IPC. The Rust core is fully testable headless.
- Project file is an open, documented zip container: `design.svg` + `manifest.json` + assets.
- The CLI is both a development tool and the community-facing artifact proving the drivers work standalone.

Why Tauri + Rust over Electron and Qt: native USB/serial access without fragile native Node modules, Rust-speed geometry for large designs (the stability/performance goal), and small binaries. The Rust crate ecosystem is MIT/Apache, which composes cleanly into our GPLv3 whole.

## Build order

Six sub-projects. Each gets its own spec → plan → implementation cycle; this document governs the whole.

1. **Protocol spike** — port the Silhouette protocol from `inkscape-silhouette` and assemble the GCC HPGL command set from public docs; USB captures now validate rather than originate the notes. Deliverable: protocol notes committed to the repo.
2. **Drivers + CLI** — both machines cut a square from an SVG via the CLI. Proves the premise before any UI exists.
3. **Editor shell** — canvas, layers, select/transform, boolean ops, text (system fonts), SVG/PDF/DXF import, project save/load.
4. **Cut workflow** — device manager, material presets, cut dialog with preview, weed lines, cut-by-color.
5. **Trace** — vtracer integration with cleanup controls.
6. **Print & cut** — registration-mark printing, Cameo 5 optical registration, GCC AAS contour cutting. Hardest feature, deliberately last.

## Errors & testing

- **Drivers:** golden-file tests (job → expected GPGL/HPGL byte stream); mock device transport in CI; real-machine manual checklist per release.
- **Geometry:** property tests — booleans and offsets produce valid, non-self-intersecting paths.
- **Device errors:** surfaced as typed states (disconnected / busy / media jam). Jobs never fail silently.
- **Print & cut:** physical accuracy procedure measuring cut-vs-print offset in millimeters.

## Out of scope (v1)

- Cricut support in any form.
- Runtime plugin API.
- Rhinestone/engraving/emboss tool workflows (roadmap after v1).
- Cloud accounts, sync, or design marketplace.
