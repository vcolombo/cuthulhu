<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Pass grouping as an explicit choice — design

Date: 2026-08-16
Status: approved — four scoping decisions confirmed 2026-08-16; revised the same day after a Codex
review returned BLOCKING on fourteen findings, all accepted (see *Revisions*).

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
swatches (`CutDialog.tsx:181-198,554-618`) → the preview (`CutPreview.tsx:19-25,75-83,118-130`) →
the e2e fake (`ui/e2e/smoke.spec.ts:275,302-341,473-505`) → the CLI's `--skip-color` and `--order`
(`crates/cli/src/main.rs:45-54`).

So this cannot be added as a planner flag. Pass identity stops being a colour, once, and the modes
arrive on top of it: **five in total** — the two `Grouping` already has (today's stroke-else-fill
default, and the single pass) plus strict stroke, strict fill, and material preset. Line type is
*not* among them; see *Line type is not a mode yet*. **#148 tracks this slice**; #45 stays open as
the parity umbrella, and its remaining criteria are #149 (one continuous job versus separate
confirmed jobs) and #150 (colour-layer alignment marks).

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
Vendor-doc and community sourced; not verified by driving the applications. **Nothing in this spec
rests on how either application inherits a material down a layer** — that behaviour is unverified
and is not cited as support for the inheritance model below.

## The key

`PassKey`, in `cutplan::pass_key` beside the type it keys:

```rust
pub enum PassKey {
    All,
    Color(Option<u32>),
    Preset(Option<String>),
}
```

`Option` inside two variants rather than one shared `Unassigned`: absence is a property of the
mode's key, not a fifth kind of pass. `Color(None)` is a shape with no visible paint *in the mode's
terms* — no visible stroke under `Stroke`, no visible fill under `Fill` — and `Preset(None)` is a
shape that resolves to no material, which is the *usual* state and the one most passes carry.

**`All` is not `Color(None)`.** Today `Grouping::Single` keys its one pass `None`, and both
`passes.rs:14-19` and `plan.rs:77-81` carry prose apologising for it — the same value means "one
pass by request, whatever its paint" in one place and "the pass of shapes with no visible paint" in
the other, so `CutError`'s message for a missing selection has to be the evasive "no planned pass
without a color". Generalising the key is the one moment that costs nothing to fix.

`ColorPass` is renamed **`DocumentPass`**: its container is already `DocumentPasses`
(`passes.rs:30-38`), so the singular is free, and every alternative collides with vocabulary
`CONTEXT.md` already spends — `Pass` is a run of the blade, `CutPass` is `driver-core`'s,
`PlannedPass` is the post-preflight one, and "group"/"batch" are on ColorPass's own `_Avoid_` list.
`CONTEXT.md`'s ColorPass entry becomes the DocumentPass entry and gains PassKey; the ColorPass name
is retired, not aliased.

The rest of the rename follows mechanically: `PassSelection { key, settings }`,
`PlannedPass { key, job }`, and `CutError::UnknownPass(PassKey)` whose `code()` becomes
`unknown_pass`. That code crosses IPC but nothing reads it: the frontend keys off `stale_plan`
alone (`CutDialog.tsx:327,376`), as `device.rs:1095-1097` states. The assertions on
`"unknown_pass_color"` in `device.rs:1331-1340,1403-1409` and `plan.rs:213-216,292-303` are the
whole cost.

## One grammar for a key, in every language

`PassKey` has a canonical string form, and it is the only form that crosses a boundary:

| Key | String |
|---|---|
| `All` | `all` |
| `Color(Some(0xFF0000FF))` | `color:ff0000ff` |
| `Color(None)` | `no-color` |
| `Preset(Some("cameo5-htv"))` | `preset:cameo5-htv` |
| `Preset(None)` | `no-preset` |

**Absence is its own token, never a reserved value inside a mode's namespace.** This is the whole
of the grammar's first rule, and it exists because the obvious spelling is broken: a preset id is
an unrestricted operator-supplied string (`crates/cutplan/src/presets.rs:9-15`, written through
`save_user_presets`), so `preset:none` for "no preset" would collide with a preset whose id is
literally `none` — two distinct passes writing one string, which means duplicate React keys, and a
`plan_mismatch` from `travel_for_order`'s exact-once check that no operator could clear. `no-color`
is spelled the same way for one rule rather than two, even though an 8-hex-digit colour could not
have collided.

A colour is always 8 hex digits, parsed case-insensitively and written lowercase, so the round trip
is a fixed point and two spellings of one key cannot both appear in a pass list.

