<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Changelog

Notable changes to Cuthulhu. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Nothing has been released yet, so there is no history before `Unreleased`.

## [Unreleased]

### Changed

- **Cut passes can be grouped by stroke colour, fill colour, or material preset**, chosen in the
  cut dialog and with `cuthulhu cut --group-by <single|color|stroke|fill|preset>`. `color` is the
  previous rule — stroke where visible, else fill — and stays what the dialog opens on; `single`
  is one pass over every cut shape, and stays what a bare `cuthulhu cut` does.

- **A Node can carry a material preset, and a Layer passes it down.** The properties panel's new
  Material control offers Inherit, No preset, or a named preset, and shows what an inherited
  value resolves to. A pass grouped by material is cut with that material's own speed and force.

- **`--by-color` and `--skip-color` are gone.** `--group-by color` replaces the first;
  `--skip-pass` replaces the second and names passes by key (`color:ff0000ff`, `no-color`,
  `preset:cameo5-htv`, `no-preset`, `all`) rather than by bare hex. `--order` takes the same keys
  and is now **repeatable** instead of comma-separated, because a preset id may contain a comma.
  Both flags now refuse a key that names no planned pass, where `--skip-color` ignored one
  silently. A cut whose selection skipped every pass says so, instead of reporting an empty file.

- `cuthulhu trace --detail` is now stated in the same units as the desktop's Detail slider: **higher
  means more detail**. It previously carried vtracer's `length_threshold` verbatim, which runs the
  other way, so the two interfaces used one word for opposite things and printed opposite advice for
  the same failure. The default is unchanged in effect — the old `4.0` is the new `9.5`. A script
  passing `--detail` explicitly will trace differently and should be updated by reflecting its value
  through the range: `13.5 - old`.

### Added

- **The cut dialog manages the operator's own material presets** for the cutter it is aimed at:
  create, duplicate, rename, edit and delete, with speed, force and repeat count validated against
  the ranges `cutplan` refuses a cut over, and a readout of what a pass cut with the preset would
  use. A preset that ships with the app is read-only and offers **Save as Copy**, which writes a
  fresh entry rather than shadowing the shipped one. A rename moves the name only — a preset's id
  is what a Node's assignment and a `preset:<id>` pass key name it by. An unsaved edit is decided
  before it can be lost: selecting another preset, aiming at another cutter and closing the dialog
  each ask first, and duplicating or deleting — both of which work from the stored entry — wait for
  that decision. A write the backend refuses keeps the edit on screen with the reason beside it.
  Editing is offered only once the aimed cutter's own presets have been read, since they are what a
  new entry's name and id have to avoid. Saving over a built-in's pair, an entry with no id or a
  blank name, a setting out of range, and a delete that would remove nothing are each refused by
  name rather than reported as success.

- The desktop shell has an application icon. macOS and Windows bundles now get a real `.icns` and
  `.ico`, where `bundle.icon` previously listed only a 32 px placeholder PNG. Both containers are
  multi-resolution and carry two artworks: the kraken mascot at 31 px and above, and a C mark below
  it, since the mascot's knife and tentacle curls stop resolving at small sizes. The Windows Store
  tile set ships alongside, including explicit target-size assets so the taskbar entry uses the C
  mark rather than a shrunken mascot. The artwork is a glyph on a transparent background — macOS 26
  and the Windows taskbar composite icons onto their own backplates, so a self-drawn tile renders
  tile-in-tile. On macOS 26+ the bundle also ships an Icon Composer icon (`Assets.car` +
  `CFBundleIconName`), which replaces the system's grey legacy plate with the mark's own cream
  squircle. Sources and the regeneration script are in `docs/branding/`.

- `cuthulhu trace` reports when a large image was reduced to 2048 px for tracing. The reduction
  always happened; it was only ever visible in the desktop.

### Fixed

- **A presets file that cannot be read says what is wrong with it.** A hand-edited
  `presets.json` with its header dropped told the operator `Corrupt("missing or invalid version
  field")` — the sentence the code wrote, wrapped in a struct literal, in quotes — and a
  permission problem told them `Io("Permission denied (os error 13)")`. All five places the
  desktop reads or writes presets sent the same code, so a file this build is too old to read
  looked exactly like a damaged one. Each refusal now says what it means and carries its own
  code, and a file that cannot be read is kept apart from one that cannot be written.

- **A cutter's failures read as sentences, and the app can tell them apart.** A cable pull, a jam,
  a cutter that never answered, and a verb issued at a moment the cutter cannot accept it all
  reached the operator as one code with a Rust value inside the message — `Busy`, or
  `Io("cable pulled")`. Each now says what it means and carries its own code, so the same fault
  reads the same whether the cutter is plugged into this computer or into a Cut Host, and
  `cuthulhu cut` no longer prints `connect: Disconnected` at a terminal.

- **The editor's refusals read as sentences.** Deleting an empty selection, transforming a Node
  the document no longer holds, assigning a material with no id, converting to a machine this
  build does not ship, and a boolean op on shapes that do not overlap each reached the operator as
  a Rust `Debug` rendering — `EmptySelection`, or `Geometry("Degenerate")` for the boolean, which
  wrapped a struct literal around a sentence the geometry layer had already written. Each now says
  what it means.

- **A boolean op over groups or layers says so**, instead of reporting the selected containers as
  missing. The Layers panel selects containers and the toolbar offers Union on any two selections,
  so the refusal was reachable and named the wrong problem.

- **A command on a Node with no parent says so**, rather than calling a Node that is plainly
  present missing. The document root and an orphan out of a manifest — whose topology is not
  validated on load — both reach this. A boolean op over such a Node used to panic instead of
  refusing, since only the first id's parent was ever checked.
