<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Pass grouping as an explicit choice — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an operator choose how a Document's shapes are split into passes — by stroke colour, fill colour, material preset, line type, today's stroke-else-fill rule, or one pass over everything — by making a pass's identity a `PassKey` instead of an `Option<u32>` colour, and giving a Node an inheritable material preset to group on. Closes #148.

**Architecture:** `PassKey { All, Color(Option<u32>), LineType(CutLineType), Preset(Option<String>) }` replaces the colour that identifies a pass in ten places from `cutplan::passes` to the e2e fake, and it crosses every boundary as one canonical string (`all`, `color:ff0000ff`, `line-type:cut`, `preset:cameo5-htv`) so the CLI, the JSON DTOs, and the dialog's row keys cannot disagree. `ColorPass` becomes `DocumentPass`. `Grouping` gains `Stroke`, `Fill`, `LineType` and `Preset` beside today's `Color` (stroke-else-fill, still the library default) and `Single`. `Node` gains `material_preset: Option<String>`, inherited down `plan_passes_with`'s existing walk so a Layer's preset covers the shapes under it. The chosen grouping rides in the `plan_cut`, `travel_for_order` and `cut` payloads rather than in `AppState`, because those are three separate round trips and a stored mode can drift from the rows the operator is looking at.

**Tech Stack:** Rust 2021 (`document`, `cutplan`, `cli`, `apps/desktop`), React + TypeScript (`apps/desktop/ui`), Playwright for e2e. **No new dependencies** in either language.

**Spec:** `docs/superpowers/specs/2026-08-16-pass-grouping-design.md` — read it first. It holds the reference-application research, the reasons `All` is not `Color(None)`, why the key is a string on the wire, why the planner does not validate a preset id, and the alternatives that were rejected.

## Global Constraints

**Reading the code blocks in this plan:** a block is the complete text of what it introduces
unless it contains a bare `…` line. A `…` appears only inside a block quoting an *existing*
function this plan modifies, and means "the surrounding lines are unchanged — do not retype
them". Every such block names the file and line range it edits, so the unchanged part is
readable in the tree. There are no placeholders: nothing in this plan is left for the
implementer to invent.