**`preset:` parses, as an empty id.** The first draft refused it, reasoning that an empty tail was
ambiguous — true when absence was spelled `preset:none`, and false once absence became its own
token. Refusing it made the grammar *non-total*: `Display` writes `preset:` for
`Preset(Some(String::new()))`, which is constructible, so serde could emit a string its own parser
rejected. Worse, the two languages disagreed about it, and that disagreement fails open: the
TypeScript mirror dropped the id, the request named no preset, and `prepare_cut` skipped its lookup
and produced default speed and force where deserialization had previously refused the request.

An empty id is therefore *parsed* everywhere and *rejected everywhere it could mean something*:
`commands::set_material_preset` refuses to assign one, `presets::load_presets` drops a file entry
that carries one, and a pass that still reaches `prepare_cut` with one is refused as
`unknown_preset` like any other id that resolves to nothing.

The representation is `Display`/`FromStr` plus `#[serde(into = "String", try_from = "String")]`, so
the JSON a DTO carries *is* that string. Three things follow, and they are the reason for choosing
a string over a serde-tagged enum:

- **No hand-mirrored union in TypeScript.** A tagged enum would need a discriminated union written
  by hand in `ipc.ts` and again in the e2e fake — the duplication #70 is open about.
- **The dialog's row key is the value.** Today it is `color ?? 'none'` (`CutDialog.tsx:554-618`),
  a second encoding invented in the UI; with a canonical string there is nothing to invent.
- **The CLI and the wire agree by construction.** `--skip-pass color:ff0000ff` and a DTO field
  hold identical bytes, so a pass named in a script and a pass named in the dialog cannot drift.

Parsing splits on the **first** `:` only, so a preset id may contain one. TypeScript gets one
`parsePassKey` — needed for the swatch, which must recover the RGBA the string carries —
table-tested against the same examples as the Rust round-trip test. That table is the
cross-language pin, and both halves must list every variant.

### Verified, not assumed

Both wire mechanisms were prototyped against the locked versions — `serde 1.0.229`,
`serde_json 1.0.151` — before this spec was approved, so the plan states these rather than hoping
for them:

- `#[serde(into = "String", try_from = "String")]` on an enum emits exactly the `Display` string
  and accepts it back, nested in a struct field and in a `Vec`. It requires `Clone` on the type,
  which `PassKey` needs anyway for its `String`.
- The grammar is **injective**: nine keys including `Preset(None)`, `Preset(Some("none"))`,
  `Preset(Some("no-preset"))`, `Preset(Some("all"))`, an id containing `:` and an id containing `,`
  all write distinct strings and round-trip. This is the property the first spelling lacked.
- Malformed keys are `serde` errors, not panics: `""`, `"color:"`, `"color:zz"`, `"color:ff0000"`,
  `"color:none"`, `"line-type:cut"`, `"no-material"` and `"all:1"` all deserialize to `Err`.
  `"preset:"` is *not* among them — see *One grammar*: it parses as an empty id, and the ways an
  empty id could mean something are closed where they live.
- `PresetAssignment` (below) serializes as `{"state":"inherit"}`, `{"state":"unassigned"}` and
  `{"state":"preset","id":"cameo5-htv"}`, `#[derive(Default)]` with `#[default]` on `Inherit`
  works, and both an absent field and an explicit `null` decode to `Inherit` while `Serialize`
  always writes a concrete value.
- A wire struct carries **two** absent-field rules at once: `cut_line_type` deriving from the
  stroke and `material_preset` defaulting to `Inherit`.

## Grouping

