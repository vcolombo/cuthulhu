<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Pass grouping as an explicit choice — design

Date: 2026-08-16
Status: approved — four scoping decisions confirmed 2026-08-16

## Purpose

Cut planning groups passes one way and cannot be asked for another: `plan_passes` keys a pass on
the shape's visible stroke, else its visible fill (`crates/cutplan/src/passes.rs:75-77`), with a
single escape hatch — `Grouping::Single`, one pass over everything, which is what a bare
`cuthulhu cut` means (`passes.rs:87-88`). Issue #45 asks for four grouping keys: stroke colour,
fill colour, layer/material preset, and line type.

Two of those four are not colours, and **a pass is identified by a colour everywhere**:
`ColorPass::color` (`passes.rs:20-21`) → `PassSelection::color` (`crates/cutplan/src/plan.rs:15-22`)
→ `PlannedPass::color` (`plan.rs:40-44`) → `CutError::UnknownPassColor` (`plan.rs:60-64`) →
`CutRequest`/`ConfiguredPassDto` (`apps/desktop/src/device.rs:51-68`) →
`PlanCutResponse` (`apps/desktop/ui/src/ipc.ts:148-158`) → `PassVm` and the travel/cut request
builders (`apps/desktop/ui/src/cut/viewmodel.ts:8-38,135-169,224-248`) → the dialog's row keys and
swatches (`apps/desktop/ui/src/cut/CutDialog.tsx:181-198,554-618`) → the preview
(`CutPreview.tsx:75-83,118-130`) → the e2e fake (`ui/e2e/smoke.spec.ts:275,302-337,473-505`) →
the CLI's `--skip-color` and `--order` (`crates/cli/src/main.rs:45-54`).

So this cannot be added as a planner flag. Pass identity stops being a colour, once, and the four
modes arrive on top of it. **#148 tracks this slice**; #45 stays open as the parity umbrella, and
its remaining criteria are #149 (one continuous job versus separate confirmed jobs) and #150
(colour-layer alignment marks), both listed under *Out of scope* below.

## What the reference applications do

Both offer colour grouping *on top of* a per-object production attribute, never as a substitute
for one — which is the same conclusion #68 reached for cuttability.

**Silhouette Studio** offers cut by line colour and cut by fill colour as distinct selectors, so a
design's fill and its outline are two different ways to split the same artwork into passes.
**Sure Cuts A Lot** groups by its per-shape/layer `Cut Line Type`, and its cut window filters by
which tool is loaded; colour grouping sits above that. Neither derives *whether* a shape cuts from
the grouping key — #68 settled that for this codebase, and #144 implemented it.

