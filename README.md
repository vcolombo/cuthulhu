<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# cuthulhu

Free, open-source (GPLv3) desktop cutting software for vinyl cutters and craft
plotters — a modern, no-lock-in alternative to the proprietary software that
ships with these machines.

The product is a Rust workspace behind a Tauri desktop app and a CLI. Both
share one planning path: a Document becomes DocumentPasses, a CutPlan, then
bytes on a Transport. `tools/` is the frozen Python protocol spike. Do not
grow it; it is research tooling, not the app.

Nothing has been released yet. Build from source.

## Machines

- **Silhouette Cameo 5** (`cameo5`) — USB, GPGL, ported from `inkscape-silhouette`.
- **GCC Puma IV** (`puma`) — HPGL over serial.
- Other non-Cricut HPGL/GPGL cutters are the roadmap, not this build. Cricut
  is out of scope (closed platform). See `docs/roadmap/supported-machines.md`.

A Cut Host (`cuthulhu-cutd`) owns the USB and serial connections on another
machine, usually a Raspberry Pi, and runs Jobs for desktops on the network.
Once a Job starts, closing the laptop that sent it does not stop it. Setup,
the protocol-version mismatch, and how to stop the daemon mid-Job are in
`docs/cut-host.md`.

## Run it

Desktop (needs [Rust](https://rustup.rs), Node 22, and a global `cargo-tauri`).
On Linux the native libraries are the ones CI installs:
`libudev-dev`, `pkg-config`, `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`librsvg2-dev`.

```sh
npm --prefix apps/desktop/ui ci
(cd apps/desktop && cargo tauri dev)
```

`cargo tauri` finds its config by searching subfolders, never parents. Run it
from `apps/desktop`. From `apps/desktop/ui` it reports that the folder is not
a Tauri project.

CLI, same engine, no window:

```sh
cargo run -p cli -- cut design.svg --device cameo5 --dry-run
cargo run -p cli -- cut design.svg --device puma --port /dev/ttyUSB0 --dry-run
cargo run -p cli -- list-devices
cargo run -p cli -- trace photo.png -o traced.svg
```

`--dry-run` prints the wire bytes and does not open a cutter. A bare `cut`
is one pass over every cut shape (`--group-by single`). The desktop opens on
a different Grouping — see below.

The spike senders are still there, and still not the product:

```sh
cd tools && python3 -m pytest
python3 tools/replay/send_serial.py --port /dev/ttyUSB0 --size-mm 20 --dry-run
python3 tools/replay/send_raw.py --size-mm 20 --dry-run
```

## Desktop

**Send to cutter** opens the Send sheet in three steps: Machine, Material &
passes, Preview & cut. Next stays on Machine until a cutter is aimed and its
status says a cut is legal (`actions.cut`); the hint is "Pick a cutter that's
ready." Material & passes is where a pass is skipped, reordered, given a
MaterialPreset, or given a weed border and weed lines. The border and the
lines are generated for that send; they are not stored in the Document, and
the sheet's weed settings last for the session only.

**Group passes by** on that step is the Grouping for the whole window. The
Layers panel and the document facts in Properties read the same value. The
desktop starts on *Colour (stroke, else fill)* and does not remember the
choice after quit. The CLI default stays one pass. Sending rows planned
under one Grouping with another Grouping cuts different shapes: the mode
travels with the plan, the travel preview, and the cut.

**Devices** lists every cutter this desktop can see — plugged into this
computer and on every paired Cut Host — not only the one the top bar is aimed
at. A host that answered on the wrong protocol version says which side to
update. A host that cannot be reached keeps the last reading that worked.
Forgetting a host that is disconnected or faulted needs the force the dialog
offers; a cutter that is actually cutting, or stopped where nothing saw it
stop, cannot be forgotten.

**Layers** groups shapes into the passes that Grouping would cut. Dragging
reorders a shape among its siblings. Pass order is the Send sheet's.

**Properties** shows lengths in the chosen unit. The Document is stored in
millimetres either way. The first launch with no saved unit follows the
computer's region: United States, Liberia, and Myanmar in inches, everyone
else in millimetres. A saved `in` or `mm` is not revised when the region
changes. Appearance (light, dark, or system) is the other saved preference.
Both live in `<config dir>/cuthulhu/preferences.json`.

Undo and redo are the editor's inverse-delta stacks (⌘Z, ⇧⌘Z). They are not
part of the project file. Cmd/Ctrl+A selects every shape the canvas drew,
once each, and does not run inside a field, a dialog, or a menu.

Closing the window asks before it goes if any cut this desktop started may
still be running, on any cutter, including one the top bar is no longer aimed
at. On macOS, Cmd-Q and the application menu's Quit ask the same question.
The prompt's answer is the contract: sending to the cutter on this computer
stops, but that cutter finishes moves it has already buffered; a Job already
sent to a Cut Host keeps running there. The Dock's Quit and a logout are not
covered — they still terminate the process without the prompt. See
`docs/adr/0002-the-close-guard-answers-for-every-cut-this-desktop-started.md`.

## Develop

```sh
cargo test --workspace --locked
npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run e2e
```

Three checks fail on ordinary-looking commits:

- **`Cargo.lock`**. CI runs `cargo test --workspace --locked` and will not
  rewrite the lock. Add a dependency and commit the lock with it.
- **`apps/desktop/ui/dist`**. The bundle is committed because the Rust build
  reads `frontendDist` with no Node installed. Change `ui/src`, then run
  `npm --prefix apps/desktop/ui run build` and commit `dist/`.
- **`apps/desktop/ipc-inventory.json`**. The e2e fake refuses any Tauri
  command or argument name absent from this file. After changing a command,
  regenerate with
  `UPDATE_IPC_INVENTORY=1 cargo test -p desktop --test ipc_inventory`
  and commit the result.

`CLAUDE.md` is the rest of the contributor map: the crate spine, why
`plan_cut` is the only way to a machine, and the Playwright install flag that
silently drops `--with-deps` if you leave out `exec --`.

## Layout

- `crates/` — `geometry`, `document`, `fileio`, `cutplan`, `trace`,
  `driver-core` (traits only), `driver-registry`, `driver-silhouette`,
  `driver-hpgl`, `cli`, `cut-host`.
- `apps/desktop` — Tauri shell. Logic is one method per command in
  `src/state.rs`; `src/ipc.rs` only maps errors. The UI is
  `ui/src`, and it reaches Rust only through `ui/src/ipc.ts`.
- `tools/` — frozen spike.
- `docs/protocol/` — machine protocol notes, with cited sources.
- `docs/adr/` — decisions that are still in force.
- `docs/superpowers/` — design specs and plans. When one disagrees with the
  code, trust the code.
- `CONTEXT.md` — the vocabulary. Use it. A DocumentPass is not a layer.

## License

GPL-3.0-or-later — see `LICENSE`. Reuses the GPL drivers `inkscape-silhouette`
and `robocut`; attribution and source citations live in `docs/protocol/`.