```rust
pub enum Grouping { Single, Color, Stroke, Fill, Preset }
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

Because `Color(None)` means something different in each colour mode, **the pass's label is
grouping-aware**: "No visible paint" under `Color`, "No visible stroke" under `Stroke`, "No visible
fill" under `Fill`. A shape with a bright fill and no stroke is in the `no-color` pass under
`Stroke`, and calling that "no visible paint" in front of the operator would be false.

`Single`'s one pass is keyed `All` and labelled **"Every cut shape"** — not "every shape", because
a `NoCut` shape is excluded from it (`passes.rs:129-131`) and counted into `skipped_not_cut`.

## Line type is not a mode yet

`CutLineType` is `{Cut, NoCut}` (`crates/document/src/node.rs:41-42`) and a `NoCut` shape never
reaches a pass — it is counted, not cut (`passes.rs:129-131`). A line-type grouping over today's
enum therefore produces exactly one pass containing exactly what `Single` produces.

An earlier revision of this spec shipped it anyway, keyed `line-type:cut`, on the argument that the
plumbing would be ready for #56. That was wrong, and the Codex review named why: the mode would be
**observably identical to `Single` while carrying different `--skip-pass`/`--order` semantics and a
different label** — a second way to say one thing, which is the failure the repo's no-alias
convention exists to prevent. An operator choosing between two modes that cut the same geometry
learns nothing from the choice.

So `Grouping::LineType` and `PassKey::LineType` are **not in this slice**. #56 adds `CutEdge`, and
adds both of them with it — additively, as a variant plus its match arms, with no rename and no
second wire migration. Adding `Draw`/`Score` here instead was rejected for a separate reason: no
driver can honour a pen or scoring tool (#57), so a `Draw` pass would encode as ordinary blade
movement and cut the material the operator asked it to draw on.

## A material preset attaches to a Node, and resolves down the tree

`Node` gains `material_preset: PresetAssignment` beside `cut_line_type`, and for the same reason it
is not on `Style`: production intent is not paint (#68).

```rust
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "state", content = "id", rename_all = "kebab-case")]
pub enum PresetAssignment {
    #[default]
    Inherit,
    Unassigned,
    Preset(String),
}
```

**Three states, not `Option<String>`.** The two-state spelling cannot say "deliberately no
material, do not inherit": with `None` meaning inherit, a shape inside an HTV Layer can never reach
the `no-preset` pass, even though that pass is a legitimate state which resolves to the operator's
own settings (`crates/cutplan/src/presets.rs:31-49`). The mechanism costs one enum, one serde
attribute, one arm in the walk and one option in a control; the alternative costs a second
migration of `Node`'s wire format across Rust, JSON, TypeScript, the e2e fake and that control —
and `manifest.json` is a bare `serde_json::to_string(&Document)` with no schema version, so every
such migration is paid in "absence is the only signal" logic that never goes away. This is the
Codex review's verdict, adopted; what would overturn it is a product invariant that every cuttable
descendant of a preset-bearing Layer must share some preset, which nothing in #45 or #68
establishes.

`NodeWire` gains `#[serde(default)] material_preset: Option<PresetAssignment>` and resolves it with
`unwrap_or_default()`, so an absent field — and an explicit `null` — mean `Inherit`. Adding a field
changes every document's serialized JSON and therefore its `doc_revision` (`passes.rs:61-65`),
exactly as #144 did — harmless, because a revision is only ever compared against one taken in the
same process run.

**Resolution happens in the walk; assignment does not descend.** `plan_passes_with` already carries
a world transform down its explicit stack (`passes.rs:106-160`); the nearest ancestor's assignment
rides alongside it, and a shape's own `Preset`/`Unassigned` wins over it. `set_material_preset`
therefore writes **only the Nodes in the selection** — never their children.

That is the opposite of `set_cut_line_type`, which descends (`crates/document/src/commands.rs:45-81`),
and the difference is the point: `cut_line_type` does not inherit, so a value on a Group would be
inert and descending is the only way to make the control mean anything. A preset *does* inherit, so
descending would write today's shapes and leave the Layer itself unset — after which a shape added
to or reparented into that Layer would disagree with its siblings, and the Layer would look
assigned while holding nothing. Copying the neighbouring command would have cancelled out the
inheritance it feeds.

**The planner does not validate the id.** Presets are machine-scoped and a user entry can be
deleted or shadowed (`presets.rs:170-210`), so an id that resolves to nothing is a real state, not
a corruption. It keys a pass, and the dialog renders it unresolved. Refusing a cut inside the
planner over a settings *lookup* would put a preset-file concern behind `plan_cut`, which exists to
refuse geometry and machine mismatches.

**`prepare_cut` refuses it, though** — that is the boundary where the preset file is actually read,
and it is machine-scoped there too, so an id belonging to another cutter is refused like one that
was deleted (`unknown_preset`). An earlier draft of this spec had it fall back to the
override-or-default path; that was wrong, and cutting real material with settings unrelated to the
pass's own name is what the silence cost.

### What the panel shows

The properties panel reports the selection's **local assignment**, and the effective value it
resolves to, as named states — extending the explicit `"mixed"` the cuttability control already
uses (`apps/desktop/ui/src/panels/cutLineType.ts:6-27`):

| Situation | State |
|---|---|
| `Inherit`, ancestor's id resolves | `Inherited — <preset name>` |
| `Inherit`, ancestor's id resolves to nothing | `Inherited — Unresolved (<id>)` |
| `Inherit`, no ancestor assignment | `Inherited — No preset` |
| `Unassigned` | `No preset` |
| `Preset(id)` | the preset's name, or `Unresolved (<id>)` |
| selection disagrees on local assignment | `Mixed` |