- **SPDX header on every file** — `// SPDX-License-Identifier: GPL-3.0-or-later` (`<!-- -->` in Markdown, `//` in Rust and TypeScript). Every file this plan touches already has one; new files need one.
- **`cargo test --workspace --locked` is the gate**, and `--locked` is mandatory. This change adds no dependency, so `Cargo.lock` must not change. If it does, something was added that this plan did not intend.
- **`ui/dist` is committed.** Any task that edits `apps/desktop/ui/src` must end with `npm --prefix apps/desktop/ui run build` and commit `apps/desktop/ui/dist` in the same commit — CI rebuilds and fails on a stale bundle. Tasks 7, 8, 9 and 10 touch `ui/src`.
- **`CONTEXT.md` is normative vocabulary**, and this change edits it (Task 11). Terms in play: **Node**, **DocumentPasses**, **PassSelection**, **CutPlan**, **Preflight**, **MaterialPreset**, **Settings**. **ColorPass is retired** and replaced by **DocumentPass** plus the new **PassKey**; do not leave both names alive.
- **Comments explain why, not what.** Every comment specified below records a constraint, a trap, or a decision taken against an alternative. Do not add comments restating code.
- **`// ponytail:` marks a deliberate simplification** with its ceiling and upgrade path. Two are specified here: `Grouping::LineType`'s single reachable key, and the CLI's one-settings-pair-per-cut.
- **The e2e fake mirrors the real backend** (`CLAUDE.md:135-138`). `apps/desktop/ui/e2e/smoke.spec.ts` re-implements `plan_passes` at `:302-341`; when the key or the grouping changes, so does the fake, or the suite lies.
- **`Grouping::Color` stays what `plan_passes` defaults to.** Existing behaviour must not move: `Color` is verbatim today's stroke-else-fill rule, and every caller that does not name a mode gets it.
- **The dialog's chosen grouping must reach all three planner call sites.** `plan_cut_response`, `travel_for_order` and `prepare_cut` each plan independently; if one of them plans a different grouping, the travel preview and the cut silently disagree. Any task that adds a call site adds the parameter.
- **Out of scope, and must not creep in:** separate confirmed jobs versus one continuous job (#149), colour-layer alignment marks (#150), `CutEdge` or any new `CutLineType` member (#56), a second per-node "production role" enum (#68 settled that the role *is* `CutLineType`), a configurable import default (#54), and generated IPC types (#70).

## File Structure

| File | Responsibility after this change |
|---|---|
| `crates/cutplan/src/pass_key.rs` | **New.** Owns `PassKey`, its canonical string grammar (`Display`/`FromStr`), and its serde string representation. Kept out of `passes.rs`, which is already 582 lines and owns the walk. |
| `crates/cutplan/src/passes.rs` | `DocumentPass { key, shapes }`, `Grouping`'s six modes, the key-selection rule per mode, and preset inheritance in the walk. |
| `crates/cutplan/src/plan.rs` | `PassSelection { key, settings }`, `PlannedPass { key, job }`, `CutError::UnknownPass(PassKey)` with code `unknown_pass`. |
| `crates/cutplan/src/preflight.rs` | Unchanged rules; `ConfiguredPass::pass` is a `&DocumentPass`. |
| `crates/document/src/node.rs` | `Node::material_preset`, defaulted absent through `NodeWire`. |
| `crates/document/src/commands.rs` | `set_material_preset`, the second per-node production attribute command. |
| `crates/cli/src/main.rs` | `--group-by`, `--skip-pass`, `--order` over pass keys; the dry-run pass label prints a `PassKey`. |
| `crates/cli/src/pipeline.rs` | `pass_order` over keys, `check_pass_flag_scope`, and one planning entry point that takes a `Grouping`. |
| `crates/cli/src/cut.rs` | The operator prompt names a pass by its key, not by a colour. |
| `apps/desktop/src/device.rs` | Cut DTOs carry `key` and `grouping`; the three planner call sites take a `Grouping`. |
| `apps/desktop/src/{state,ipc,main}.rs` | `set_material_preset`, and the grouping parameter on `plan_cut`/`travel_for_order`. |
| `apps/desktop/ui/src/ipc.ts` | `PassKey` (a string), `Grouping`, and the three cut callers that now pass a grouping. |
| `apps/desktop/ui/src/cut/viewmodel.ts` | `PassVm.key`, `parsePassKey`, and the request builders. |
| `apps/desktop/ui/src/cut/{CutDialog,CutPreview}.tsx` | The grouping picker, a row label per key kind, and a swatch derived from a parsed key. |
| `apps/desktop/ui/src/panels/{materialPreset.ts,PropertiesPanel.tsx}` | The operator's per-node preset control, mirroring the cuttability one. |
| `apps/desktop/ui/e2e/smoke.spec.ts` | The fake keys passes the same way, honours the grouping argument, and its stale `travel_for_order` comment is corrected (#143). |
| `CONTEXT.md`, `CLAUDE.md`, `CHANGELOG.md` | DocumentPass and PassKey replace ColorPass in the normative vocabulary; the cut-path paragraph names the grouping. |

**Task order is load-bearing.** Tasks 1 and 2 add things nothing reads yet (`PassKey`, `Node::material_preset`). Task 3 makes the planner key on `PassKey` and inherit presets — the first behaviour change, and the one every later task consumes. Task 4 carries it through selection and refusal. Tasks 5 and 6 give the two binaries a way to choose a mode. Tasks 7–9 are the UI, in dependency order (wire → dialog → panel). Task 10 makes the e2e fake tell the truth again. Task 11 updates the vocabulary the whole change contradicts. **Every task ends green on `cargo test --workspace --locked`**, and every task that touches `ui/src` also ends green on `npm --prefix apps/desktop/ui test`.

---

### Task 1: `PassKey` and its one grammar

**Files:**
- Create: `crates/cutplan/src/pass_key.rs`
- Modify: `crates/cutplan/src/lib.rs` (add `mod pass_key;` and re-export)

**Interfaces:**
- Consumes: `document::CutLineType`.
- Produces: `cutplan::PassKey` with `Display`, `FromStr` (`Err = String`), `From<PassKey> for String`, `TryFrom<String>`, `Serialize`/`Deserialize` as the canonical string. Task 3 keys passes with it, Task 4 selects and refuses with it, Task 5 parses CLI values into it, Task 6 serializes it, Task 7 mirrors it in TypeScript.

- [ ] **Step 1: Write the failing tests**

Create `crates/cutplan/src/pass_key.rs` containing only the SPDX header, `use` lines, and this test module:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant, in both directions. The same table appears in TypeScript
    /// (`apps/desktop/ui/src/cut/viewmodel.test.ts`) because the string is the only thing
    /// that crosses the boundary — if the two tables disagree, the dialog and the planner
    /// disagree about what a pass is called.
    #[test]
    fn every_key_round_trips_through_its_canonical_string() {
        let table = [
            (PassKey::All, "all"),
            (PassKey::Color(Some(0xFF0000FF)), "color:ff0000ff"),
            (PassKey::Color(Some(0x0000FFFF)), "color:0000ffff"),
            (PassKey::Color(None), "color:none"),
            (PassKey::LineType(CutLineType::Cut), "line-type:cut"),
            (PassKey::LineType(CutLineType::NoCut), "line-type:no-cut"),
            (PassKey::Preset(Some("cameo5-htv".into())), "preset:cameo5-htv"),
            (PassKey::Preset(None), "preset:none"),
        ];
        for (key, text) in table {
            assert_eq!(key.to_string(), text);
            assert_eq!(text.parse::<PassKey>().unwrap(), key);
            assert_eq!(serde_json::to_string(&key).unwrap(), format!("\"{text}\""));
            assert_eq!(serde_json::from_str::<PassKey>(&format!("\"{text}\"")).unwrap(), key);
        }
    }

    /// A colour is written lowercase and read either way, so the round trip is a fixed
    /// point: two spellings of one key would otherwise both appear in a pass list, and
    /// `plan_cut` matches keys by equality.
    #[test]
    fn a_colour_is_read_in_any_case_and_written_in_one() {
        assert_eq!("color:FF0000ff".parse::<PassKey>().unwrap(), PassKey::Color(Some(0xFF0000FF)));
        assert_eq!(PassKey::Color(Some(0xFF0000FF)).to_string(), "color:ff0000ff");
    }

    /// A preset id may contain a colon, because ids are the operator's and nothing
    /// validates them. Splitting on the first separator is what allows that.
    #[test]
    fn a_preset_id_may_contain_the_separator() {
        let key: PassKey = "preset:vinyl:thin".parse().unwrap();
        assert_eq!(key, PassKey::Preset(Some("vinyl:thin".into())));
        assert_eq!(key.to_string(), "preset:vinyl:thin");
    }

    /// Refused rather than coerced, and refused with the string in the message: these
    /// arrive from a person typing `--skip-pass`, so the error is read by a human.
    #[test]
    fn a_malformed_key_is_refused_by_name() {
        for bad in ["", "all:1", "color:", "color:zz", "color:ff0000", "line-type:draw", "colour:ff0000ff"] {
            let err = bad.parse::<PassKey>().expect_err("{bad} must not parse");
            assert!(err.contains(bad), "{err} should quote the input");
        }
    }

    /// A 6-digit colour is the trap `parse_hex_color` was written for: it would parse as
    /// `0x00RRGGBB`, a colour no shape has, and silently match nothing.
    #[test]
    fn a_six_digit_colour_is_refused_rather_than_zero_padded() {
        assert!("color:ff0000".parse::<PassKey>().is_err());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cutplan pass_key`

Expected: compile errors — `cannot find type 'PassKey' in this scope`.

- [ ] **Step 3: Write the implementation**

Above the test module in `crates/cutplan/src/pass_key.rs`:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
//! What a pass is called, and the one string form that name takes everywhere.

use document::CutLineType;
use serde::{Deserialize, Serialize};

/// What a `DocumentPass` is keyed on — the answer to "which pass is this?" for whichever
/// `Grouping` produced it.
///
/// `All` is deliberately not `Color(None)`. Before #148 the single pass a `Grouping::Single`
/// plan holds and the pass of shapes with no visible paint were both keyed `None`, so a
/// refusal could only say the evasive "no planned pass without a color". One value meaning
/// two things is what this variant buys out.
///
/// The `Option`s are inside their variants rather than a shared `Unassigned`, because
/// absence is a property of that mode's key: `Color(None)` is a shape with no visible paint,
/// and `Preset(None)` is a shape nobody has assigned a material to — the ordinary state, not
/// an error.
///
/// Serialized as its canonical string (`Display`/`FromStr`) rather than as a tagged enum:
/// the CLI needs a human grammar, the cut dialog needs a stable row key, and the e2e fake
/// has to produce byte-identical values. One representation cannot drift from itself.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum PassKey {
    All,
    Color(Option<u32>),
    LineType(CutLineType),
    Preset(Option<String>),
}

impl std::fmt::Display for PassKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PassKey::All => write!(f, "all"),
            // Lowercase, always: `FromStr` accepts either case, so writing one of them is
            // what makes the round trip a fixed point.
            PassKey::Color(Some(c)) => write!(f, "color:{c:08x}"),
            PassKey::Color(None) => write!(f, "color:none"),
            PassKey::LineType(CutLineType::Cut) => write!(f, "line-type:cut"),
            PassKey::LineType(CutLineType::NoCut) => write!(f, "line-type:no-cut"),
            PassKey::Preset(Some(id)) => write!(f, "preset:{id}"),
            PassKey::Preset(None) => write!(f, "preset:none"),
        }
    }
}

impl std::str::FromStr for PassKey {
    type Err = String;
    fn from_str(s: &str) -> Result<PassKey, String> {
        let unknown = || format!("'{s}' is not a pass key (all, color:RRGGBBAA, color:none, line-type:cut, line-type:no-cut, preset:<id>, preset:none)");
        // First separator only: a preset id is the operator's string and may contain a
        // colon, so the grammar must not constrain ids further than the app does.
        let Some((mode, value)) = s.split_once(':') else {
            return if s == "all" { Ok(PassKey::All) } else { Err(unknown()) };
        };
        match (mode, value) {
            ("color", "none") => Ok(PassKey::Color(None)),
            // Eight digits exactly: a 6-digit RRGGBB would parse as 0x00RRGGBB — a colour
            // no shape carries — and match nothing while looking like it should.
            ("color", hex) if hex.len() == 8 => u32::from_str_radix(hex, 16)
                .map(|c| PassKey::Color(Some(c)))
                .map_err(|_| format!("'{s}' has a colour that is not 8 hex digits (RRGGBBAA)")),
            ("color", _) => Err(format!("'{s}' has a colour that is not 8 hex digits (RRGGBBAA)")),
            ("line-type", "cut") => Ok(PassKey::LineType(CutLineType::Cut)),
            ("line-type", "no-cut") => Ok(PassKey::LineType(CutLineType::NoCut)),
            ("preset", "none") => Ok(PassKey::Preset(None)),
            ("preset", id) => Ok(PassKey::Preset(Some(id.to_string()))),
            _ => Err(unknown()),
        }
    }
}

// The pair serde's `into`/`try_from` needs. `into` clones, which is why `PassKey` derives
// `Clone` — it holds a `String` and would need it anyway.
impl From<PassKey> for String {
    fn from(key: PassKey) -> String { key.to_string() }
}
impl TryFrom<String> for PassKey {
    type Error = String;
    fn try_from(s: String) -> Result<PassKey, String> { s.parse() }
}
```

In `crates/cutplan/src/lib.rs`, add the module beside the existing ones and re-export it exactly as `passes`/`plan` are re-exported (match whatever form that file already uses — `pub use pass_key::*;` if the others are glob re-exports).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. Nothing consumes `PassKey` yet, so no existing test can see it.

- [ ] **Step 5: Commit**

```bash
git add crates/cutplan/src/pass_key.rs crates/cutplan/src/lib.rs
git commit -m "Give a pass a name that is not a colour, in one spelling

#148 needs a pass key that can be a line type or a preset, and the CLI, the
IPC payloads, the cut dialog's row keys and the e2e fake all have to agree on
what a pass is called. One canonical string, parsed and written in one place,
is what keeps four consumers from inventing four encodings."
```

---

### Task 2: A Node carries a material preset

**Files:**
- Modify: `crates/document/src/node.rs:44-63` (the `Node` struct and its two constructors), plus `NodeWire` and its `From` impl
- Modify: `crates/document/src/commands.rs` (add `set_material_preset` after `set_cut_line_type` at `:55-81`)
- Test: `crates/document/src/node.rs` (existing `mod tests`), `crates/document/src/commands.rs` (existing `mod tests`), `crates/fileio/src/project.rs` (existing `mod tests`, beside `a_project_saved_before_cuttability_derives_it_from_stroke`)

**Interfaces:**
- Consumes: nothing.
- Produces: `Node::material_preset: Option<String>` and `document::commands::set_material_preset(&Document, &[NodeId], Option<String>) -> Result<Delta, CmdError>`. Task 3 groups on the field, Task 6 exposes the command over IPC, Task 9 renders its control.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/document/src/node.rs`:

```rust
    /// A new Node has no material assigned. Unlike `cut_line_type`, there is no
    /// import default to argue about: a preset is the operator's choice per shape or
    /// per Layer, and "none" is the honest starting state.
    #[test]
    fn a_new_node_has_no_material_preset() {
        let mut ids = IdGen::default();
        assert_eq!(Node::shape(ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 }).material_preset, None);
        assert_eq!(Node::container(ids.next(), NodeKind::Group).material_preset, None);
    }

    /// A document written before the field existed had no way to assign a preset, so
    /// absence genuinely means none — the migration is the default, and this is the one
    /// place that is true of the two production attributes a Node carries.
    #[test]
    fn a_node_saved_without_a_material_preset_has_none() {
        let json = r#"{"id":7,"kind":{"Shape":{"Rect":{"w":1.0,"h":1.0}}},
                       "transform":[1.0,0.0,0.0,1.0,0.0,0.0],
                       "style":{"stroke":255,"fill":null},
                       "cut_line_type":"Cut","children":[]}"#;
        let node: Node = serde_json::from_str(json).unwrap();
        assert_eq!(node.material_preset, None);
        assert_eq!(node.cut_line_type, CutLineType::Cut, "premise: the other attribute still decodes");
    }

    /// Written on every save, so a preset survives a round trip and the field stops being
    /// absent the first time a document is written by this version.
    #[test]
    fn a_material_preset_round_trips() {
        let mut node = Node::shape(NodeId(1), ShapeKind::Rect { w: 1.0, h: 1.0 });
        node.material_preset = Some("cameo5-htv".into());
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains(r#""material_preset":"cameo5-htv""#), "{json}");
        assert_eq!(serde_json::from_str::<Node>(&json).unwrap(), node);
    }
```

Append to `mod tests` in `crates/fileio/src/project.rs`, beside the cuttability migration test
#144 added — serde in isolation cannot see the container an operator's file actually is:

```rust
    /// The migration at the level an operator experiences it: a real `.cut` written before
    /// the field existed. The manifest is pruned rather than hand-written, so the fixture
    /// cannot drift from `Document`'s real shape — everything except `material_preset` is
    /// exactly what `save_project` emits today.
    #[test]
    fn a_project_saved_before_material_presets_loads_with_none() {
        let mut doc = document::Document::new();
        let shape = document::Node::shape(doc.ids.next(),
            document::ShapeKind::Rect { w: 10.0, h: 10.0 });
        let shape_id = shape.id;
        doc.apply(document::Delta(vec![
            document::NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX },
        ]));

        let mut manifest: serde_json::Value = serde_json::from_str(&doc.snapshot_json()).unwrap();
        for node in manifest["nodes"].as_object_mut().unwrap().values_mut() {
            assert!(node.as_object_mut().unwrap().remove("material_preset").is_some(),
                "premise: every node is written with the field, so pruning it makes an old file");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.cut");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.finish().unwrap();

        let back = load_project(&path).unwrap();
        assert_eq!(back.get(shape_id).unwrap().material_preset, None);
        assert_eq!(back.get(shape_id).unwrap().cut_line_type, document::CutLineType::Cut,
            "premise: the neighbouring migration still runs");
    }
```

`design.svg` is deliberately absent from the fixture: `load_project` reads only `manifest.json`
(`crates/fileio/src/project.rs:29`), so a container without it is a valid old file for this
test's purpose and one fewer thing to keep in step.

Append to `mod tests` in `crates/document/src/commands.rs`:

```rust
    /// Descends into containers for the same reason `set_cut_line_type` does: the planner
    /// reads a preset on the shape it inherits to, and a control that set it on a Group
    /// alone would look like it did nothing. Selecting a Layer means its shapes.
    #[test]
    fn set_material_preset_marks_shapes_under_a_container() {
        let mut doc = Document::new();
        let group = Node::container(doc.ids.next(), NodeKind::Group);
        let group_id = group.id;
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![
            NodeOp::Add { parent: doc.root, node: group, index: usize::MAX },
            NodeOp::Add { parent: group_id, node: shape, index: usize::MAX },
        ]));

        let delta = set_material_preset(&doc, &[group_id], Some("cameo5-htv".into())).unwrap();
        doc.apply(delta);
        assert_eq!(doc.get(shape_id).unwrap().material_preset.as_deref(), Some("cameo5-htv"));
        assert_eq!(doc.get(group_id).unwrap().material_preset, None,
            "the container is not what the planner reads");
    }

    /// Clearing is a value, not a separate verb — the panel's "No preset" option and an
    /// undo of an assignment are the same edit.
    #[test]
    fn set_material_preset_clears_with_none() {
        let mut doc = Document::new();
        let mut shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        shape.material_preset = Some("cameo5-htv".into());
        let shape_id = shape.id;
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        let delta = set_material_preset(&doc, &[shape_id], None).unwrap();
        doc.apply(delta);
        assert_eq!(doc.get(shape_id).unwrap().material_preset, None);
    }

    /// Re-picking the value a selection already has emits nothing, so it cannot land an
    /// undo step that undoes nothing — the same rule `set_cut_line_type` follows.
    #[test]
    fn set_material_preset_emits_nothing_for_an_unchanged_selection() {
        let mut doc = Document::new();
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        assert!(set_material_preset(&doc, &[shape_id], None).unwrap().0.is_empty());
        assert_eq!(set_material_preset(&doc, &[], None), Err(CmdError::EmptySelection));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p document material_preset`

Expected: compile errors — `no field 'material_preset' on type 'Node'` and `cannot find function 'set_material_preset'`.

- [ ] **Step 3: Write the implementation**

In `crates/document/src/node.rs`, add the field to `Node` (after `cut_line_type` at `:51`) and to both constructors:

```rust
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(from = "NodeWire")]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub transform: Affine,   // relative to parent
    pub style: Style,
    pub cut_line_type: CutLineType,
    /// The `MaterialPreset::id` this Node's geometry is cut with, if the operator assigned
    /// one. A sibling of `cut_line_type` and for the same reason (#68): production intent is
    /// not paint. Read by `cutplan::plan_passes`, which inherits it down the tree, so a
    /// value on a Layer covers the shapes under it.
    ///
    /// Never validated here: presets are machine-scoped and a user entry can be deleted, so
    /// an id that resolves to nothing is a real state a Document may hold.
    pub material_preset: Option<String>,
    pub children: Vec<NodeId>,
}
impl Node {
    pub fn shape(id: NodeId, kind: ShapeKind) -> Node {
        Node { id, kind: NodeKind::Shape(kind), transform: Affine::identity(),
               style: Style::default(), cut_line_type: CutLineType::Cut,
               material_preset: None, children: vec![] }
    }
    pub fn container(id: NodeId, kind: NodeKind) -> Node {
        Node { id, kind, transform: Affine::identity(),
               style: Style::default(), cut_line_type: CutLineType::Cut,
               material_preset: None, children: vec![] }
    }
}
```

Add the field to `NodeWire` beside `cut_line_type`, and to its `From` impl. `#[serde(default)]` is the whole migration here — say why in a comment, because the neighbouring field's rule is deliberately different:

```rust
    /// A plain `#[serde(default)]`, unlike `cut_line_type` above: a document written before
    /// this field existed had no way to assign a preset, so absence *is* none. There is
    /// nothing to derive and no old behaviour to preserve.
    #[serde(default)]
    material_preset: Option<String>,
```

and in `From<NodeWire> for Node`, pass it straight through: `material_preset: w.material_preset,`.

In `crates/document/src/commands.rs`, add after `set_cut_line_type` (`:81`):

```rust
/// Assign `value` to every shape in `ids` and every shape beneath a container in `ids`.
///
/// The same walk as `set_cut_line_type`, for the same reason: `cutplan::plan_passes` reads
/// the attribute on the shape (inheriting a container's value down to it), so setting it on
/// a Group alone would be a control that visibly does nothing.
pub fn set_material_preset(doc: &Document, ids: &[NodeId], value: Option<String>)
    -> Result<Delta, CmdError> {
    if ids.is_empty() { return Err(CmdError::EmptySelection); }
    let mut ops = vec![];
    let mut seen = HashSet::new();
    let mut stack: Vec<NodeId> = ids.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let node = doc.get(id).ok_or(CmdError::NotFound)?;
        // Skips a revisit rather than refusing it: an overlapping selection (a Group and a
        // shape inside it) is the ordinary case here, unlike `plan_passes_with`'s walk from
        // the single root where a revisit can only mean a cycle.
        if !seen.insert(id) { continue; }
        match &node.kind {
            NodeKind::Group | NodeKind::Layer => stack.extend(node.children.iter().rev().copied()),
            NodeKind::Shape(_) => {
                if node.material_preset == value { continue; }
                let before = node.clone();
                let mut after = before.clone();
                after.material_preset = value.clone();
                ops.push(NodeOp::Update { id, before, after });
            }
        }
    }
    Ok(Delta(ops))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. Every `Node` literal in the workspace is built through `Node::shape`/`Node::container`, so the new field needs no other edit; if a struct literal exists somewhere, the compiler names it.

