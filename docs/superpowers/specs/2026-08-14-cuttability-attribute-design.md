<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Cuttability as an explicit per-node attribute — design

Date: 2026-08-14
Status: approved — three scoping decisions confirmed 2026-08-14

## Purpose

Issue #68 asked whether cuttability should follow the path or the stroke, and closed with a third
answer taken from the reference applications: **neither — it is an explicit per-object attribute,
defaulted at import.** Fill and stroke go back to being display, and to selecting how passes are
*split*, never whether a shape is cut at all.

Both reference applications agree on the shape and disagree on exactly one detail. Silhouette
Studio assigns a per-object **Cut Style** of `Cut` / `Cut Edge` / `No Cut` and imports SVGs with
none of them set, escapable through `Preferences > Import > SVG > Cut Lines`. Sure Cuts A Lot
assigns a per-shape **Cut Line Type** of `Cut` / `Draw` / `Score` / `Print+Cut Cut` /
`Print+Cut Print` / `Color Layer Alignment`, and defaults to cut. So: an explicit attribute, an
**enum rather than a boolean** — "draw with a pen" and "print, do not cut" are states a boolean
cannot hold — and an import-time default. #68 adopted SCAL's default, because the dominant
complaint in the Silhouette ecosystem is users re-tracing files whose vectors were already fine.

This spec records how that lands in this codebase. #144 tracks it. #45 and #56 each extend the
enum and are written assuming it exists.

## The attribute

`CutLineType` in `document`, a sibling of `Node::style` rather than a member of it — keeping it
off `Style` is the substance of the decision, not a detail of it:

```rust
pub enum CutLineType { Cut, NoCut }
```

Two members ship. The others are named here so nobody invents a parallel enum: `CutEdge` is
#56's, and its geometry semantics are the substantial part of that issue; `Draw`, `Score`,
`PrintCutCut`, `PrintCutPrint` and `ColorLayerAlignment` are #45's, alongside the grouping modes.
Adding a variant later is a one-line change here and a match arm wherever it is consumed, which
is the point of an enum over a bool.