The control offers `Inherit`, `No preset`, and each preset by name. A Layer holding an explicit
preset selected together with an inheriting child reads `Mixed` even though both resolve to the
same id, because the command edits local assignments and saying otherwise would misreport what a
click is about to overwrite.

## A preset-keyed pass cuts with that preset's settings

`prepare_cut` resolves a pass's `Settings` from `ConfiguredPassDto.preset_id` alone
(`apps/desktop/src/device.rs:840-860`). So a pass keyed `preset:cameo5-htv` whose row carries
`presetId: null` is cut with defaults — the operator groups by material and then gets none of that
material's settings unless they re-select it by hand, once per pass. That is the feature failing at
the only thing it exists for.

The dialog therefore **initialises a preset-keyed row's `presetId` from its key**. An id that
resolves to nothing stays on the row rather than being cleared: the request carries it and the row
shows it unresolved. **`prepare_cut` then refuses the cut** (`unknown_preset`) rather than falling
back to the override-or-default path — the first draft said it should fall back, and that was
wrong. The operator asked for that material's speed and force; a machine-scoped preset disappears
for ordinary reasons (the project was converted, the entry was deleted), and cutting real material
with settings unrelated to the pass's own name is what the silence cost. Every other key kind still
starts with no preset, because no key kind other than `Preset` names one.

## The mode travels in the payload, with the rows it produced

Three call sites plan independently: `plan_cut_response` (`device.rs:1125-1134`),
`travel_for_order` (`device.rs:1155-1180`), and `prepare_cut` (`device.rs:850-873`). They are three
separate IPC round trips, and if they disagree about the grouping, the travel preview shows one
arrangement while the cut executes another — silently, because every pass key still matches
something.

So the mode is a parameter of each: `plan_cut(grouping)`, `travel_for_order(docRevision, grouping,
passes)`, and `CutRequest::grouping`. **Not** stored in `AppState`: a stored mode can be changed
between the plan and the cut, and the stale-plan check guards the *document*, not the mode.

Putting the mode in the payload is necessary and not sufficient. The dialog holds the mode, the
rows and the plan revision as **one installed-plan state**, replaced atomically when a plan lands.
Today they are three independent `useState` values (`CutDialog.tsx:105-109`) read separately by
`startCut` (`:314-316`) and `refreshTravel` (`:363-368`); leaving them that way means a mode change
makes the *old* rows sendable under the *new* mode for as long as the replan is in flight, and
where the two key sets overlap the backend accepts them and cuts the wrong shapes. So: changing the
mode clears travel, disables Cut and the row controls, and only the arriving plan re-enables them —
the same discipline `planSeq`/`travelSeq` already apply to out-of-order replies (`:164-179`), which
PR #142's review drove into existence.

## The CLI says pass, not colour

Clean cutover, no aliases:

| Today | After |
|---|---|
| `--by-color` | `--group-by <single\|color\|stroke\|fill\|preset>`, default `single` |
| `--skip-color ff0000ff` | `--skip-pass color:ff0000ff`, repeatable |
| `--order ff0000ff,0000ffff` | `--order color:ff0000ff --order color:0000ffff`, repeatable |

**`--order` becomes repeatable instead of comma-separated.** A preset id may contain a comma, and a
comma-separated list would make such a pass unnameable — an operator-supplied string deciding
whether a flag can address a pass. Repeating the flag also matches `--skip-pass`, so the two read
alike, and the cut order is the order the flags appear.

Both flags **refuse a key that names no planned pass**, rather than ignoring it. `--order` used to
drop an unknown colour silently and `--skip-color` still does (`crates/cli/src/pipeline.rs:81-94`);
with four spellings of a key a typo is likelier than it was, and a silently ignored `--skip-pass`
means cutting a colour the operator believed they had excluded. A key from another mode —
`--group-by fill --skip-pass preset:cameo5-htv` — needs no rule of its own: it names no planned
pass, so it is refused by the same check.

`check_color_flag_scope` becomes `check_pass_flag_scope` and keeps refusing rather than ignoring
(`pipeline.rs:178-196`): under `--group-by single` there is one pass, so there is nothing to skip
and nothing to order. Its two messages, and `check_interactive`'s (`pipeline.rs:198-205`), stop
naming `--by-color`.

Two output details are deliberate, because the plain and grouped paths merge into one:

- **`--group-by single --dry-run` prints raw bytes with no pass header**, exactly as a bare
  `cuthulhu cut --dry-run` does today (`crates/cli/src/main.rs:121-129`). Every other mode prints
  `-- pass i/n (<key>) --` before each pass, exactly as `--by-color` does today. The rule is "a
  header names a pass among several; `single` has none to name", and it is what keeps a scripted
  dry run parsing what it parsed before.