Sources: [cut by line colour or fill colour](https://www.silhouette101.com/archives/cut-by-line-color-or-fill-color-basic-edition-and-higher),
[cut line types](https://www.thepinningmama.com/cut-lines-explanation-and-types-in-silhouette-studio-silhouette-bootcamp-lesson-25/),
[Cut Line Type in SCAL Pro](https://www.craftedge.com/tutorials/cutlinetype/cutlinetype.php),
[SCAL cut settings](https://www.craftedge.com/help/scalbridge/cutwindow_general.php).
Vendor-doc and community sourced; not verified by driving the applications.

## The key

`PassKey`, in `cutplan::passes` beside the type it keys:

```rust
pub enum PassKey {
    All,
    Color(Option<u32>),
    LineType(CutLineType),
    Preset(Option<String>),
}
```

`Option` inside two variants rather than one shared `Unassigned`: absence is a property of the
mode's key, not a fifth kind of pass. `Color(None)` is a shape with no visible paint at all;
`Preset(None)` is a shape no operator has assigned a material to — which, unlike the colour case,
is the *usual* state and the one most passes will carry.

**`All` is not `Color(None)`.** Today `Grouping::Single` keys its one pass `None`, and both
`passes.rs:14-19` and `plan.rs:77-81` carry prose apologising for it — the same value means "one
pass by request, whatever its paint" in one place and "the pass of shapes with no visible paint"
in the other, so `CutError`'s message for a missing selection has to be the evasive "no planned
pass without a color". Generalising the key is the one moment that costs nothing to fix.

`ColorPass` is renamed **`DocumentPass`**: its container is already `DocumentPasses`
(`passes.rs:30-38`), so the singular is free, and every alternative collides with vocabulary
`CONTEXT.md` already spends — `Pass` is a run of the blade, `CutPass` is `driver-core`'s,
`PlannedPass` is the post-preflight one, and "group"/"batch" are on ColorPass's own `_Avoid_` list.
`CONTEXT.md`'s ColorPass entry becomes the DocumentPass entry and gains PassKey; the ColorPass
name is retired, not aliased.

The rest of the rename follows mechanically: `PassSelection { key, settings }`,
`PlannedPass { key, job }`, and `CutError::UnknownPass(PassKey)` whose `code()` becomes
`unknown_pass`. That code crosses IPC but nothing reads it: the frontend keys off `stale_plan`
alone (`CutDialog.tsx:327,376`), as `device.rs:1095-1097` states. The assertions on
`"unknown_pass_color"` in `device.rs:1331-1340,1403-1409` and `plan.rs:213-216,297-303` are the
whole cost.

## One grammar for a key, in every language

`PassKey` has a canonical string form, and it is the only form that crosses a boundary:

| Key | String |
|---|---|
| `All` | `all` |
| `Color(Some(0xFF0000FF))` | `color:ff0000ff` |
| `Color(None)` | `color:none` |
| `LineType(Cut)` | `line-type:cut` |
| `LineType(NoCut)` | `line-type:no-cut` |
| `Preset(Some("cameo5-htv"))` | `preset:cameo5-htv` |
| `Preset(None)` | `preset:none` |

`Display` and `FromStr` in Rust, plus `#[serde(into = "String", try_from = "String")]` so the JSON
a DTO carries *is* that string. Three things follow, and they are the reason for choosing a string
over a serde-tagged enum:

- **No hand-mirrored union in TypeScript.** A tagged enum would need a discriminated union written
  by hand in `ipc.ts` and again in the e2e fake — the duplication #70 is open about.
- **The dialog's row key is the value.** Today it is `color ?? 'none'` (`CutDialog.tsx:554-618`),
  a second encoding invented in the UI; with a canonical string there is nothing to invent.
- **The CLI and the wire agree by construction.** `--skip-pass color:ff0000ff` and a DTO field
  hold identical bytes, so a pass named in a script and a pass named in the dialog cannot drift.

Parsing splits on the **first** `:` only. Preset ids are kebab-case and machine-scoped
(`presets.rs:53-167`), and the grammar tolerates a colon inside one rather than forbidding it.
Colour is hex RRGGBBAA, the form `--order` already takes; parsed case-insensitively and always
written lowercase, so a round trip is a fixed point and two spellings of one key cannot both
appear in a pass list.

TypeScript gets one `parsePassKey` — needed for the swatch, which must recover the RGBA the string
carries — table-tested against the same examples as the Rust round-trip test. That table is the
cross-language pin, and both halves must list every variant.

## Grouping

```rust
pub enum Grouping { Single, Color, Stroke, Fill, LineType, Preset }
```

`Color` keeps today's stroke-else-fill rule verbatim and stays what `plan_passes` defaults to
(`passes.rs:95-97`), so no existing caller changes behaviour. `Stroke` and `Fill` are strict: a
shape with no visible stroke under `Stroke` keys `Color(None)`, the same bucket a shape with no
paint at all lands in.

`Color` earns its place beside the two strict modes rather than being their union. #144 introduced
the fallback deliberately (`passes.rs:67-77`): fill-only art is the common case after a trace or an
Illustrator export, and under strict `Stroke` all of it collapses into one unrecognisable
colourless pass. A default that leaves an operator unable to tell their passes apart is the
behaviour the fallback exists to prevent.

`LineType` **ships with one reachable key.** `CutLineType` is `{Cut, NoCut}`
(`crates/document/src/node.rs:41-42`) and `NoCut` shapes never reach a pass — they are counted
into `skipped_not_cut` (`passes.rs:129-131`) — so a line-type plan holds exactly one pass, keyed
`line-type:cut`, until the enum gains a second cuttable member. This is deliberate and recorded
rather than hidden: the mode, the key, the DTOs, the dialog control and the CLI value all become
useful the moment #56 adds `CutEdge`, with no further change to any of them. Adding `Draw` or
`Score` here instead was rejected — no driver can honour a pen or a scoring tool (#57), so a
`Draw` pass would encode as ordinary blade movement and cut the material the operator asked it to
draw on.

`Single` is unchanged in behaviour and keys `All`.

## A material preset attaches to a Node, and inherits

`Node` gains `material_preset: Option<String>` — a `MaterialPreset::id` (`presets.rs:9-15`) —
beside `cut_line_type`, and for the same reason it is not on `Style`: production intent is not
paint (#68).

`NodeWire` gains the field with `#[serde(default)]`. Unlike `cut_line_type` there is nothing to
derive: a document written before this change had no way to assign a preset, so absent genuinely
means `None`, and the migration is the default. Adding a field changes every document's serialized
JSON and therefore its `doc_revision` (`passes.rs:61-65`), exactly as #144 did — harmless, because
a revision is only ever compared against one taken in the same process run.

**Inheritance happens in the walk, not in the document.** `plan_passes_with` already carries a
world transform down its explicit stack (`passes.rs:106-160`); the nearest ancestor's preset rides
alongside it, and a shape's own value overrides. So assigning a preset to a Layer covers every
shape under it — #45's "object/layer" criterion with one field, no second attribute, and no stored
derived state that could go stale when a node is reparented.

**The planner does not validate the id.** Presets are machine-scoped and user entries can be
deleted or shadowed (`presets.rs:170-210`), so an id that resolves to nothing is a real state, not
a corruption. It keys a pass; the dialog renders it unresolved and falls back to the operator's
settings. Refusing a cut inside the planner over a settings *lookup* would put a preset-file
concern behind `plan_cut`, which exists to refuse geometry and machine mismatches.

Setting it is `set_material_preset(ids, Option<String>)`, mirroring `set_cut_line_type`
(`apps/desktop/src/ipc.rs:53-57`) — one method on `state.rs`, a thin command, a properties-panel
control beside the cuttability one.

## The mode travels in the payload

Three call sites plan independently: `plan_cut_response` (`device.rs:1125-1134`),
`travel_for_order` (`device.rs:1155-1180`), and `prepare_cut` (`device.rs:850-873`). They are three
separate IPC round trips, and if they disagree about the grouping, the travel preview shows one
arrangement while the cut executes another — silently, because every pass key still matches
something.

So the mode is a parameter of each: `plan_cut(grouping)`, `travel_for_order(docRevision, grouping,
passes)`, and `CutRequest::grouping`. **Not** stored in `AppState`: a stored mode can be changed
between the plan and the cut, and the stale-plan check guards the *document*, not the mode. Passing
it makes the request self-describing, which is what makes a mismatch impossible rather than
unlikely.

## The CLI says pass, not colour

Clean cutover, no aliases:

| Today | After |
|---|---|
| `--by-color` | `--group-by <single\|color\|stroke\|fill\|line-type\|preset>`, default `single` |
| `--skip-color ff0000ff` | `--skip-pass color:ff0000ff` |
| `--order ff0000ff,0000ffff` | `--order color:ff0000ff,color:0000ffff` |

Default `single` preserves what a bare `cuthulhu cut` does today (`cli/src/pipeline.rs:128-141`),
and `--group-by color` is exactly the old `--by-color`. `check_color_flag_scope` becomes
`check_pass_flag_scope` and keeps refusing rather than ignoring (`pipeline.rs:178-196`): under
`--group-by single` there is one pass, so there is nothing to skip and nothing to order. Its two
messages, and `check_interactive`'s (`pipeline.rs:198-205`), stop naming `--by-color`.

Requiring the `color:` prefix on `--order` values is a breaking change for a script, and is the
point: one grammar, no dialect where a bare hex string means a colour key only because colours
happened to come first.

A key from a mode other than the one selected — `--group-by fill --skip-pass line-type:cut` — is
not a special case and needs no rule of its own: it names no planned pass, so `UnknownPass` refuses
it by name, which is what already happens to a colour that is not in the document.

## Verification

Everything here is decided before a byte reaches a Transport, so nothing is added to
`apps/desktop/MANUAL-CHECKLIST.md`.

- **Planner** — one document with two strokes, two fills, a `NoCut` shape and a layer-assigned
  preset, planned through all six modes with the expected key set for each; `PassKey` round trip
  over every variant; preset inheritance (layer covers children, shape overrides, absent stays
  `Preset(None)`).
- **Selection** — `UnknownPass` refused rather than dropped for each key kind, and its sentence per
  variant, extending the table test at `plan.rs:290-305`.
- **Document** — a node saved before the field loads with `material_preset: None`, through a real
  `.cut` file, as `fileio`'s cuttability migration test does.
- **Desktop** — a grouping chosen in the dialog reaches all three planner call sites; an
  unresolved preset renders as a row; `parsePassKey` against the Rust table.
- **CLI** — `--group-by` accepts every mode, `--skip-pass`/`--order` are refused under `single`,
  and a pass key that names no pass is refused by name.
- **e2e** — the fake's `planFromDoc`, travel and cut mirror keys and the grouping argument
  (`smoke.spec.ts:275,302-337,473-505`). #143's stale `travel_for_order` comment is fixed in the
  same change; it was filed as a ride-along for exactly this.

## Rejected alternatives

**A colour-keyed slice first, generalising the key later.** Smaller, and it would have shipped
`ByStroke`/`ByFill` without touching `PassSelection`. Rejected because the rename cascade is the
expensive half either way, and doing it second means two passes over the same ten files, two sets
of DTO churn, and an interim vocabulary in `CONTEXT.md` that is wrong the moment preset grouping
lands.

**A tagged enum on the wire.** Structurally honest, and it forces a hand-written discriminated
union into `ipc.ts` plus the e2e fake — the duplication #70 already tracks — while the dialog still
needs a flat string for a row key.

**Grouping stored in `AppState`.** One fewer parameter on three commands, at the cost of a mode
that can change between plan and cut with nothing to detect it.

**A second per-node enum for #45's "production role".** #68 settled that the role *is*
`CutLineType`; a parallel attribute would be two answers to one question. Recorded here because
#45's text still reads as if it were separate.

## Out of scope

The first two are filed; the rest are named so nobody builds half of one here. None is blocked by
anything in this slice.

- **One continuous job versus separate confirmed jobs** (#149). The per-pass completion policy
  already exists in `DeviceManager`; making it a per-plan choice is a `driver-core` question, not a
  grouping one.
- **Colour-layer alignment marks in every enabled pass** (#150). Needs registration marks, which
  #25 owns.
- **`CutEdge`** (#56), and `Draw`/`Score`/`PrintCutCut`/`PrintCutPrint`/`ColorLayerAlignment`
  members — each needs a driver answer for the tool it implies.
- **A configurable import default** for either attribute (#54).
