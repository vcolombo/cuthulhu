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

- **A refused Cut press no longer erases state another press or cutter still needs, and quitting
  warns about every remote cut this desktop started.** Dispatch attempts now own their own
  window-guard marks, while retry ids remain grouped by the wire id they were sent under. A
  refusal or lost connection can therefore retract only its own attempt. The close guard is no
  longer scoped to the aimed cutter, and Cut Host snapshots carry the admitted-before-worker
  state that ordinary status cannot see, so neither a poll nor a host forget can call a Job free
  during that gap. Quitting stops local motion and leaves host-owned Jobs running, as the prompt
  now says.

- **A Cut Host that refuses a cut says so, instead of reporting one that may be running.** Every
  refusal a host sent — a cut off the bed, the wrong cutter attached, a cutter already busy —
  reached the operator as `dispatch_unconfirmed` with "the Job may already be cutting there. Press
  Cut again", because the test for whether anything was outstanding asked only whether the host had
  been reached, and a refusal is a host that was reached. A mis-scaled document on a first press was
  told its Job might be cutting and to press the button that starts a blade again. A refusal now
  keeps its own code and its own sentence; a press made while an earlier dispatch really is
  unsettled still warns.

- **An unconfirmed dispatch no longer promises that pressing Cut again cannot cut twice.** It said
  "the host recognizes the same Job and will not cut it twice", which nothing on this side can
  know: a host forgets an accepted id past its retention and past its capacity cap, and the
  desktop's record of it is process-local, so a restart between the two presses leaves no id to
  reuse. It now says what is true — the Job may be cutting, only the cutter can tell you, and the
  retry goes out under the same id, which is what lets a host that still remembers it read it as
  this Job rather than starting a second one. Where the failure cleared that id, it says the next
  press is a new Job rather than offering a retry it cannot make.

- **A Cut Host's acceptance has to name the dispatch it is answering.** `Accepted` carries the
  dispatch id the host is answering about, and the desktop discarded it — so a reply naming some
  other Job was read as this one's acceptance: the operator was told their cut had started, and the
  record a real retry needs was dropped as settled. That id is the whole of the protection against
  cutting twice, and it was the one field that could prove an answer belonged to this dispatch,
  since every other correlation on the connection is structural. A reply that names a different
  dispatch is now refused as a host answering outside the protocol, which drops the connection and
  leaves the dispatch unconfirmed under the id it went out with — reachable only from a peer that
  is not this daemon, which is also the only kind that could send one.

- **A Cut Host that answers with the wrong reply names it, instead of calling itself unreachable.**
  A reply arriving where the request could not use it told the operator `the host could not be
  reached (the host answered with Devices([DeviceInfo { instance_id: "usb:1:4", machine_id:
  "cameo5", transport: Usb { locator: "1:4" }, candidate: false, host: None }]))` — every field of
  every cutter that host knows, rendered with `Debug`, inside a sentence blaming a network that had
  just carried the answer. The reply and the one that was owed are now named instead, and the
  failure carries its own code rather than the one that means a Pi is off, so a host that answered
  is no longer reported as one that never did. A dispatch answered this way still counts as
  unconfirmed, and still keeps the id it went out under, so pressing Cut again is the retry a host
  that deduplicates can recognise rather than a second Job it has never seen.

- **A `cuthulhu cut` on an SVG it cannot import says why.** A truncated file told the operator
  `SVG parse: Parse("SVG data parsing failed cause the root node was opened but never closed")`
  — the parser's own account of the problem, wrapped in a struct literal, in quotes, behind a
  verb that repeats the sentence following it. The desktop has printed the sentence for this
  exact failure since project files were versioned; the CLI was the one caller still handing
  over the Rust value. "Why" rather than "what is wrong with it", because a file can also be
  turned away for stating its own size as zero, or for being written in UTF-16, and neither of
  those is a file with anything wrong with it.

- **A presets file that cannot be read says what is wrong with it.** A hand-edited
  `presets.json` with its header dropped told the operator `Corrupt("missing or invalid version
  field")` — the sentence the code wrote, wrapped in a struct literal, in quotes — and a
  permission problem told them `Io("Permission denied (os error 13)")`. All five places the
  desktop reads or writes presets also sent one code, `preset_error`, so nothing could tell a
  file this build is too old to read from a damaged one except by reading the Rust value in the
  message. Each refusal now says what it means and carries its own code, and a file that cannot
  be read is kept apart from one that cannot be written.

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