- **"no cuttable paths in SVG" stops covering two different facts.** A file that planned no passes
  keeps that sentence; a selection that skipped every pass it planned gets its own, because
  `--skip-pass no-preset` on a file where nothing carries a material is an empty selection, not an
  empty file.

## Verification

Everything here is decided before a byte reaches a Transport, so nothing is added to
`apps/desktop/MANUAL-CHECKLIST.md` — but two of its live checks name flags this removes
(`:91`, `:102`) and must be rewritten to the new ones.

- **Planner** — one document with two strokes, two fills, a `NoCut` shape and a Layer-assigned
  preset, planned through all five modes with the expected key set for each; `PassKey` round trip
  over every variant plus the injectivity cases; preset resolution (`Inherit` takes the ancestor's,
  `Unassigned` overrides it to `no-preset`, an explicit id overrides it, no ancestor means
  `no-preset`).
- **Document** — `Inherit` is what an absent field and an explicit `null` decode to, through a real
  `.cut` file; `set_material_preset` writes only the selected Nodes; a shape reparented into an
  assigned Layer picks that Layer's preset up on the next plan without being edited.
- **Selection** — `UnknownPass` refused rather than dropped for each key kind, and its sentence per
  variant.
- **Desktop** — a grouping chosen in the dialog reaches all three planner call sites; a mode change
  makes Cut and travel unavailable until the new plan installs; a preset-keyed row carries that
  preset's settings into the `Job`; an unresolved preset renders as a row and is refused at Cut.
- **CLI** — every mode accepted, `--skip-pass`/`--order` refused under `single` and refused for a
  key that names no pass, `--order` repeatable and order-preserving, the dry-run header rule, and
  the two distinct empty-cut sentences.
- **e2e** — the fake honours the grouping in all three handlers and mirrors the backend's exact-once
  identity check, so a frontend that sends stale keys fails the suite instead of passing it.
  #143's stale `travel_for_order` comment is fixed here; it was filed as a ride-along for exactly
  this change.

## Rejected alternatives

**A colour-keyed slice first, generalising the key later.** Smaller, and it would have shipped
`Stroke`/`Fill` without touching `PassSelection`. Rejected because the rename cascade is the
expensive half either way, and doing it second means two passes over the same ten files, two sets
of DTO churn, and an interim vocabulary in `CONTEXT.md` that is wrong the moment preset grouping
lands.

**A tagged enum on the wire.** Structurally honest, and it forces a hand-written discriminated
union into `ipc.ts` plus the e2e fake — the duplication #70 already tracks — while the dialog still
needs a flat string for a row key. The collision that made a naive string grammar unusable is
answered by making absence its own token, not by abandoning the string.

**Grouping stored in `AppState`.** One fewer parameter on three commands, at the cost of a mode
that can change between plan and cut with nothing to detect it.

**`material_preset: Option<String>` with `None` meaning inherit.** See *Three states*: it cannot
express a deliberate "no material" under an assigned Layer, and adding that later is a second wire
migration.

**Descending to shapes when assigning a preset.** Consistent with `set_cut_line_type`, and it
cancels the inheritance out — see *Resolution happens in the walk*.

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
- **`Grouping::LineType` and `PassKey::LineType`** (#56), together with `CutEdge` — the member that
  makes line-type grouping split anything. `Draw`/`Score`/`PrintCutCut`/`PrintCutPrint`/
  `ColorLayerAlignment` each need a driver answer for the tool they imply.
- **A configurable import default** for either attribute (#54).
- **Per-pass settings on the CLI**, which would need a flag that names a pass key. One
  `--speed`/`--force` pair still applies to every pass.

## Revisions

Revised 2026-08-16 after a Codex review of the first draft and its plan returned **BLOCKING** with
fourteen findings, all accepted. The four that changed this document's decisions rather than its
wording:

1. **The grammar was not injective.** `preset:none` meant both "no preset" and a preset whose id is
   literally `none`, since ids are unrestricted operator strings. Absence became its own token.
2. **The preset command cancelled out the inheritance.** It copied `set_cut_line_type`'s descent,
   which exists precisely because `CutLineType` does *not* inherit. Assignment now writes only the
   selection, and the two-state `Option<String>` became `PresetAssignment` so "deliberately no
   material" is representable at all.
3. **A preset-keyed pass was cut with default settings**, because rows initialised `presetId` to
   null while `prepare_cut` reads only that field.
4. **`Grouping::LineType` shipped a mode identical to `Single`.** Removed; it returns with #56.

The remaining ten were plan-level defects and stale citations, fixed in
`docs/superpowers/plans/2026-08-16-pass-grouping.md`.