- [ ] **Step 5: Commit**

```bash
git add crates/document/src/node.rs crates/document/src/commands.rs crates/fileio/src/project.rs
git commit -m "Let a Node say which material it is cut with, and inherit nothing yet

#148 groups passes by material preset, which needs somewhere to put one. It
sits beside cut_line_type rather than in Style for the reason #68 settled:
production intent is not paint. Absence is none here, with nothing to derive
from an old document, because a preset was never assignable before now."
```

---

### Task 3: The planner keys on a `PassKey` and inherits a preset

**Files:**
- Modify: `crates/cutplan/src/passes.rs:14-21` (`ColorPass` → `DocumentPass`), `:31-32` (`DocumentPasses::passes`), `:67-100` (`pass_key`, `Grouping`, `plan_passes`), `:100-169` (`plan_passes_with`'s walk), `:175` (`travel_moves`)
- Modify: `crates/cutplan/src/preflight.rs:5-11` (the `ColorPass` import and `ConfiguredPass::pass`)
- Test: `crates/cutplan/src/passes.rs` (existing `mod tests` from `:193`)

**Interfaces:**
- Consumes: `PassKey` (Task 1), `Node::material_preset` (Task 2).
- Produces: `DocumentPass { key: PassKey, shapes: Vec<PlannedShape> }`, `Grouping { Single, Color, Stroke, Fill, LineType, Preset }`, and `plan_passes_with(&Document, Grouping)`. Task 4 matches selections against `key`; Tasks 5–10 choose a `Grouping`.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/cutplan/src/passes.rs`:

```rust
    /// One document, six modes, and the key set each produces. The point of the table is
    /// that the modes are only different in what they key on: the same shapes are cut, in
    /// the same document order, and only the split changes.
    #[test]
    fn every_grouping_keys_the_same_shapes_differently() {
        let mut doc = Document::new();
        // Red stroke + green fill, green stroke + green fill, fill only, no visible paint.
        let mut a = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        a.style = Style { stroke: Some(RED), fill: Some(GREEN) };
        let mut b = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        b.style = Style { stroke: Some(GREEN), fill: Some(GREEN) };
        let mut c = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        c.style = Style { stroke: None, fill: Some(BLUE) };
        let mut d = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        d.style = Style { stroke: None, fill: None };
        doc.apply(Delta(vec![
            NodeOp::Add { parent: doc.root, node: a, index: usize::MAX },
            NodeOp::Add { parent: doc.root, node: b, index: usize::MAX },
            NodeOp::Add { parent: doc.root, node: c, index: usize::MAX },
            NodeOp::Add { parent: doc.root, node: d, index: usize::MAX },
        ]));

        let keys = |g: Grouping| -> Vec<String> {
            plan_passes_with(&doc, g).unwrap().passes.iter().map(|p| p.key.to_string()).collect()
        };

        assert_eq!(keys(Grouping::Single), vec!["all"]);
        // Stroke where visible, else fill: the rule #144 shipped, unchanged.
        assert_eq!(keys(Grouping::Color),
            vec!["color:ff0000ff", "color:00ff00ff", "color:0000ffff", "color:none"]);
        // Strict: a shape with no visible stroke keys on no colour at all, which is the
        // same bucket a shape with no paint whatsoever lands in.
        assert_eq!(keys(Grouping::Stroke), vec!["color:ff0000ff", "color:00ff00ff", "color:none"]);
        assert_eq!(keys(Grouping::Fill), vec!["color:00ff00ff", "color:0000ffff", "color:none"]);
        // One reachable key until CutLineType gains a second cuttable member (#56).
        assert_eq!(keys(Grouping::LineType), vec!["line-type:cut"]);
        assert_eq!(keys(Grouping::Preset), vec!["preset:none"]);

        // Every mode cuts all four shapes; only the split differs.
        for g in [Grouping::Single, Grouping::Color, Grouping::Stroke, Grouping::Fill,
                  Grouping::LineType, Grouping::Preset] {
            let planned = plan_passes_with(&doc, g).unwrap();
            let shapes: usize = planned.passes.iter().map(|p| p.shapes.len()).sum();
            assert_eq!(shapes, 4, "{g:?} dropped a shape");
            assert_eq!(planned.skipped_not_cut, 0);
        }
    }

    /// `plan_passes` is what every caller that does not name a mode gets, and #148 must not
    /// move it: `Color` is verbatim the stroke-else-fill rule those callers already had.
    #[test]
    fn the_default_grouping_is_unchanged_colour_grouping() {
        let mut doc = Document::new();
        let mut shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        shape.style = Style { stroke: None, fill: Some(RED) };
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        assert_eq!(plan_passes(&doc).unwrap().passes, plan_passes_with(&doc, Grouping::Color).unwrap().passes);
        assert_eq!(plan_passes(&doc).unwrap().passes[0].key, PassKey::Color(Some(RED)));
    }

    /// A preset on a Layer covers the shapes under it, and a shape overrides it. This is
    /// why inheritance lives in the walk: the alternative is storing a derived value on
    /// every shape, which goes stale the moment a node is reparented.
    #[test]
    fn a_preset_is_inherited_from_the_nearest_ancestor() {
        let mut doc = Document::new();
        let mut layer = Node::container(doc.ids.next(), NodeKind::Layer);
        layer.material_preset = Some("cameo5-htv".into());
        let layer_id = layer.id;
        let inherits = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let mut overrides = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        overrides.material_preset = Some("cameo5-vinyl-adhesive".into());
        let outside = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        doc.apply(Delta(vec![
            NodeOp::Add { parent: doc.root, node: layer, index: usize::MAX },
            NodeOp::Add { parent: layer_id, node: inherits, index: usize::MAX },
            NodeOp::Add { parent: layer_id, node: overrides, index: usize::MAX },
            NodeOp::Add { parent: doc.root, node: outside, index: usize::MAX },
        ]));

        let keys: Vec<String> = plan_passes_with(&doc, Grouping::Preset).unwrap()
            .passes.iter().map(|p| p.key.to_string()).collect();
        assert_eq!(keys, vec!["preset:cameo5-htv", "preset:cameo5-vinyl-adhesive", "preset:none"]);
    }

    /// An id no preset file resolves keys a pass anyway. Refusing here would put a
    /// settings-file concern behind `plan_cut`, which exists to refuse geometry and machine
    /// mismatches — and a user preset can be deleted while a document still names it.
    #[test]
    fn an_unknown_preset_id_still_keys_a_pass() {
        let mut doc = Document::new();
        let mut shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        shape.material_preset = Some("deleted-by-the-operator".into());
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        assert_eq!(plan_passes_with(&doc, Grouping::Preset).unwrap().passes[0].key,
            PassKey::Preset(Some("deleted-by-the-operator".into())));
    }

    /// The predicate is still `cut_line_type`, and it is still checked before the outline
    /// is resolved (#139) — a grouping mode changes the key, never the order of those two.
    #[test]
    fn a_no_cut_shape_is_counted_under_every_grouping() {
        let mut doc = Document::new();
        let mut shape = Node::shape(doc.ids.next(), ShapeKind::Text {
            family: "no such family".into(), size_mm: 10.0, text: "x".into() });
        shape.cut_line_type = CutLineType::NoCut;
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        for g in [Grouping::Single, Grouping::Color, Grouping::Stroke, Grouping::Fill,
                  Grouping::LineType, Grouping::Preset] {
            let planned = plan_passes_with(&doc, g).unwrap();
            assert_eq!(planned.skipped_not_cut, 1, "{g:?}");
            assert!(planned.passes.is_empty(), "{g:?}");
        }
    }
```

Add `const GREEN: u32 = 0x00FF00FF;` beside the existing colour constants if the module does not already have one, and extend the module's `use` line with `CutLineType` and `NodeKind` if they are not already imported.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cutplan passes`

Expected: compile errors — `no variant 'Stroke' found for enum 'Grouping'`, `no field 'key' on type 'ColorPass'`.

- [ ] **Step 3: Write the implementation**

Rename the type and key it (replacing `crates/cutplan/src/passes.rs:14-21`):

```rust
/// All shapes cut together as one pass, and the key that says which pass it is. What the
/// key means is the `Grouping`'s business: a colour, a line type, a material preset, or
/// `All` for the single pass a `Grouping::Single` plan holds.
///
/// Named for the Document rather than for a colour because a colour is now one of four
/// things a pass can be keyed on — the type was `ColorPass` while that was the only one.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DocumentPass { pub key: PassKey, pub shapes: Vec<PlannedShape> }
```

Update `DocumentPasses::passes` to `Vec<DocumentPass>` (`:31-32`) and `travel_moves`'s parameter to `&[&DocumentPass]` (`:175`).

Replace `pass_key`, `Grouping` and `plan_passes` (`:67-97`) with:

```rust
/// The colour a shape's pass is keyed on under a colour-ish `Grouping`. Alpha-0 counts as
/// absent, exactly as a 0-alpha stroke did when the stroke decided cuttability.
///
/// `Color` falls back from stroke to fill because a shape with no stroke can be cut since
/// #144 — traced and fill-only art is the common case — and a pass with no colour at all is
/// something an operator cannot recognise in a pass list. `Stroke` and `Fill` are strict by
/// request: an operator who asked to split by one paint does not want the other silently
/// standing in for it.
fn color_key(style: &Style, grouping: Grouping) -> Option<u32> {
    let visible = |c: Option<u32>| c.filter(|c| c & 0xFF != 0);
    match grouping {
        Grouping::Color => visible(style.stroke).or(visible(style.fill)),
        Grouping::Stroke => visible(style.stroke),
        Grouping::Fill => visible(style.fill),
        // Not reachable: the caller only asks for a colour under a colour mode.
        Grouping::Single | Grouping::LineType | Grouping::Preset => None,
    }
}

/// How `plan_passes` splits cut shapes into passes.
///
/// `Color` is today's rule — stroke where visible, else fill — and stays the default, so a
/// caller that names no mode plans exactly what it planned before #148. `Single` is one pass
/// in document order, which is what `cuthulhu cut` without `--group-by` has always meant.
///
/// ponytail: `LineType` has one reachable key while `CutLineType` is `{Cut, NoCut}` — a
/// `NoCut` shape never reaches a pass at all. It splits real passes as soon as the enum
/// gains a second cuttable member (`CutEdge`, #56), with no change needed here.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Grouping { Single, Color, Stroke, Fill, LineType, Preset }

/// Walk the document in preorder from `doc.root`, group the shapes whose `CutLineType` is
/// `Cut` by the key `grouping` asks for, and flatten each shape's outline under its
/// accumulated world transform. A `NoCut` shape is counted, not cut. Iterative (explicit
/// stack) so depth is not bounded by the Rust call stack; a `visited` set catches cycles in
/// malformed docs.
pub fn plan_passes(doc: &Document) -> Result<DocumentPasses, PlanError> {
    plan_passes_with(doc, Grouping::Color)
}
```

In `plan_passes_with`, the stack carries the inherited preset alongside the world transform, and the key comes from the mode. Replace the stack declaration and the push in the container arm:

```rust
    // The nearest ancestor's preset rides down the walk beside the world transform. Storing
    // a resolved value on each shape instead would go stale the moment a node is reparented.
    let mut stack: Vec<(NodeId, Affine, Option<&str>)> = vec![(doc.root, Affine::identity(), None)];
```

```rust
            NodeKind::Group | NodeKind::Layer => {
                let inherited = node.material_preset.as_deref().or(preset);
                // Push in reverse so preorder visits children left-to-right.
                for &child in node.children.iter().rev() {
                    stack.push((child, world, inherited));
                }
            }
```

and in the `Cut` arm, replace the `let color = match grouping { … }` block plus the pass lookup:

```rust
                        let key = match grouping {
                            // One bucket, and a key that says so: `Color(None)` would be
                            // the pass of unpainted shapes, which is a different fact.
                            Grouping::Single => PassKey::All,
                            Grouping::Color | Grouping::Stroke | Grouping::Fill =>
                                PassKey::Color(color_key(&node.style, grouping)),
                            Grouping::LineType => PassKey::LineType(node.cut_line_type),
                            // A shape's own value wins over what it inherited; neither is
                            // checked against the preset file, which is `plan_cut`'s
                            // non-business (a deleted user preset is a real state).
                            Grouping::Preset => PassKey::Preset(
                                node.material_preset.clone().or_else(|| preset.map(String::from))),
                        };
                        match passes.iter_mut().find(|p| p.key == key) {
                            Some(pass) => pass.shapes.push(shape),
                            None => passes.push(DocumentPass { key, shapes: vec![shape] }),
                        }
```

Destructure the third stack element in the `while let` (`(id, parent_world, preset)`), and add `PassKey` to the module's imports.

In `crates/cutplan/src/preflight.rs`, change the import at `:5` to `use crate::passes::DocumentPass;` and `ConfiguredPass::pass` to `&'a DocumentPass`.

Fix the doc comments that describe the old behaviour: `DocumentPasses`' own comment (`:23-29`) is still true; `plan.rs:15-17`'s "keyed on the colour" is Task 4's to fix.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: compile errors first in `crates/cutplan/src/plan.rs` and the desktop, which Tasks 4 and 6 own. **Get `-p cutplan --lib passes` green in this task, and expect the workspace build to fail until Task 4 lands** — the rename cannot be split from its consumers any smaller than that. If a reviewer needs a green workspace at every task boundary, fold Task 4 into this one.

Run: `cargo test -p cutplan --lib passes`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cutplan/src/passes.rs crates/cutplan/src/preflight.rs
git commit -m "Split passes by what the caller asked for, not only by a colour

Grouping gains stroke, fill, line type and preset beside the stroke-else-fill
rule, which stays the default so nothing existing re-plans differently. The
pass a Single plan holds is keyed All rather than the colourless key it shared
with unpainted shapes. A preset inherits down the walk, so a Layer's value
covers its shapes without storing a copy that reparenting would falsify."
```

---

### Task 4: Selection and refusal speak in keys

**Files:**
- Modify: `crates/cutplan/src/plan.rs:15-22` (`PassSelection`), `:38-44` (`PlannedPass`), `:58-99` (`CutError`, its `Display`, its `code`), `:125-152` (matching and flattening)
- Test: `crates/cutplan/src/plan.rs` (existing `mod tests` from `:154`)

**Interfaces:**
- Consumes: `PassKey` (Task 1), `DocumentPass` (Task 3).
- Produces: `PassSelection { key: PassKey, settings: Settings }`, `PlannedPass { key: PassKey, job: Job }`, `CutError::UnknownPass(PassKey)` with `code()` `"unknown_pass"`. Tasks 5 and 6 build selections; Task 5 prints `PlannedPass::key`.

- [ ] **Step 1: Write the failing tests**

Replace the two colour-named tests in `crates/cutplan/src/plan.rs`'s `mod tests` — `unknown_pass_color_is_refused_not_dropped` (`:213`) and the refusal-table entries in `every_refusal_has_a_code_and_a_sentence` (`:292-305`) — with:

```rust
    /// A selection naming a pass that was not planned is refused, never quietly dropped:
    /// cutting three of four passes because one name was wrong is a ruined sheet.
    #[test]
    fn an_unknown_pass_is_refused_not_dropped() {
        let planned = passes(&[(RED, 0.0, 0.0)]);
        let missing = PassKey::Color(Some(0xDEADBEEF));
        let err = plan_cut(&planned, &profile(500.0, 500.0), &caps(),
            &opts(vec![PassSelection { key: missing.clone(), settings: Settings::default() }]))
            .unwrap_err();
        assert_eq!(err, CutError::UnknownPass(missing));
    }

    /// Every refusal, with the code a caller branches on and the sentence an operator
    /// reads. One arm per key kind, because the sentence names the key: "no planned pass
    /// called preset:cameo5-htv" is a different fact from a colour that is not there.
    #[test]
    fn every_refusal_has_a_code_and_a_sentence() {
        let stale = CutError::StalePlan { expected: 7, actual: 9 };
        assert_eq!(stale.code(), "stale_plan");
        assert_eq!(stale.to_string(), "the document changed since this cut was planned");

        for (key, sentence) in [
            (PassKey::Color(Some(0xFF0000FF)), "no planned pass is called color:ff0000ff"),
            (PassKey::Color(None), "no planned pass is called color:none"),
            (PassKey::All, "no planned pass is called all"),
            (PassKey::LineType(document::CutLineType::Cut), "no planned pass is called line-type:cut"),
            (PassKey::Preset(Some("cameo5-htv".into())), "no planned pass is called preset:cameo5-htv"),
        ] {
            let err = CutError::UnknownPass(key);
            assert_eq!(err.code(), "unknown_pass");
            assert_eq!(err.to_string(), sentence);
        }

        let wrapped = CutError::Preflight(PreflightError::NothingToCut);
        assert_eq!(wrapped.code(), "nothing_to_cut");
        assert_eq!(wrapped.to_string(), PreflightError::NothingToCut.to_string());
    }
```

Update the module's other tests mechanically: every `PassSelection { color: Some(RED), … }` becomes `PassSelection { key: PassKey::Color(Some(RED)), … }`, and every `p.color` on a `PlannedPass` becomes `p.key`. The `select(&[…])` helper (used at `:215`, `:220`, `:229`) should take `PassKey`s so the call sites read as keys rather than as colours.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cutplan plan`

Expected: compile errors — `no field 'key' on type 'PassSelection'`, `no variant 'UnknownPass'`.

- [ ] **Step 3: Write the implementation**

```rust
/// One pass the caller wants cut, named by the key `plan_passes` gave it. Order within
/// `PlanOptions::passes` is the order they are cut.
#[derive(Clone, Debug, PartialEq)]
pub struct PassSelection {
    pub key: PassKey,
    pub settings: Settings,
}
```

```rust
/// One pass, ready to encode. Keeps its key attached to the geometry it belongs to so
/// callers can label passes without index-matching a second list.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedPass {
    pub key: PassKey,
    pub job: Job,
}
```

```rust
#[derive(Debug, PartialEq)]
pub enum CutError {
    StalePlan { expected: u64, actual: u64 },
    UnknownPass(PassKey),
    Preflight(PreflightError),
}
```

In `Display`, the two colour arms collapse into one, because the key knows how to name itself — which is the point of it having one spelling:

```rust
            // The key's own `Display`, not a re-spelling of it: a caller who typed
            // `--skip-pass preset:cameo5-htv` must read that string back verbatim.
            CutError::UnknownPass(key) => write!(f, "no planned pass is called {key}"),
```

In `code()`, `CutError::UnknownPass(_) => "unknown_pass"`.

In `plan_cut`, match on the key:

```rust
        let pass = planned
            .passes
            .iter()
            .find(|p| p.key == sel.key)
            .ok_or_else(|| CutError::UnknownPass(sel.key.clone()))?;
```

and in the `CutPlan` construction, `key: c.pass.key.clone(),`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cutplan --locked`

Expected: PASS. The workspace still fails to build in `cli` and `apps/desktop`, which Tasks 5 and 6 own.

- [ ] **Step 5: Commit**

```bash
git add crates/cutplan/src/plan.rs
git commit -m "Refuse a pass by the name it was asked for, whatever kind of name it is

A selection is keyed rather than coloured, so the refusal says which pass was
not planned in exactly the spelling the caller used. Two colour-specific
sentences collapse into one, including the evasive 'no planned pass without a
color' that only existed because a colourless pass and a single pass shared
one key."
```

---

### Task 5: The CLI chooses a grouping and names passes by key

**Files:**
- Modify: `crates/cli/src/main.rs:44-54` (the three flags), `:113-131` (dispatch), `:155-188` (`cut_by_color`)
- Modify: `crates/cli/src/pipeline.rs:71-96` (`pass_order`), `:98-151` (both planning entry points), `:168-205` (`parse_hex_color`, the scope check, `check_interactive`)
- Modify: `crates/cli/src/cut.rs:44-50` (`format_pass_color`), `:99-106` (`pass_color`)
- Test: `crates/cli/src/pipeline.rs` and `crates/cli/src/cut.rs` (existing `mod tests`)

**Interfaces:**
- Consumes: `PassKey`, `Grouping`, `PassSelection { key, .. }`, `PlannedPass { key, .. }`.
- Produces: `--group-by`, `--skip-pass`, `--order`; `pipeline::plan_cut_from_svg(svg, driver, settings, grouping, skip_passes, order, allow_out_of_bounds)`, `pipeline::check_pass_flag_scope`, `cut::format_pass_key`.

- [ ] **Step 1: Write the failing tests**

In `crates/cli/src/pipeline.rs`'s `mod tests`, replace `colour_flags_are_refused_without_by_color` (`:367-380`) and `parse_hex_color_requires_eight_digits` (`:301-306`), and add the grouping cases:

```rust
    /// `--skip-pass` and `--order` name passes, and a single-pass cut has one pass whose
    /// name nobody needs. Refused rather than ignored: a flag that silently does nothing is
    /// how a cut ends up including a colour the operator thought they had skipped.
    #[test]
    fn pass_flags_are_refused_for_a_single_pass_cut() {
        assert_eq!(
            check_pass_flag_scope(&["color:ff0000ff".into()], &None, Grouping::Single),
            Err("--skip-pass applies to a grouped cut; --group-by single is one pass over every shape".into())
        );
        assert_eq!(
            check_pass_flag_scope(&[], &Some("color:ff0000ff".into()), Grouping::Single),
            Err("--order applies to a grouped cut; --group-by single is one pass over every shape".into())
        );
        for g in [Grouping::Color, Grouping::Stroke, Grouping::Fill, Grouping::LineType, Grouping::Preset] {
            assert!(check_pass_flag_scope(&["color:ff0000ff".into()], &Some("color:ff0000ff".into()), g).is_ok());
        }
    }

    /// `--order` puts named passes first in the order given, then everything else in
    /// planned order; `--skip-pass` removes. Keys, not colours, so a preset-grouped cut can
    /// be sequenced the same way a colour-grouped one always could.
    #[test]
    fn pass_order_sequences_and_skips_by_key() {
        let planned = passes(&[(RED, 0.0, 0.0), (BLUE, 20.0, 0.0)]);
        let keys = pass_order(&planned.passes, &[], Some("color:0000ffff".into())).unwrap();
        assert_eq!(keys, vec![PassKey::Color(Some(BLUE)), PassKey::Color(Some(RED))]);

        let keys = pass_order(&planned.passes, &["color:ff0000ff".into()], None).unwrap();
        assert_eq!(keys, vec![PassKey::Color(Some(BLUE))]);
    }

    /// A key that names no planned pass is refused by name rather than ignored — including
    /// a key from another mode, which needs no rule of its own because it simply is not
    /// there. `--order` used to drop unknown colours silently.
    #[test]
    fn an_order_key_that_names_no_pass_is_refused() {
        let planned = cutplan::plan_passes(&doc_from_svg(two_color_svg()).unwrap()).unwrap();
        let err = pass_order(&planned.passes, &[], Some("line-type:cut".into())).unwrap_err();
        assert!(err.contains("line-type:cut"), "{err}");
    }

    /// A malformed key is `PassKey`'s error, surfaced unchanged: one grammar means one
    /// error message, and the CLI is where a person types it.
    #[test]
    fn a_malformed_pass_key_is_refused_with_the_grammar() {
        let planned = cutplan::plan_passes(&doc_from_svg(two_color_svg()).unwrap()).unwrap();
        let err = pass_order(&planned.passes, &["ff0000ff".into()], None).unwrap_err();
        assert!(err.contains("is not a pass key"), "{err}");
    }

    /// The plain path still means one pass over everything, and now says so with a mode.
    #[test]
    fn the_default_grouping_plans_one_pass_named_all() {
        let two_fills = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <rect width="5" height="5" fill="#ff0000"/><rect x="6" width="5" height="5" fill="#00ff00"/></svg>"##;
        let plan = plan_cut_from_svg(two_fills, cameo5().as_ref(), &cut_settings(),
            Grouping::Single, &[], None, false).unwrap();
        assert_eq!(plan.passes.len(), 1);
        assert_eq!(plan.passes[0].key, PassKey::All);
    }

    /// And a grouped cut still sees both fills, through the caller the CLI uses.
    #[test]
    fn colour_grouping_plans_a_pass_per_visible_paint() {
        let two_fills = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <rect width="5" height="5" fill="#ff0000"/><rect x="6" width="5" height="5" fill="#00ff00"/></svg>"##;
        let plan = plan_cut_from_svg(two_fills, cameo5().as_ref(), &cut_settings(),
            Grouping::Color, &[], None, false).unwrap();
        assert_eq!(plan.passes.len(), 2);
    }

    /// The TTY rule is about passes, not about a flag name: one pass never pauses, so it is
    /// allowed unattended whichever mode produced it.
    #[test]
    fn an_unattended_multi_pass_cut_is_refused() {
        assert_eq!(
            check_interactive(false, 2),
            Err("a cut with more than one pass requires an interactive terminal".into())
        );
        assert!(check_interactive(false, 1).is_ok());
        assert!(check_interactive(true, 2).is_ok());
    }
```

In `crates/cli/src/cut.rs`'s `mod tests`, update the two prompt assertions (`:290-297`, `:312-314`) to the key spelling:

```rust
    /// The prompt names the pass the way every other surface does. `#0000ff` was a second
    /// spelling of one key, invented here and nowhere else.
    #[test]
    fn a_swap_prompt_names_the_pass_by_its_key() {
        // `plan` and `status` are the module's existing helpers; `plan` now takes keys.
        let plan = plan(&[PassKey::Color(Some(0xFF0000FF)), PassKey::Color(Some(0x0000FFFF)), PassKey::All]);
        let at_second = status(
            Actions { cancel: true, resume: true, ..Actions::default() },
            Phase::AwaitingColorSwap,
            Some(PassPosition { index: 1, total: 3 }),
        );
        assert_eq!(pause_prompt(Pause::Swap, &plan, &at_second),
            "Pass 2/3 (color:0000ffff): swap tool, press Enter to resume");
    }

    /// A pass index the plan does not have degrades to a readable label rather than
    /// panicking a live cut — the prompt is cosmetic, the cut is not.
    #[test]
    fn a_prompt_for_a_pass_outside_the_plan_says_unknown() {
        let plan = plan(&[PassKey::All]);
        let outside = status(
            Actions { cancel: true, resume: true, ..Actions::default() },
            Phase::AwaitingColorSwap,
            Some(PassPosition { index: 7, total: 9 }),
        );
        assert!(pause_prompt(Pause::Swap, &plan, &outside).contains("(unknown pass)"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cli`

Expected: compile errors — `cannot find function 'check_pass_flag_scope'`, and `plan_cut_from_svg` taking the wrong number of arguments.

- [ ] **Step 3: Write the implementation**

`crates/cli/src/main.rs` — replace the three flags (`:44-54`):

```rust
        /// How to split the cut into passes: single (one pass over everything), color
        /// (stroke where visible, else fill), stroke, fill, line-type, or preset
        #[arg(long, value_enum, default_value_t = GroupBy::Single)]
        group_by: GroupBy,
        /// Skip cutting the pass with this key (e.g. color:ff0000ff, preset:cameo5-htv);
        /// may be repeated
        #[arg(long = "skip-pass")]
        skip_pass: Vec<String>,
        /// Comma-separated pass keys to cut first, in this order; the rest follow in
        /// planned order
        #[arg(long)]
        order: Option<String>,
```

with a `clap::ValueEnum` next to `Command` that maps to `cutplan::Grouping` — clap owns the flag's spelling (`single`, `color`, `stroke`, `fill`, `line-type`, `preset`), `cutplan` owns the enum:

```rust
/// The `--group-by` spellings. A separate enum from `cutplan::Grouping` so the CLI's
/// kebab-case flag values are clap's business and the planner's enum stays free of
/// presentation.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum GroupBy { Single, Color, Stroke, Fill, LineType, Preset }

impl From<GroupBy> for cutplan::Grouping {
    fn from(g: GroupBy) -> cutplan::Grouping {
        match g {
            GroupBy::Single => cutplan::Grouping::Single,
            GroupBy::Color => cutplan::Grouping::Color,
            GroupBy::Stroke => cutplan::Grouping::Stroke,
            GroupBy::Fill => cutplan::Grouping::Fill,
            GroupBy::LineType => cutplan::Grouping::LineType,
            GroupBy::Preset => cutplan::Grouping::Preset,
        }
    }
}
```

Rewrite the dispatch arm (`:115-132`) so one planning call serves both paths — the old `if !by_color` split existed because the plain path had its own planner call, and a mode makes that split nothing but a dry-run label difference:

```rust
        Command::Cut { file, device, dry_run, speed, force, port, baud, group_by, skip_pass, order, allow_out_of_bounds } => {
            let driver = driver_for(&device)?;
            let grouping: cutplan::Grouping = group_by.into();
            check_pass_flag_scope(&skip_pass, &order, grouping)?;
            let svg = std::fs::read(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
            let settings = Settings { speed, force, repeat_count: 1 };
            cut_planned(&svg, driver.as_ref(), &device, &settings, grouping, &skip_pass, order,
                        dry_run, port, baud, allow_out_of_bounds)
        }
```

Rename `cut_by_color` to `cut_planned`, give it a `grouping: cutplan::Grouping` parameter, pass it through to `plan_cut_from_svg`, and print the key:

```rust
    if dry_run {
        for (i, pass) in passes.iter().enumerate() {
            println!("-- pass {}/{} ({}) --", i + 1, passes.len(), pass.key);
            let bytes = dry_run_pass_bytes(driver, &pass.job, i, passes.len())?;
            print_hex_ascii(&bytes);
        }
        return Ok(());
    }
```

`crates/cli/src/pipeline.rs` — `pass_order` returns keys and refuses an unknown one:

```rust
/// The passes to cut, in cut order: apply `--order` (named passes to the front, in the
/// order given; the rest keep their planned order) and then `--skip-pass`.
///
/// An `--order` key that names no planned pass is refused rather than ignored. It used to
/// be dropped silently, which made a typo indistinguishable from a colour the document did
/// not contain — and with four kinds of key, a typo is likelier, not rarer.
pub fn pass_order(
    planned: &[cutplan::DocumentPass],
    skip_passes: &[String],
    order: Option<String>,
) -> Result<Vec<cutplan::PassKey>, String> {
    let mut keys: Vec<cutplan::PassKey> = planned.iter().map(|p| p.key.clone()).collect();

    if let Some(order) = order {
        let wanted: Vec<cutplan::PassKey> = order
            .split(',')
            .map(|s| s.trim().parse::<cutplan::PassKey>())
            .collect::<Result<_, _>>()?;
        let mut front = vec![];
        for key in wanted {
            let Some(i) = keys.iter().position(|k| *k == key) else {
                return Err(format!("--order names {key}, which is not a pass this file plans"));
            };
            front.push(keys.remove(i));
        }
        front.extend(keys);
        keys = front;
    }

    let skip: Vec<cutplan::PassKey> = skip_passes
        .iter()
        .map(|s| s.trim().parse::<cutplan::PassKey>())
        .collect::<Result<_, _>>()?;
    keys.retain(|k| !skip.contains(k));
    Ok(keys)
}
```

`plan_cut_from_svg` gains `grouping` and drops the plain/grouped split; `plan_plain_cut` and `parse_hex_color` are deleted — `plan_plain_cut` is `plan_cut_from_svg(.., Grouping::Single, &[], None, ..)`, and colour parsing now lives in `PassKey::from_str`:

```rust
/// Plan a cut from an SVG: import, group, order, select, and validate through
/// `cutplan::plan_cut` — the same entry point the desktop uses, so the CLI gets preflight
/// rather than sending unchecked geometry at the machine.
///
/// One entry point for every mode. `Grouping::Single` used to have its own function because
/// the plain path did its own planning; with the mode named explicitly there is nothing left
/// for a second function to say.
pub fn plan_cut_from_svg(
    svg: &[u8],
    driver: &dyn Driver,
    settings: &Settings,
    grouping: cutplan::Grouping,
    skip_passes: &[String],
    order: Option<String>,
    allow_out_of_bounds: bool,
) -> Result<cutplan::CutPlan, String> {
    let doc = doc_from_svg(svg)?;
    // Planned once: --order and --skip-pass name passes, so the keys have to be known
    // before a selection can be built, and plan_cut cuts the very passes handed to it here.
    let planned = cutplan::plan_passes_with(&doc, grouping).map_err(|e| e.to_string())?;
    // Checked here rather than left to `plan_cut`: with no passes at all, every selection is
    // unmatched, and "no planned pass is called all" describes the request instead of the file.
    if planned.passes.is_empty() {
        return Err("no cuttable paths in SVG".into());
    }
    let keys = pass_order(&planned.passes, skip_passes, order)?;

    // ponytail: one `--speed`/`--force` pair applies to every pass; the CLI has no per-pass
    // settings and no presets. Per-pass settings would need a flag that names a pass key.
    let passes = keys
        .into_iter()
        .map(|key| cutplan::PassSelection { key, settings: settings.clone() })
        .collect();

    // No revision to be stale against: the document was imported a few lines ago.
    let opts = cutplan::PlanOptions { passes, expect_revision: None, allow_out_of_bounds };
    cutplan::plan_cut(&planned, driver.profile(), &driver.caps(), &opts).map_err(describe_cut_error)
}
```

The scope check and the TTY check stop naming a flag that no longer exists:

```rust
/// `--skip-pass` and `--order` name passes, which only a grouped cut has more than one of.
/// A single-pass cut puts every shape in one pass, so these flags cannot do anything there
/// and are refused rather than ignored.
pub fn check_pass_flag_scope(
    skip_passes: &[String],
    order: &Option<String>,
    grouping: cutplan::Grouping,
) -> Result<(), String> {
    if grouping != cutplan::Grouping::Single {
        return Ok(());
    }
    if !skip_passes.is_empty() {
        return Err("--skip-pass applies to a grouped cut; --group-by single is one pass over every shape".into());
    }
    if order.is_some() {
        return Err("--order applies to a grouped cut; --group-by single is one pass over every shape".into());
    }
    Ok(())
}

/// More than one pass needs a human at the keyboard between passes; a plan with one pass
/// never pauses, so it is allowed even without a TTY.
pub fn check_interactive(is_tty: bool, pass_count: usize) -> Result<(), String> {
    if !is_tty && pass_count > 1 {
        return Err("a cut with more than one pass requires an interactive terminal".into());
    }
    Ok(())
}
```

`crates/cli/src/cut.rs` — the prompt names the key:

```rust
/// The pass the job is paused on, as the operator sees it named everywhere else.
///
/// A bad index cannot happen on the normal path (the reported position indexes the same
/// plan), but the prompt is cosmetic, so a mismatch degrades to a label rather than
/// panicking a process mid-cut.
pub fn format_pass_key(plan: &cutplan::CutPlan, status: &CutStatus) -> String {
    let index = status.pass.map(|p| p.index).unwrap_or(0);
    match plan.passes.get(index) {
        Some(pass) => pass.key.to_string(),
        None => "unknown pass".into(),
    }
}
```

and `pause_prompt` interpolates it directly: `format!("Pass {pass}/{total} ({key}): swap tool, press Enter to resume")`. Delete `format_pass_color` and `pass_color`.

Update `crates/cli/tests/plain_cut.rs` and `crates/cli/tests/dry_run.rs` call sites to the new `plan_cut_from_svg` signature (they call it directly with `&[], None`; add `cutplan::Grouping::Color` or `Single` to match what each test's comment says it is exercising).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cli --locked`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cli
git commit -m "Let the CLI ask for a grouping, and name a pass the way everything else does

--group-by replaces --by-color, --skip-pass replaces --skip-color, and --order
takes pass keys, so a preset-grouped cut can be sequenced the way a colour one
always could. An --order key that names no pass is now refused: it used to be
dropped silently, which made a typo look like a colour the file did not have.
The plain path loses its own planner call, since a mode says what it meant."
```

---

### Task 6: The desktop threads the grouping through all three planner calls

**Files:**
- Modify: `apps/desktop/src/device.rs:50-65` (`CutRequest`, `ConfiguredPassDto`), `:840-874` (`prepare_cut`), `:1103-1142` (`PlanCutResponse`, `plan_cut_response`), `:1147-1203` (`TravelPassDto`, `travel_for_order`)
- Modify: `apps/desktop/src/state.rs` (add `set_material_preset` beside `set_cut_line_type` at `:66`), `apps/desktop/src/ipc.rs:53-57` and `:138-145`, `apps/desktop/src/main.rs:47-92` (register the command)
- Test: `apps/desktop/src/device.rs` (existing `mod tests`)

**Interfaces:**
- Consumes: `PassKey`, `Grouping`, `PassSelection { key, .. }`, `document::commands::set_material_preset`.
- Produces: `plan_cut(state, grouping)`, `travel_for_order(state, doc_revision, grouping, passes)`, `cut(state, dev, request)` where `CutRequest` carries `grouping`, `set_material_preset(state, ids, value)`. Task 7 calls all four from TypeScript.

- [ ] **Step 1: Write the failing tests**

In `apps/desktop/src/device.rs`'s `mod tests`, update the two error-code assertions (`:1331-1340`, `:1403-1409`) to `"unknown_pass"`, and add:

```rust
    /// The grouping the dialog asked for is the grouping that gets cut. Without this the
    /// operator could preview a fill-grouped plan and cut a stroke-grouped one, because
    /// each command plans on its own.
    #[test]
    fn a_cut_honours_the_grouping_it_was_sent() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        // One shape with a red stroke and a green fill: the two colour modes key it
        // differently, so the request's grouping is observable in what matches.
        app.add_rect_with_style(10.0, 10.0, Some(RED), Some(GREEN));

        let planned = plan_cut_response(&app.editor.doc, Grouping::Fill).expect("premise: a plan exists");
        let stroke_key = "color:ff0000ff";
        let request = CutRequest {
            device_instance_id: dev.instance_id.clone(),
            doc_revision: planned.doc_revision.clone(),
            grouping: Grouping::Fill,
            passes: vec![enabled_pass(stroke_key)],
        };
        // Fill grouping keys that shape on its fill, so the stroke key names nothing.
        let err = dev.cut_from_request(&app, request).unwrap_err();
        assert_eq!(err.code, "unknown_pass");
    }

    /// Travel is replanned with the same grouping for the same reason, and a stale
    /// revision still wins over a key mismatch: the document changing is the more
    /// actionable fact.
    #[test]
    fn travel_honours_the_grouping_it_was_sent() {
        let mut app = AppState::new();
        app.add_rect_with_style(10.0, 10.0, Some(RED), Some(GREEN));
        let revision = plan_cut_response(&app.editor.doc).unwrap().doc_revision;

        assert!(travel_for_order(&app.editor.doc, &revision, Grouping::Fill,
            &[on("color:00ff00ff")]).is_ok());
        let err = travel_for_order(&app.editor.doc, &revision, Grouping::Fill,
            &[on("color:ff0000ff")]).unwrap_err();
        assert_eq!(err.code, "unknown_pass");
    }

    /// The response tells the dialog what to key its rows on, in the same spelling the
    /// request must send back.
    #[test]
    fn a_plan_response_names_its_passes_by_key() {
        let mut app = AppState::new();
        app.add_rect(10.0, 10.0);
        let response = plan_cut_response(&app.editor.doc, Grouping::Single).unwrap();
        assert_eq!(response.passes[0].key, "all");
    }
```

Add the two test helpers the cases above use, next to the existing ones: `enabled_pass(key: &str) -> ConfiguredPassDto` and `on(key: &str) -> TravelPassDto`, both parsing the key with `key.parse().unwrap()`; and `AppState::add_rect_with_style` if `add_rect` cannot set paint (follow whatever the existing helper does).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cuthulhu-desktop` (or the desktop crate's real name from its `Cargo.toml`)

Expected: compile errors — `no field 'grouping' on type 'CutRequest'`, and `plan_cut_response` taking one argument.

- [ ] **Step 3: Write the implementation**

DTOs carry the key and the mode:

```rust
#[derive(Deserialize)]
pub struct CutRequest {
    pub device_instance_id: String,
    pub doc_revision: String,
    /// How the dialog grouped the passes it is naming. Sent rather than remembered: the
    /// plan, the travel and the cut are three round trips, and a mode kept in `AppState`
    /// could be changed between them while the stale-plan check only guards the document.
    pub grouping: Grouping,
    pub passes: Vec<ConfiguredPassDto>,
}

#[derive(Deserialize)]
pub struct ConfiguredPassDto {
    pub key: PassKey,
    pub enabled: bool,
    pub preset_id: Option<String>,
    pub speed: Option<u32>,
    pub force: Option<u32>,
    pub repeat_count: Option<u32>,
}
```

In `prepare_cut`, the selection carries the key and the plan uses the request's grouping:

```rust
                PassSelection { key: dto.key.clone(), settings: resolve_settings(preset, &override_) }
```

```rust
        let planned = plan_passes_with(&app.editor.doc, request.grouping)
            .map_err(|e| IpcError::new("plan_error", e.to_string()))?;
```

`plan_cut_response` gains a mode. Keep the no-argument name as the `Grouping::Color` default only if a caller needs it; otherwise thread the parameter and rename:

```rust
#[derive(Debug, Serialize)]
pub struct PlanCutPassSummary {
    /// The pass's key, as the canonical string the dialog keys its rows on and sends back
    /// in a cut request. A string rather than a tagged object so the CLI, this DTO and the
    /// dialog all hold one spelling.
    pub key: PassKey,
    pub shape_count: usize,
    pub node_ids: Vec<document::NodeId>,
    pub starts: Vec<Option<[f64; 2]>>,
}

/// Summarizes `plan_passes_with` output for the UI — not the raw `DocumentPasses` (which
/// carries full flattened polylines the cut dialog doesn't need).
///
/// Takes the grouping rather than defaulting it: unlike `cutplan::plan_passes`, this has no
/// caller that means "whatever the default is" — the dialog always has a mode selected.
pub fn plan_cut_response(doc: &document::Document, grouping: Grouping)
    -> Result<PlanCutResponse, IpcError> {
    let planned = plan_passes_with(doc, grouping).map_err(|e| IpcError::new("plan_error", e.to_string()))?;
    let refs: Vec<&DocumentPass> = planned.passes.iter().collect();
    let travel = cutplan::travel_moves(&refs);
    Ok(PlanCutResponse {
        passes: planned.passes.iter().map(|p| PlanCutPassSummary {
            key: p.key.clone(),
            shape_count: p.shapes.len(),
            node_ids: p.shapes.iter().map(|s| s.node_id).collect(),
            starts: p.shapes.iter().map(|s| {
                s.polylines.first().and_then(|p| p.first()).map(|pt| [pt.x, pt.y])
            }).collect(),
        }).collect(),
        skipped_not_cut: planned.skipped_not_cut,
        doc_revision: planned.doc_revision.to_string(),
        travel: travel.into_iter().map(|(a, b)| [a.x, a.y, b.x, b.y]).collect(),
    })
}
```

`travel_for_order` takes the mode and matches keys — including the duplicate-versus-unknown distinction, which is about identity and so is unchanged in shape:

```rust
pub fn travel_for_order(
    doc: &document::Document,
    doc_revision: &str,
    grouping: Grouping,
    configured: &[TravelPassDto],
) -> Result<Vec<[f64; 4]>, IpcError> {
```

with `plan_passes_with(doc, grouping)` inside, `remaining`/`refs` as `Vec<&DocumentPass>`, and the lookup:

```rust
        let Some(i) = remaining.iter().position(|p| p.key == pass.key) else {
            return Err(if planned.passes.iter().any(|p| p.key == pass.key) {
                IpcError::new("plan_mismatch", "the requested pass list does not name every planned pass exactly once")
            } else {
                map_cut_error(CutError::UnknownPass(pass.key.clone()))
            });
        };
```

`TravelPassDto::color` becomes `pub key: PassKey`.

`state.rs`, mirroring `set_cut_line_type` (`:66-73`) — the whole method:

```rust
    pub fn set_material_preset(&mut self, ids: Vec<NodeId>, value: Option<String>)
        -> Result<Delta, CmdError> {
        let d = commands::set_material_preset(&self.editor.doc, &ids, value)?;
        // Same rule as `set_cut_line_type`: an empty delta is a no-op the operator asked
        // for, and committing it would clear the redo stack and add an undo step that does
        // nothing.
        if d.0.is_empty() { return Ok(d); }
        Ok(self.editor.commit(d))
    }
```

`ipc.rs` — one thin command each, and the two planner commands gain the parameter:

```rust
#[tauri::command]
pub fn set_material_preset(state: tauri::State<AppStateHandle>, ids: Vec<NodeId>, value: Option<String>)
    -> Result<Delta, String> {
    state.lock().unwrap().set_material_preset(ids, value).map_err(|e| format!("{e:?}"))
}
```

```rust
#[tauri::command]
pub fn plan_cut(state: tauri::State<AppStateHandle>, grouping: Grouping) -> Result<PlanCutResponse, IpcError> {
    plan_cut_response(&state.lock().unwrap().editor.doc, grouping)
}

#[tauri::command]
pub fn travel_for_order(
    state: tauri::State<AppStateHandle>,
    doc_revision: String,
    grouping: Grouping,
    passes: Vec<TravelPassDto>,
) -> Result<Vec<[f64; 4]>, IpcError> {
    // Fully qualified because the command and the function it forwards to share a name.
    crate::device::travel_for_order(&state.lock().unwrap().editor.doc, &doc_revision, grouping, &passes)
}
```

(keep whatever the existing bodies do about locking; only the signature and the forwarded argument change.)

Register `ipc::set_material_preset` in `main.rs`'s `generate_handler!` list, immediately after `ipc::set_cut_line_type` (`:56`).

**`{e:?}` is deliberate here** — #93 owns replacing `Debug` with `Display` across all eleven editor/file commands, and doing one of them differently would leave two conventions in one file.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. This is the first task where the whole workspace builds again since Task 3.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src
git commit -m "Send the grouping with every cut request, so three round trips agree

plan_cut, travel_for_order and cut each plan the document themselves; a mode
kept in AppState could change between them and the stale-plan check would not
notice, so the preview would show one arrangement and the machine cut another.
Also exposes set_material_preset, the second per-node production attribute."
```

---

### Task 7: The TypeScript wire speaks in keys

**Files:**
- Modify: `apps/desktop/ui/src/ipc.ts:148-162` (`PlanCutPassSummary`), `:209-225` (`planCut`, `TravelPass`, `travelForOrder`), and the `setCutLineType` neighbourhood at `:41-43` (add `setMaterialPreset`)
- Modify: `apps/desktop/ui/src/cut/viewmodel.ts:8-38` (`PassVm`, `ConfiguredPassDto`, `CutRequest`), `:166-170` (`toTravelPasses`), `:232-249` (`toCutRequest`), and add `parsePassKey`
- Test: `apps/desktop/ui/src/cut/viewmodel.test.ts`

**Interfaces:**
- Consumes: the Rust DTOs from Task 6.
- Produces: `PassKey` (a string alias), `Grouping`, `ParsedPassKey`, `parsePassKey`, `PassVm.key`, `planCut(grouping)`, `travelForOrder(docRevision, grouping, passes)`, `setMaterialPreset(args)`. Tasks 8 and 9 consume them.

- [ ] **Step 1: Write the failing tests**

Add to `apps/desktop/ui/src/cut/viewmodel.test.ts`:

```ts
describe("parsePassKey", () => {
  // The same table as crates/cutplan/src/pass_key.rs's round-trip test. These two tables
  // are the only thing keeping the dialog and the planner agreed on what a pass is called,
  // so a variant added on one side must be added here.
  it.each([
    ["all", { kind: "all" }],
    ["color:ff0000ff", { kind: "color", color: 0xff0000ff }],
    ["color:none", { kind: "color", color: null }],
    ["line-type:cut", { kind: "lineType", lineType: "Cut" }],
    ["line-type:no-cut", { kind: "lineType", lineType: "NoCut" }],
    ["preset:cameo5-htv", { kind: "preset", presetId: "cameo5-htv" }],
    ["preset:none", { kind: "preset", presetId: null }],
  ])("parses %s", (key, expected) => {
    expect(parsePassKey(key)).toEqual(expected);
  });

  it("keeps a colon inside a preset id", () => {
    expect(parsePassKey("preset:vinyl:thin")).toEqual({ kind: "preset", presetId: "vinyl:thin" });
  });

  // A key the backend produced that this cannot read is a backend/frontend mismatch, not
  // operator input. It renders as itself rather than throwing, because a dialog that
  // crashes mid-cut is worse than one showing a string nobody recognises.
  it("returns the raw key it cannot parse", () => {
    expect(parsePassKey("line-type:draw")).toEqual({ kind: "unknown", raw: "line-type:draw" });
  });
});

describe("toTravelPasses", () => {
  it("names every row by key, disabled ones included", () => {
    const rows = [
      { key: "color:00ff00ff", enabled: false },
      { key: "all", enabled: true },
    ];
    expect(toTravelPasses(rows)).toEqual([
      { key: "color:00ff00ff", enabled: false },
      { key: "all", enabled: true },
    ]);
  });
});

describe("toCutRequest", () => {
  it("sends the grouping alongside the keyed passes", () => {
    const rows: PassVm[] = [
      { key: "preset:cameo5-htv", shapeCount: 2, enabled: true, presetId: null,
        speed: null, force: null, repeatCount: null },
    ];
    expect(toCutRequest("dev-1", "42", "Preset", rows)).toEqual({
      device_instance_id: "dev-1",
      doc_revision: "42",
      grouping: "Preset",
      passes: [{ key: "preset:cameo5-htv", enabled: true, preset_id: null,
                 speed: null, force: null, repeat_count: null }],
    });
  });
});
```

Update every existing `PassVm` literal in that file (`reorderPass` at `:129-205`, `reorderForReplan` at `:209-231`, `effectiveSettings` at `:250-338`, `toCutRequest` at `:383-430`) to carry `key: "color:ff0000ff"`-style values instead of `color: 0xff0000ff`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm --prefix apps/desktop/ui test -- viewmodel`

Expected: FAIL — `parsePassKey is not a function`, and type errors on `key`.

- [ ] **Step 3: Write the implementation**

`apps/desktop/ui/src/ipc.ts`:

```ts
/** A pass's name, in the canonical form `cutplan::PassKey` writes: `all`, `color:ff0000ff`,
 *  `color:none`, `line-type:cut`, `line-type:no-cut`, `preset:<id>`, `preset:none`. Sent
 *  back verbatim in a travel or cut request — the string *is* the identity. */
export type PassKey = string;

/** How the planner splits shapes into passes. Mirrors `cutplan::Grouping`. */
export type Grouping = "Single" | "Color" | "Stroke" | "Fill" | "LineType" | "Preset";

export type PlanCutPassSummary = {
  key: PassKey;
  shape_count: number;
  node_ids: number[];
  /** Each shape's first world-space point, parallel to node_ids — where the blade lands.
   *  null is a shape whose outline flattened to nothing. */
  starts: ([number, number] | null)[];
};
```

```ts
export async function planCut(grouping: Grouping): Promise<PlanCutResponse> {
  return invoke("plan_cut", { grouping });
}

/** A pass as the dialog has it configured: where it sits in the order, and whether it is cut. */
export type TravelPass = { key: PassKey; enabled: boolean };

export async function travelForOrder(
  docRevision: string,
  grouping: Grouping,
  passes: TravelPass[],
): Promise<[number, number, number, number][]> {
  return invoke("travel_for_order", { docRevision, grouping, passes });
}
```

and beside `setCutLineType`:

```ts
export async function setMaterialPreset(args: Args) {
  return invoke("set_material_preset", args);
}
```

`apps/desktop/ui/src/cut/viewmodel.ts`:

```ts
export type PassVm = {
  key: PassKey;
  shapeCount: number;
  enabled: boolean;
  presetId: string | null;
  speed: number | null;
  force: number | null;
  repeatCount: number | null;
};

// Wire types (match Rust ConfiguredPassDto and CutRequest)
export type ConfiguredPassDto = {
  key: PassKey;
  enabled: boolean;
  preset_id: string | null;
  speed: number | null;
  force: number | null;
  repeat_count: number | null;
};

export type CutRequest = {
  device_instance_id: string;
  doc_revision: string;
  grouping: Grouping;
  passes: ConfiguredPassDto[];
};

/** What a `PassKey` says, for the one thing the UI needs from inside it: a swatch needs the
 *  RGBA, a row label needs the preset id. The mirror of `cutplan::PassKey::from_str` — the
 *  example table in `viewmodel.test.ts` is what keeps the two agreed. */
export type ParsedPassKey =
  | { kind: "all" }
  | { kind: "color"; color: number | null }
  | { kind: "lineType"; lineType: "Cut" | "NoCut" }
  | { kind: "preset"; presetId: string | null }
  | { kind: "unknown"; raw: string };

export function parsePassKey(key: PassKey): ParsedPassKey {
  if (key === "all") return { kind: "all" };
  // First separator only, so a preset id may contain one — same rule as the Rust parser.
  const at = key.indexOf(":");
  if (at === -1) return { kind: "unknown", raw: key };
  const mode = key.slice(0, at);
  const value = key.slice(at + 1);
  if (mode === "color") {
    if (value === "none") return { kind: "color", color: null };
    // Eight digits exactly: a shorter string would parse to a colour no shape carries.
    if (/^[0-9a-fA-F]{8}$/.test(value)) return { kind: "color", color: parseInt(value, 16) };
    return { kind: "unknown", raw: key };
  }
  if (mode === "line-type") {
    if (value === "cut") return { kind: "lineType", lineType: "Cut" };
    if (value === "no-cut") return { kind: "lineType", lineType: "NoCut" };
    return { kind: "unknown", raw: key };
  }
  if (mode === "preset") {
    return { kind: "preset", presetId: value === "none" ? null : value };
  }
  return { kind: "unknown", raw: key };
}
```

`toTravelPasses` and `toCutRequest` carry the key and the mode:

```ts
export function toTravelPasses<T extends { key: PassKey; enabled: boolean }>(
  rows: T[],
): { key: PassKey; enabled: boolean }[] {
  return rows.map((r) => ({ key: r.key, enabled: r.enabled }));
}
```

```ts
export function toCutRequest(
  deviceInstanceId: string,
  docRevision: string,
  grouping: Grouping,
  passes: PassVm[],
): CutRequest {
  return {
    device_instance_id: deviceInstanceId,
    doc_revision: docRevision,
    grouping,
    passes: passes.map((p) => ({
      key: p.key,
      enabled: p.enabled,
      preset_id: p.presetId,
      speed: p.speed,
      force: p.force,
      repeat_count: p.repeatCount,
    })),
  };
}
```

Import `PassKey` and `Grouping` from `../ipc` in `viewmodel.ts`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npm --prefix apps/desktop/ui test`

Expected: PASS for `viewmodel`; `CutDialog`-level failures are Task 8's.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src/ipc.ts apps/desktop/ui/src/cut/viewmodel.ts apps/desktop/ui/src/cut/viewmodel.test.ts
git commit -m "Carry a pass key across IPC as the one string both sides already write

The wire type is the canonical spelling rather than a mirrored union, so the
dialog keys rows on exactly what the planner named and sends it back verbatim.
parsePassKey exists for the two things the UI needs from inside a key — a
swatch's RGBA and a preset id — and its example table matches the Rust one."
```

---

### Task 8: The cut dialog offers the choice

**Files:**
- Modify: `apps/desktop/ui/src/cut/CutDialog.tsx:167-202` (`replan`), `:363-395` (travel refresh, enable), `:548-625` (rows and the skipped sentence), plus the grouping control and its state
- Modify: `apps/desktop/ui/src/cut/CutPreview.tsx:19-25` (`PreviewPass.color` → `key`), `:75-83,118-130` (pass colour from a parsed key)
- Test: `apps/desktop/ui/src/cut/viewmodel.test.ts` (the row-label helper)

**Interfaces:**
- Consumes: `parsePassKey`, `PassVm.key`, `planCut(grouping)`, `travelForOrder(docRevision, grouping, passes)`, `toCutRequest(dev, revision, grouping, rows)`.
- Produces: `passRowLabel(key, presets)` in `viewmodel.ts` — the pure half of a row's label, so the JSX stays a rendering of it.

- [ ] **Step 1: Write the failing test**

Add to `viewmodel.test.ts`:

```ts
describe("passRowLabel", () => {
  const presets = [{ id: "cameo5-htv", name: "HTV", machine_id: "cameo5",
                     settings: { speed: 5, force: 20, repeat_count: 1 }, builtin: true }];

  it("names a colour pass by its swatch, not by words", () => {
    expect(passRowLabel("color:ff0000ff", presets)).toEqual({ swatch: "#ff0000", text: null });
  });

  it("says what the colourless pass holds, since no swatch can", () => {
    expect(passRowLabel("color:none", presets)).toEqual({ swatch: null, text: "No visible paint" });
  });

  it("names the single pass for what it is", () => {
    expect(passRowLabel("all", presets)).toEqual({ swatch: null, text: "Every shape" });
  });

  it("resolves a preset to its name", () => {
    expect(passRowLabel("preset:cameo5-htv", presets)).toEqual({ swatch: null, text: "HTV" });
  });

  // A preset a document names but the preset file no longer has: the planner keys the pass
  // anyway (a user entry can be deleted), so the dialog has to render one.
  it("shows an unresolved preset id as unknown", () => {
    expect(passRowLabel("preset:deleted", presets)).toEqual({ swatch: null, text: "deleted (unknown preset)" });
  });

  it("names the unassigned-preset pass", () => {
    expect(passRowLabel("preset:none", presets)).toEqual({ swatch: null, text: "No preset" });
  });

  it("names a line-type pass", () => {
    expect(passRowLabel("line-type:cut", presets)).toEqual({ swatch: null, text: "Cut lines" });
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm --prefix apps/desktop/ui test -- viewmodel`

Expected: FAIL — `passRowLabel is not a function`.

- [ ] **Step 3: Write the implementation**

In `viewmodel.ts`:

```ts
/** How a pass row identifies itself: a swatch when the key is a colour, words otherwise.
 *  Pure so the wording is testable without rendering the dialog — the same split every
 *  other dialog here uses. */
export function passRowLabel(
  key: PassKey,
  presets: Preset[],
): { swatch: string | null; text: string | null } {
  const parsed = parsePassKey(key);
  switch (parsed.kind) {
    case "color":
      return parsed.color === null
        ? { swatch: null, text: "No visible paint" }
        // Drop the alpha byte: a swatch is a colour, and 0-alpha keys never reach here.
        : { swatch: `#${(parsed.color >>> 8).toString(16).padStart(6, "0")}`, text: null };
    case "all":
      return { swatch: null, text: "Every shape" };
    case "lineType":
      return { swatch: null, text: parsed.lineType === "Cut" ? "Cut lines" : "No-cut lines" };
    case "preset": {
      if (parsed.presetId === null) return { swatch: null, text: "No preset" };
      const preset = presets.find((p) => p.id === parsed.presetId);
      // An id the preset file no longer resolves is a real state, not a bug: presets are
      // machine-scoped and a user entry can be deleted while a document still names it.
      return { swatch: null, text: preset ? preset.name : `${parsed.presetId} (unknown preset)` };
    }
    case "unknown":
      return { swatch: null, text: parsed.raw };
  }
}
```

In `CutDialog.tsx`, hold the mode in state and pass it to all three calls:

```tsx
  // The dialog owns the mode and sends it with every request. Held here rather than in the
  // backend so the rows on screen and the mode that produced them are one piece of state.
  const [grouping, setGrouping] = useState<ipc.Grouping>("Color");
```

`replan` becomes `replan(mode = grouping)` and calls `ipc.planCut(mode)`, mapping `key: p.key` instead of `color: p.color`; the travel refresh calls `ipc.travelForOrder(planRevision, grouping, toTravelPasses(rows))`; the cut calls `toCutRequest(dev, planRevision, grouping, rows)`.

Add the control above the pass list:

```tsx
        <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
          Group passes by
          <select
            aria-label="Group passes by"
            value={grouping}
            onChange={(e) => {
              const next = e.target.value as ipc.Grouping;
              setGrouping(next);
              // Replanned with the new mode immediately: the rows, the skipped count, the
              // travel and the preview are all derived from it, and showing rows from the
              // previous mode beside the new selection is the disagreement this avoids.
              replan(next);
            }}
          >
            <option value="Color">Colour (stroke, else fill)</option>
            <option value="Stroke">Stroke colour</option>
            <option value="Fill">Fill colour</option>
            <option value="Preset">Material preset</option>
            <option value="LineType">Line type</option>
            <option value="Single">One pass over everything</option>
          </select>
        </label>
```

The row keys off the pass key and renders the label:

```tsx
              const label = passRowLabel(row.key, presets);
              …
              <div key={row.key} data-testid="cut-pass-row" style={{…}}>
                {label.swatch !== null ? (
                  <span style={{ width: 12, height: 12, display: "inline-block", background: label.swatch }} />
                ) : null}
                {label.text !== null ? <span>{label.text}</span> : null}
                <span>{row.shapeCount} shape(s)</span>
```

In `CutPreview.tsx`, the pass's draw colour comes from the parsed key rather than a numeric field: `const parsed = parsePassKey(pass.key); const color = parsed.kind === "color" && parsed.color !== null ? cssColor(parsed.color) : textColor;` — keeping the existing fallback exactly as it is for every non-colour key, which is what a non-colour pass has always looked like.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npm --prefix apps/desktop/ui test` then `npm --prefix apps/desktop/ui run build`

Expected: PASS, and a clean build.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src/cut apps/desktop/ui/dist
git commit -m "Give the operator the grouping control, and a row that can name itself

A pass keyed on a preset or a line type has no swatch to be recognised by, so
the row says what it holds instead — including an unresolved preset id, which
is a real state because a user preset can be deleted while a document names it.
Changing the mode replans at once: rows, skipped count, travel and preview all
come from it, and a stale row list beside a new mode is the lie to avoid."
```

---

### Task 9: The operator can assign a material preset

**Files:**
- Create: `apps/desktop/ui/src/panels/materialPreset.ts`
- Modify: `apps/desktop/ui/src/panels/PropertiesPanel.tsx:1-51`, and `App.tsx` where `cutLineType`/`onChangeCutLineType` are wired
- Test: `apps/desktop/ui/src/panels/materialPreset.test.ts` (create, mirroring the cut-line-type helper's tests if they exist)

**Interfaces:**
- Consumes: `ipc.setMaterialPreset`, `Preset` from `viewmodel.ts`.
- Produces: `selectionMaterialPreset(nodes, selected)` returning `string | null | "mixed"`, and the panel control.

- [ ] **Step 1: Write the failing test**

Create `apps/desktop/ui/src/panels/materialPreset.test.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { selectionMaterialPreset } from "./materialPreset";

const shape = (id: number, material_preset: string | null) => ({
  id, kind: { Shape: { Rect: { w: 1, h: 1 } } }, transform: [1, 0, 0, 1, 0, 0],
  style: { stroke: 255, fill: null }, cut_line_type: "Cut" as const, material_preset,
  children: [] as number[],
});
const group = (id: number, children: number[]) => ({
  id, kind: "Group" as const, transform: [1, 0, 0, 1, 0, 0],
  style: { stroke: 255, fill: null }, cut_line_type: "Cut" as const,
  material_preset: null, children,
});

describe("selectionMaterialPreset", () => {
  it("reports the one value a selection agrees on", () => {
    const nodes = { "1": shape(1, "cameo5-htv"), "2": shape(2, "cameo5-htv") };
    expect(selectionMaterialPreset(nodes as never, [1, 2])).toBe("cameo5-htv");
  });

  it("reports mixed when the shapes disagree", () => {
    const nodes = { "1": shape(1, "cameo5-htv"), "2": shape(2, null) };
    expect(selectionMaterialPreset(nodes as never, [1, 2])).toBe("mixed");
  });

  // Walks into containers for the same reason the cut-line-type helper does: the value is
  // read on the shape, so a Group's own null would show a control that does nothing.
  it("reads the shapes under a selected container", () => {
    const nodes = { "1": group(1, [2]), "2": shape(2, "cameo5-htv") };
    expect(selectionMaterialPreset(nodes as never, [1])).toBe("cameo5-htv");
  });

  // Distinct from "no preset assigned", which is `null` on a shape that exists.
  it("returns undefined when there is no shape to speak for", () => {
    expect(selectionMaterialPreset({} as never, [])).toBeUndefined();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm --prefix apps/desktop/ui test -- materialPreset`

Expected: FAIL — module not found.

- [ ] **Step 3: Write the implementation**

Create `apps/desktop/ui/src/panels/materialPreset.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { DocNode } from "../App";

/// What to show for a selection: the one id every shape agrees on, `null` for "no preset",
/// `"mixed"` when they disagree, or `undefined` when there is no shape to speak for.
/// `null` and `undefined` are different answers — one is a shape with no material, the
/// other is nothing selected — and the panel renders them differently.
///
/// Mirrors `commands::set_material_preset`, which walks into containers because the
/// attribute is read on shapes: a panel that read a Group's own value would show an inert one.
export function selectionMaterialPreset(
  nodes: Record<string, DocNode>,
  selected: number[],
): string | null | "mixed" | undefined {
  const values = new Set<string | null>();
  const seen = new Set<number>();
  const stack = [...selected];
  while (stack.length > 0) {
    const id = stack.pop()!;
    if (seen.has(id)) continue;
    seen.add(id);
    const node = nodes[String(id)];
    if (!node) continue;
    if (typeof node.kind === "object" && "Shape" in node.kind) values.add(node.material_preset ?? null);
    else stack.push(...node.children);
  }
  if (values.size === 0) return undefined;
  return values.size === 1 ? [...values][0] : "mixed";
}
```

Add `material_preset: string | null` to the `DocNode` type in `App.tsx` (beside `cut_line_type`), add the two props to `PropertiesPanel`, and render the control beside the cuttability checkbox:

```tsx
      {materialPreset !== undefined ? (
        <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
          Material
          <select
            aria-label="Material preset"
            value={materialPreset === "mixed" ? "" : materialPreset ?? ""}
            onChange={(e) => onChangeMaterialPreset(e.target.value === "" ? null : e.target.value)}
          >
            {/* A mixed selection shows this empty option as selected rather than picking a
                side; choosing it commits "no preset", which one undo reverses. */}
            <option value="">{materialPreset === "mixed" ? "Mixed" : "No preset"}</option>
            {presets.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
          </select>
        </label>
      ) : null}
```

Wire it in `App.tsx` the way `onChangeCutLineType` is wired: call `ipc.setMaterialPreset({ ids: selection, value })`, apply the returned `Delta`, and refresh the snapshot — copy the existing handler's shape exactly rather than inventing a second pattern. The panel needs the machine's presets, which the app already loads for the cut dialog; pass that list down.

Update the `bounds === null && cutLineType === null` "No selection" condition to also require `materialPreset === undefined`, so a selection that has a material never shows "No selection".

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npm --prefix apps/desktop/ui test` then `npm --prefix apps/desktop/ui run build`

Expected: PASS, clean build.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src apps/desktop/ui/dist
git commit -m "Let the operator put a material on a Node, so preset grouping has input

Mirrors the cuttability control, including walking into containers: the value
is read on the shape, so assigning it to a Layer has to reach the shapes under
it or the control would look inert. 'No preset' and 'nothing selected' are
different answers and render differently."
```

---

### Task 10: The e2e fake tells the truth again

**Files:**
- Modify: `apps/desktop/ui/e2e/smoke.spec.ts:1-70` (the fake's `Node` type and fixtures), `:302-341` (`planFromDoc`), `:459-505` (the three handlers), and every pass/travel/cut assertion at `:707,733,774,798,823,842,864`
- Modify: the stale comment at `:468-472` (#143)

**Interfaces:**
- Consumes: the real DTO shapes from Tasks 6 and 7.
- Produces: nothing — this is the suite catching up to the backend it mirrors.

- [ ] **Step 1: Write the failing assertions**

Update the fake's fixtures to carry the new field, and the pass assertions to keys. The two-colour test at `:707` becomes:

```ts
    // The fake groups by the mode the dialog asked for, so this asserts the same thing the
    // real planner would: two visible strokes, two passes, keyed the way the planner keys them.
    const keys = await page.evaluate(() => (window as unknown as {
      __travelRequests: { key: string; enabled: boolean }[][]
    }).__travelRequests.at(-1)?.map((p) => p.key));
    expect(keys).toEqual(["color:ff0000ff", "color:00ff00ff"]);
```

and add a mode test:

```ts
  test("changing the grouping replans and renames the passes", async ({ page }) => {
    await openCutDialog(page);
    await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

    await page.getByLabel("Group passes by").selectOption("Single");
    // One pass, named for what it holds rather than for a colour it does not have.
    await expect(page.getByTestId("cut-pass-row")).toHaveCount(1);
    await expect(page.getByText("Every shape")).toBeVisible();
  });
```

- [ ] **Step 2: Run the suite to verify it fails**

Run: `npm --prefix apps/desktop/ui run e2e`

Expected: FAIL — the fake still returns `color`, and the dialog has no grouping control in the fake's world.

- [ ] **Step 3: Update the fake**

`planFromDoc` takes the grouping and keys the way the planner does:

```ts
  // Mirrors crates/cutplan/src/passes.rs's plan_passes_with: preorder walk, skip Shape leaf
  // nodes whose CutLineType is NoCut, and key the rest as the grouping asks — a colour
  // (stroke where visible, else fill, with 0-alpha counting as absent; strict under Stroke
  // and Fill), the line type, the inherited material preset, or `all` for one pass.
  function planFromDoc(grouping: Grouping = "Color") {
    const byKey = new Map<string, { key: string; node_ids: number[] }>();
    let skipped = 0;
    const visible = (c: number | null | undefined) => ((c ?? 0) & 0xff) !== 0 ? c! : null;
    const colorKey = (n: Node) => {
      const stroke = visible(n.style.stroke);
      const fill = visible(n.style.fill);
      const c = grouping === "Stroke" ? stroke : grouping === "Fill" ? fill : stroke ?? fill;
      return c === null ? "color:none" : `color:${(c >>> 0).toString(16).padStart(8, "0")}`;
    };
    const walk = (id: number, inherited: string | null) => {
      const n = doc.nodes[id];
      if (!n) return;
      const isShape = typeof n.kind === "object" && n.kind !== null && "Shape" in (n.kind as object);
      const preset = n.material_preset ?? inherited;
      if (isShape) {
        if (n.cut_line_type === "NoCut") {
          skipped++;
        } else {
          const key =
            grouping === "Single" ? "all"
            : grouping === "LineType" ? `line-type:${n.cut_line_type === "Cut" ? "cut" : "no-cut"}`
            : grouping === "Preset" ? `preset:${preset ?? "none"}`
            : colorKey(n);
          const existing = byKey.get(key);
          if (existing) existing.node_ids.push(id);
          else byKey.set(key, { key, node_ids: [id] });
        }
      }
      for (const c of n.children) walk(c, preset);
    };
    walk(doc.root, null);
    …
  }
```

(keep the `starts`-all-null comment and the snapshot-as-revision comment verbatim; only the keying changes.)

The three handlers take the grouping and key on it — and the stale comment #143 filed gets fixed here, which is the ride-along it was filed for:

```ts
    plan_cut: (a) => {
      const plan = planFromDoc(a.grouping as Grouping);
      …
    },
    // Mirrors device::travel_for_order's contract, not its geometry: the same stale-plan
    // refusal, then synthetic segments (one per adjacent pair of *enabled* passes, x
    // encoding the position in the order) — the real command does not route the head to a
    // pass that will not be cut. Received lists are recorded on `window.__travelRequests`
    // so a test can assert what the dialog asked for, including the grouping; travel itself
    // lands on a canvas Playwright cannot read.
    travel_for_order: (a) => {
      const passes = a.passes as { key: string; enabled: boolean }[];
      …
      const stale = planFromDoc(a.grouping as Grouping).doc_revision !== a.docRevision;
      …
    },
    cut: (a) => {
      const request = a.request as { device_instance_id: string; doc_revision: string;
        grouping: Grouping; passes: { key: string; enabled: boolean }[] };
      …
      const plan = planFromDoc(request.grouping);
      …
    },
```

Add `material_preset: null` to every fixture Node the fake builds (`:1-70`), and `material_preset: string | null` to its `Node` type.

- [ ] **Step 4: Run the suite to verify it passes**

Run: `npm --prefix apps/desktop/ui run e2e` then `cargo test --workspace --locked`

Expected: PASS on both.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/e2e/smoke.spec.ts
git commit -m "Teach the e2e fake to key passes and honour a grouping, and fix its comment

The fake re-implements plan_passes, so a grouping it ignores makes the suite
green on a dialog that cannot work. Also settles #143: the comment named a
hook that had been renamed to __travelRequests and a rule that had changed to
enabled-passes-only."
```

---

### Task 11: The vocabulary says what the code now does

**Files:**
- Modify: `CONTEXT.md:40-47` (the ColorPass entry), `:59-62` (PassSelection), and add PassKey
- Modify: `CLAUDE.md:79-99` (the cut-path section), `:154-160` (the vocabulary list)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: everything above.
- Produces: no code.

- [ ] **Step 1: Update `CONTEXT.md`**

Replace the **ColorPass** entry with **DocumentPass** and add **PassKey** and **Grouping** beside it:

```markdown
**DocumentPass**:
Every shape in a Document cut in a single run of the blade, together with the PassKey that
says which pass it is. What the shapes have in common is whatever the Grouping asked for —
a colour, a line type, a material preset — or nothing but the operator's request, for the
single pass. Which shapes are cut at all is their CutLineType's business, not their paint's.
_Avoid_: ColorPass (retired with #148), layer, colour group, batch

**PassKey**:
What a DocumentPass is called: `all` for the single pass, a colour, a CutLineType, or a
MaterialPreset id — each with one canonical spelling (`color:ff0000ff`, `line-type:cut`,
`preset:cameo5-htv`) that the CLI, the IPC payloads and the cut dialog all use. Absence
inside a key is a real value: `color:none` is a shape with no visible paint, `preset:none`
one nobody assigned a material to.
_Avoid_: pass id, pass name, colour

**Grouping**:
How a Document's cut shapes are split into DocumentPasses — by stroke colour, fill colour,
stroke-else-fill, material preset, line type, or not at all. A request, not a property of
the Document: the same Document plans differently under two Groupings, and the mode travels
with every cut request so a preview and a cut cannot disagree about it.
_Avoid_: mode, split, pass strategy
```

Update the **PassSelection** entry: it names a pass "by its PassKey", not "by colour".

- [ ] **Step 2: Update `CLAUDE.md`**

In the cut-path section, the plain-CLI paragraph (`:90-97`) now reads as `--group-by single` rather than "no `--by-color`", and the pass-identity sentence gains the key. In the conventions list (`:156-160`), replace `ColorPass` with `DocumentPass`, `PassKey` and `Grouping`.

- [ ] **Step 3: Update `CHANGELOG.md`**

Follow the file's existing format. The operator-visible facts: grouping is selectable in the cut dialog and with `--group-by`; a Node can carry a material preset; `--by-color` and `--skip-color` are gone, replaced by `--group-by` and `--skip-pass`; `--order` takes pass keys.

- [ ] **Step 4: Verify**

Run: `cargo test --workspace --locked && npm --prefix apps/desktop/ui test && grep -rn "ColorPass\|by-color\|skip-color" --include="*.rs" --include="*.ts" --include="*.tsx" --include="*.md" . | grep -v CHANGELOG`

Expected: tests PASS, and the grep returns nothing outside `CHANGELOG.md` (where the removed flags are named as removed). A surviving `ColorPass` is a rename this plan missed.

- [ ] **Step 5: Commit**

```bash
git add CONTEXT.md CLAUDE.md CHANGELOG.md
git commit -m "Retire ColorPass from the vocabulary, since a pass is no longer a colour

CONTEXT.md is normative, so the term the code no longer has cannot stay in it.
DocumentPass, PassKey and Grouping replace it, and PassSelection now names a
pass by its key."
```

---

## Verification

The plan is done when, from a clean tree on `vcolombo/pass-grouping`:

- `cargo test --workspace --locked` passes and `Cargo.lock` is unchanged.
- `npm --prefix apps/desktop/ui test` and `npm --prefix apps/desktop/ui run e2e` pass, and `apps/desktop/ui/dist` is committed and current.
- `cuthulhu cut file.svg --device cameo5 --dry-run` prints one pass labelled `all`; `--group-by color --dry-run` prints one labelled pass per visible paint; `--group-by preset --skip-pass preset:none --dry-run` refuses by name when nothing carries a preset.
- The grep in Task 11 Step 4 returns nothing.
- Hardware verification is **not** required and nothing is added to `apps/desktop/MANUAL-CHECKLIST.md`: every decision here lands before a byte reaches a Transport, and the `MockTransport`/dry-run paths cover it. A real multi-pass cut grouped by preset is worth doing opportunistically, not blocking on.
