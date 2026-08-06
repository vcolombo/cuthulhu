<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Changelog

Notable changes to Cuthulhu. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Nothing has been released yet, so there is no history before `Unreleased`.

## [Unreleased]

### Changed

- `cuthulhu trace --detail` is now stated in the same units as the desktop's Detail slider: **higher
  means more detail**. It previously carried vtracer's `length_threshold` verbatim, which runs the
  other way, so the two interfaces used one word for opposite things and printed opposite advice for
  the same failure. The default is unchanged in effect — the old `4.0` is the new `9.5`. A script
  passing `--detail` explicitly will trace differently and should be updated by reflecting its value
  through the range: `13.5 - old`.

### Added

- The desktop shell has an application icon. macOS and Windows bundles now get a real `.icns` and
  `.ico`, where `bundle.icon` previously listed only a 32 px placeholder PNG. Both containers are
  multi-resolution and carry two artworks: the kraken mascot at 27 px and above, and a C mark below
  it, since the mascot's knife and tentacle curls stop resolving at small sizes. Sources and the
  regeneration script are in `docs/branding/`.

- `cuthulhu trace` reports when a large image was reduced to 2048 px for tracing. The reduction
  always happened; it was only ever visible in the desktop.