`CONTEXT.md` gains the term, and its ColorPass entry stops being true as written —
"A shape with no stroke belongs to no ColorPass and is not cut" becomes a statement about
`CutLineType`. `docs/superpowers/specs/2026-07-22-editor-shell-design.md:85` ("for a cutting
tool, *stroke = the cut line*") is superseded and should say so rather than be edited silently;
it is a record of what was believed in SP3.

## Migration is not the import default

**The value a new import gets and the value an old project gets are deliberately different.**

A new import gets `Cut`. An existing saved project has no such field, and giving it `Cut` would
make every previously-non-cutting strokeless shape start cutting the next time the file is
opened — a silent change to what a saved document sends to a machine, on real material. Existing
projects instead derive: stroke present with non-zero alpha → `Cut`, otherwise → `NoCut`. A file
saved before this change cuts exactly what it cut before.

`manifest.json` is raw `serde_json::to_string(&Document)` with **no schema version**, so absence
of the field is the only migration signal available. That rules out `#[serde(default)]`, which
cannot distinguish an absent field from an explicit `Cut`, and it rules out a context-free
default, which cannot see the node's stroke.

The mechanism is `#[serde(from = "NodeWire")]` on `Node`: a wire struct whose `cut_line_type` is
`Option<CutLineType>`, and a `From<NodeWire> for Node` that derives from `style.stroke` when it
is `None`. A node's own deserializer already has its `style` in hand, so the derivation is local
and needs no tree walk. `Serialize` stays derived and always writes a concrete value, so a file
round-trips once and is never ambiguous again.

This is deliberately broader than `fileio::load_project`, which is where the legacy-machine-id
migration lives, because a `Node` is also deserialized through `Document::snapshot_json()` and
across IPC. Confining it to `load_project` would leave the other paths to guess.

`// ponytail:` the wire struct exists only for documents written before this change. Once no such
file is expected in the wild, delete `NodeWire` and derive `Deserialize` again.

## What `plan_passes` does

The predicate moves; the ordering established by #139 does not. `plan_passes` already resolves a
shape's outline only *after* deciding the plan includes it, and that decision is a single
expression against `node.style.stroke` (`crates/cutplan/src/passes.rs:92`). It becomes a single
expression against `node.cut_line_type`. Nothing else in the traversal changes — descent still
reads `NodeKind`, and the cycle set and world transforms are untouched.

Two consequences follow, and both are behaviour changes rather than refactors:

**Colour grouping needs an answer for shapes with no stroke**, because they can now be cut. The
rule: group by stroke when it is present with non-zero alpha, otherwise by fill on the same
terms, otherwise by nothing. `ColorPass::color` is already `Option<u32>`, so the last case needs
no type change — but `None` becomes reachable for the first time, and every consumer that renders
a pass swatch or prints a pass header must be checked rather than assumed.

**`DocumentPasses::skipped_no_stroke` is renamed**, because after this change it counts shapes
excluded for a reason that is no longer about stroke. `skipped_not_cut` says what it now means.
It crosses IPC into the desktop, so the DTO and the TypeScript change with it; leaving the old
name would be a lie that survives longer than the code that caused it.

## What gets deleted

`trace::mirror_fill_onto_stroke` (`crates/trace/src/lib.rs:335`, called at `:304`), with its
tests. It exists only because vtracer emits fill-only paths that plan zero passes, and its own
doc comment states the premise this spec removes: that it promotes fill inside `trace` rather
than in `import_svg` or `plan_cut` because "filled but unstroked means do not cut" is a
deliberate distinction downstream. Once cuttability is explicit, that distinction is gone and the
workaround double-applies. Traced output then carries fill only, and groups by fill — which is
what #15 wanted and what the workaround was faking.

Deleting it changes what `design.svg` and the trace CLI emit: a traced path no longer carries an
invented `stroke` attribute. Anything rendering trace output from the stroke needs checking.

## The plain CLI path

`cli::pipeline::doc_from_svg_all_cuttable` gives every imported path a uniform stroke so that a
plain `cuthulhu cut` plans exactly one pass. That overwrite does two jobs at once: it makes the
geometry cuttable, and it collapses everything into a single colour bucket. This spec takes the
first job away, which leaves the overwrite destroying the document's real colours for no purpose
but to manipulate grouping — a worse lie than the one it replaces, because it is now gratuitous.
Left alone it also breaks: with real colours preserved, a file with several fills plans several
passes, silently changing what plain `cuthulhu cut` does.

**Grouping becomes an explicit planning input.** A `Grouping` enum with `ByColor` (today's
behaviour, the default) and `Single`; `plan_passes_with(doc, grouping)` carries it, and
`plan_passes(doc)` keeps its present signature delegating with `Grouping::ByColor`. Only the
plain CLI path passes `Single` — 17 of the 18 current call sites are untouched.

This is the direction #45 already prescribes ("make grouping an explicit planning option"), so
the first mode seeds that design rather than colliding with it; #45 extends the enum with fill,
preset and line-type modes exactly as it extends `CutLineType`.

The rejected alternative is worth recording, because it looks cheaper and is wrong: plan by
colour as usual, then merge the passes afterwards. `plan_passes` emits shapes in document order
*within* each colour group, so merging concatenates colour-by-colour and quietly reorders the cut
inside the resulting pass. `Grouping::Single` emits one pass in true document order for the same
effort.

## Testing

- **Migration is the load-bearing test**: a manifest written without the field, one node stroked
  and one not, loads to `Cut` and `NoCut` respectively — mirroring
  `legacy_machine_ids_migrate_on_load`, which is the precedent this follows.
- **Round-trip stability**: a document saved after this change reloads byte-identically, and
  `snapshot_round_trips_through_json` still holds with the wire struct in place.
- **Planning**: a fill-only shape marked `Cut` plans into a pass keyed on its fill; a stroked
  shape marked `NoCut` plans into none; a shape with neither colour lands in the `None` pass.
- **Regression**: the three ordering tests from #139 keep passing with the predicate swapped —
  they pin *when* the outline resolves, which this must not disturb.
- **Trace**: the mirror's tests are deleted, replaced by one asserting that fill-only trace
  output plans a pass per fill colour without it.
- **Grouping**: `Grouping::Single` over a multi-colour document yields exactly one pass whose
  shapes are in document order — not colour-grouped order, which is the specific way the rejected
  post-hoc merge would have been wrong. A plain `cuthulhu cut` over multi-fill art still plans
  one pass, which is the behaviour this guards.
- **The e2e fake mirrors `Document::snapshot_json()`**, so `apps/desktop/ui/e2e/smoke.spec.ts`
  changes with the JSON shape or the suite lies.

## Scoping decisions

Three questions that changed scope materially, settled 2026-08-14.

1. **The import default ships hardcoded to `Cut`; configurability moves to #54.** Moves, not
   drops — but a deferral only holds if the receiving issue records it, and #54 was filed
   2026-07-26, three weeks before #68 decided cuttability was an attribute at all. Its
   import-settings criterion named placement defaults and default fill/stroke style, nothing
   about a cut-line-type default. **#54 has been given an acceptance criterion for it and #144
   amended to put the preference out of its scope** — without both edits, this paragraph would be
   the only surviving record that the capability was ever promised.

   Deferred rather than built here because there is no general preferences store — `presets.json`
   and `hosts.json` are per-feature files, and a third ad-hoc file for one setting is how that
   problem grows. The pressure is also low: the complaint Silhouette's escape hatch exists to
   solve is "nothing cuts on import", which a `Cut` default already answers, and a user wanting
   the opposite can select-all and set `NoCut`.

   Note for whoever builds #54: its "default fill/stroke style" preference is unaffected in
   wording but narrower in meaning after this change — fill and stroke are paint and grouping
   keys only, and no longer decide whether anything cuts.
2. **The UI surface is a control on the existing properties panel, and nothing more.** The cut
   dialog's "not cut: N" readout should say *why* alongside it. Richer surfaces — grouping mode
   pickers, per-layer roles — are #45's.
3. **Grouping becomes an explicit planning input**, per the plain-CLI-path section above.

## Out of scope

- `CutEdge` geometry (#56) and the production roles plus grouping modes (#45). This spec defines
  the enum they extend and nothing they own.
- Print-and-cut (#25), which is what makes the `PrintCut*` members worth having.
- Changing the ordering `plan_passes` established in #139. The predicate swaps; the sequence does
  not.
