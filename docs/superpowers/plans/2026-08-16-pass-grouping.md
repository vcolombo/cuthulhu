<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Pass grouping as an explicit choice — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an operator choose how a Document's shapes are split into passes — by stroke colour, fill colour, material preset, today's stroke-else-fill rule, or one pass over everything — by making a pass's identity a `PassKey` instead of an `Option<u32>` colour, and giving a Node an inheritable material assignment to group on. Closes #148.

**Architecture:** `PassKey { All, Color(Option<u32>), Preset(Option<String>) }` replaces the colour that identifies a pass in ten places from `cutplan::passes` to the e2e fake, and it crosses every boundary as one canonical, **injective** string (`all`, `color:ff0000ff`, `no-color`, `preset:cameo5-htv`, `no-preset`) so the CLI, the JSON DTOs and the dialog's row keys cannot disagree. `ColorPass` becomes `DocumentPass`. `Grouping` becomes `{Single, Color, Stroke, Fill, Preset}` with `Color` (stroke-else-fill, today's rule) staying the library default. `Node` gains `material_preset: PresetAssignment` — a three-state `Inherit`/`Unassigned`/`Preset(id)` — resolved down `plan_passes_with`'s existing preorder walk, while the editing command writes only the selection. The chosen grouping rides in the `plan_cut`, `travel_for_order` and `cut` payloads, and the dialog holds it with the rows it produced as one installed-plan state.

**Tech Stack:** Rust 2021 (`document`, `fileio`, `cutplan`, `cli`, `apps/desktop`), React + TypeScript (`apps/desktop/ui`), Playwright for e2e. **No new dependencies** in either language.

**Spec:** `docs/superpowers/specs/2026-08-16-pass-grouping-design.md` — read it first. It holds the reference-application research, why absence is its own grammar token, why `PresetAssignment` has three states, why assignment does not descend, why line-type grouping is *not* in this slice, and the alternatives that were rejected. Its *Revisions* section lists the four decisions a Codex review overturned; this plan implements the revised ones.

> **Two of this plan's contracts were overturned during review, after it was written. The task
> bodies below still state the original ones and are deliberately left as they were** — this file is
> a record of the instructions the implementation was given, not a description of what shipped. Read
> the spec's *Revisions* section for the reasoning; the shipped contract is:
>
> 1. **`preset:` parses, as an empty preset id.** Tasks 1, 3 and 10 say it is malformed and must be
>    refused (`:147`, `:153`, `:240`, `:1746`, `:1951`). Refusing it made the grammar non-total —
>    `Display` wrote a string `FromStr` rejected — and the first fix for *that* let an empty id
>    reach a blade with default settings. The class is closed where an empty id could act instead:
>    assignment refuses it (`crates/document/src/commands.rs:91`), preset loading drops it
>    (`crates/cutplan/src/presets.rs:198`), and `prepare_cut` refuses it
>    (`apps/desktop/src/device.rs:865`).
> 2. **An unresolved preset is refused at Cut, not cut with fallback settings.** Task 5 says it falls
>    back to override-or-default (`:1796`). `prepare_cut` does a machine-scoped lookup and returns
>    `unknown_preset` (`apps/desktop/src/device.rs:844`): cutting real material with settings
>    unrelated to the pass's own name is not a safe default.

## Global Constraints

**Reading the code blocks in this plan:** a block is the complete text of what it introduces unless it contains a bare `…` line. A `…` appears only inside a block quoting an *existing* function this plan modifies, and means "the surrounding lines are unchanged — do not retype them". Every such block names the file and line range it edits. There are no placeholders: nothing here is left for the implementer to invent.

- **SPDX header on every file** — `// SPDX-License-Identifier: GPL-3.0-or-later` (`<!-- -->` in Markdown, `//` in Rust and TypeScript). Every file this plan touches already has one; new files need one.
- **`cargo test --workspace --locked` is the gate**, and `--locked` is mandatory. This change adds no dependency, so `Cargo.lock` must not change.
- **`ui/dist` is committed.** Any task that edits `apps/desktop/ui/src` must end with `npm --prefix apps/desktop/ui run build` and commit `apps/desktop/ui/dist` in the same commit — CI rebuilds and fails on a stale bundle. Tasks 6, 7, 8 and 9 touch `ui/src`.
- **`CONTEXT.md` is normative vocabulary**, and this change edits it (Task 10). **ColorPass is retired** and replaced by **DocumentPass** plus **PassKey**, **Grouping** and **PresetAssignment**; do not leave both names alive.
- **Comments explain why, not what.** Every comment specified below records a constraint, a trap, or a decision taken against an alternative.
- **`// ponytail:` marks a deliberate simplification** with its ceiling and upgrade path. One is specified here: the CLI's single settings pair for every pass.
- **The e2e fake mirrors the real backend** (`CLAUDE.md:135-138`). `apps/desktop/ui/e2e/smoke.spec.ts` re-implements `plan_passes` at `:302-341`; a fake that ignores the grouping or skips the identity check makes the suite green on a frontend that cannot work.
- **`Grouping::Color` stays what `plan_passes` defaults to.** Existing behaviour must not move.
- **The grouping must reach all three planner call sites** — `plan_cut_response`, `travel_for_order`, `prepare_cut` — *and* the dialog must not be able to send rows from one mode under another. Both halves are required; the payload alone is not enough.
- **Out of scope, and must not creep in:** separate confirmed jobs versus one continuous job (#149), colour-layer alignment marks (#150), `Grouping::LineType`/`PassKey::LineType` and `CutEdge` (#56), a second per-node "production role" enum (#68 settled that the role *is* `CutLineType`), a configurable import default (#54), generated IPC types (#70), per-pass CLI settings, and replacing `{e:?}` in `apps/desktop/src/ipc.rs` (#93 owns all eleven sites; doing one differently leaves two conventions in one file).

## File Structure

| File | Responsibility after this change |
|---|---|
| `crates/cutplan/src/pass_key.rs` | **New.** Owns `PassKey`, its injective canonical grammar (`Display`/`FromStr`), and its serde string representation. Kept out of `passes.rs`, which is already 582 lines and owns the walk. |
| `crates/cutplan/src/passes.rs` | `DocumentPass { key, shapes }`, `Grouping`'s five modes, the key rule per mode, and preset resolution in the walk. |
| `crates/cutplan/src/plan.rs` | `PassSelection { key, settings }`, `PlannedPass { key, job }`, `CutError::UnknownPass(PassKey)` with code `unknown_pass`. |
| `crates/cutplan/src/preflight.rs` | Unchanged rules; `ConfiguredPass::pass` is a `&DocumentPass`. |
| `crates/document/src/node.rs` | `PresetAssignment` and `Node::material_preset`, defaulting to `Inherit` through `NodeWire`. |
| `crates/document/src/commands.rs` | `set_material_preset`, which writes the selection and never descends. |
| `crates/cli/src/main.rs` | `--group-by`, repeatable `--skip-pass` and `--order` over pass keys; the dry-run header rule. |
| `crates/cli/src/pipeline.rs` | `pass_order` over keys with both flags validated, `check_pass_flag_scope`, and one planning entry point taking a `Grouping`. |
| `crates/cli/src/cut.rs` | The operator prompt names a pass by its key. |
| `apps/desktop/src/device.rs` | Cut DTOs carry `key` and `grouping`; the three planner call sites take a `Grouping`. |
| `apps/desktop/src/{state,ipc,main}.rs` | `set_material_preset`, and the grouping parameter on `plan_cut`/`travel_for_order`. |
| `apps/desktop/ui/src/ipc.ts` | `PassKey`, `Grouping`, `PresetAssignmentJson`, and the three cut callers. |
| `apps/desktop/ui/src/cut/viewmodel.ts` | `PassVm.key`, `parsePassKey`, `passRowLabel`, `presetIdForKey`, and the request builders. |
| `apps/desktop/ui/src/cut/{CutDialog,CutPreview}.tsx` | The grouping picker, one installed-plan state, grouping-aware row labels, a swatch from a parsed key. |
| `apps/desktop/ui/src/panels/{materialPreset.ts,PropertiesPanel.tsx}` | The three-state per-Node material control and the pure helpers behind it. |
| `apps/desktop/ui/src/App.tsx` | Owns the preset list (today it is local to the cut dialog) so the panel and the dialog read one copy. |
| `apps/desktop/ui/e2e/smoke.spec.ts` | The fake keys passes, honours the grouping, mirrors the exact-once identity check, and implements `set_material_preset`. |
| `CONTEXT.md`, `CLAUDE.md`, `CHANGELOG.md`, `apps/desktop/MANUAL-CHECKLIST.md` | Vocabulary, cut-path prose, operator-visible changes, and the two live checks that name removed flags. |

**Task order is load-bearing.** Tasks 1 and 2 add things nothing reads yet. Task 3 is the cutover — planner *and* selection in one commit, because the type change cannot be split below that pair without leaving the workspace unbuildable (a Codex review confirmed even `cargo test -p cutplan --lib passes` cannot compile with `plan.rs` still on colours: Cargo compiles every library test module before filtering by name). Tasks 4 and 5 give the two binaries the choice. Tasks 6–8 are the UI in dependency order. Task 9 makes the e2e fake tell the truth. Task 10 updates the vocabulary the change contradicts. **Every task ends green on `cargo test --workspace --locked`**, and every task touching `ui/src` also ends green on `npm --prefix apps/desktop/ui test`.

---

### Task 1: `PassKey` and its injective grammar

**Files:**
- Create: `crates/cutplan/src/pass_key.rs`
- Modify: `crates/cutplan/src/lib.rs` (add `mod pass_key;` and re-export as the neighbouring modules are re-exported)

**Interfaces:**
- Consumes: nothing.
- Produces: `cutplan::PassKey` with `Display`, `FromStr` (`Err = String`), `From<PassKey> for String`, `TryFrom<String>`, and `Serialize`/`Deserialize` as the canonical string. Task 3 keys passes with it, Task 4 parses CLI values into it, Task 5 serializes it, Task 6 mirrors it in TypeScript.

- [ ] **Step 1: Write the failing tests**

Create `crates/cutplan/src/pass_key.rs` with the SPDX header and this test module only:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Every variant, both directions, plus the JSON form. The same table appears in
    /// TypeScript (`apps/desktop/ui/src/cut/viewmodel.test.ts`) because the string is the
    /// only thing that crosses the boundary — if the two tables disagree, the dialog and the
    /// planner disagree about what a pass is called.
    #[test]
    fn every_key_round_trips_through_its_canonical_string() {
        let table = [
            (PassKey::All, "all"),
            (PassKey::Color(Some(0xFF0000FF)), "color:ff0000ff"),
            (PassKey::Color(Some(0x0000FFFF)), "color:0000ffff"),
            (PassKey::Color(None), "no-color"),
            (PassKey::Preset(Some("cameo5-htv".into())), "preset:cameo5-htv"),
            (PassKey::Preset(None), "no-preset"),
        ];
        for (key, text) in table {
            assert_eq!(key.to_string(), text);
            assert_eq!(text.parse::<PassKey>().unwrap(), key);
            assert_eq!(serde_json::to_string(&key).unwrap(), format!("\"{text}\""));
            assert_eq!(serde_json::from_str::<PassKey>(&format!("\"{text}\"")).unwrap(), key);
        }
    }

    /// The property the first spelling lacked. A preset id is an unrestricted operator
    /// string (`presets.rs:9-15`), so `preset:none` for "no preset" collided with a preset
    /// whose id is literally `none` — two passes writing one string, which is duplicate
    /// React keys and a `plan_mismatch` no operator can clear. Absence is its own token.
    #[test]
    fn no_two_keys_write_the_same_string() {
        let keys = [
            PassKey::All,
            PassKey::Color(Some(0xFF0000FF)),
            PassKey::Color(None),
            PassKey::Preset(None),
            PassKey::Preset(Some("none".into())),
            PassKey::Preset(Some("no-preset".into())),
            PassKey::Preset(Some("all".into())),
            PassKey::Preset(Some("vinyl:thin".into())),
            PassKey::Preset(Some("with,comma".into())),
        ];
        let mut seen: HashMap<String, PassKey> = HashMap::new();
        for key in &keys {
            let text = key.to_string();
            assert_eq!(text.parse::<PassKey>().unwrap(), *key, "round trip {text}");
            if let Some(prev) = seen.insert(text.clone(), key.clone()) {
                panic!("{prev:?} and {key:?} both write {text}");
            }
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

    /// A preset id may contain the separator, because ids are the operator's and nothing
    /// validates them. Splitting on the first `:` is what allows that.
    #[test]
    fn a_preset_id_may_contain_the_separator() {
        let key: PassKey = "preset:vinyl:thin".parse().unwrap();
        assert_eq!(key, PassKey::Preset(Some("vinyl:thin".into())));
        assert_eq!(key.to_string(), "preset:vinyl:thin");
    }

    /// Refused rather than coerced, and quoting the input: these arrive from a person typing
    /// `--skip-pass`. `preset:` is refused because an empty tail is the one id that would be
    /// indistinguishable from a truncated key; `color:none` and `line-type:cut` are refused
    /// because they are the retired spellings, and accepting them quietly would resurrect the
    /// collision and a mode that no longer exists.
    #[test]
    fn a_malformed_key_is_refused_by_name() {
        for bad in ["", "all:1", "preset:", "color:", "color:zz", "color:ff0000",
                    "color:none", "line-type:cut", "no-material", "colour:ff0000ff"] {
            let err = bad.parse::<PassKey>().expect_err("must not parse");
            assert!(err.contains(bad), "{err} should quote the input");
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cutplan pass_key`

Expected: compile errors — `cannot find type 'PassKey' in this scope`.

- [ ] **Step 3: Write the implementation**

Above the test module:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
//! What a pass is called, and the one string form that name takes everywhere.

use serde::{Deserialize, Serialize};

/// What a `DocumentPass` is keyed on — the answer to "which pass is this?" for whichever
/// `Grouping` produced it.
///
/// `All` is deliberately not `Color(None)`. Before #148 the single pass a `Grouping::Single`
/// plan holds and the pass of shapes with no visible paint were both keyed `None`, so a
/// refusal could only say the evasive "no planned pass without a color".
///
/// The `Option`s sit inside their variants rather than a shared `Unassigned`, because absence
/// is a property of that mode's key: `Color(None)` is a shape with no visible paint *in the
/// mode's terms*, and `Preset(None)` is a shape resolving to no material — the ordinary
/// state, not an error.
///
/// Serialized as its canonical string (`Display`/`FromStr`) rather than as a tagged enum: the
/// CLI needs a human grammar, the cut dialog needs a stable row key, and the e2e fake has to
/// produce byte-identical values. One representation cannot drift from itself.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum PassKey {
    All,
    Color(Option<u32>),
    Preset(Option<String>),
}

impl std::fmt::Display for PassKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PassKey::All => write!(f, "all"),
            // Lowercase, always: `FromStr` accepts either case, so writing one of them is
            // what makes the round trip a fixed point.
            PassKey::Color(Some(c)) => write!(f, "color:{c:08x}"),
            PassKey::Preset(Some(id)) => write!(f, "preset:{id}"),
            // Absence is its own token, never a reserved value inside a mode's namespace: a
            // preset id is an unrestricted operator string, so `preset:none` would collide
            // with a preset actually called `none`. `no-color` follows the same rule for one
            // grammar rather than two, even though 8 hex digits could not have collided.
            PassKey::Color(None) => write!(f, "no-color"),
            PassKey::Preset(None) => write!(f, "no-preset"),
        }
    }
}

impl std::str::FromStr for PassKey {
    type Err = String;
    fn from_str(s: &str) -> Result<PassKey, String> {
        let unknown = || format!("'{s}' is not a pass key (all, color:RRGGBBAA, no-color, preset:<id>, no-preset)");
        // First separator only: a preset id may contain a colon, so the grammar must not
        // constrain ids further than the application does.
        let Some((mode, value)) = s.split_once(':') else {
            return match s {
                "all" => Ok(PassKey::All),
                "no-color" => Ok(PassKey::Color(None)),
                "no-preset" => Ok(PassKey::Preset(None)),
                _ => Err(unknown()),
            };
        };
        match (mode, value) {
            // Eight digits exactly: a 6-digit RRGGBB would parse as 0x00RRGGBB — a colour no
            // shape carries — and match nothing while looking like it should.
            ("color", hex) if hex.len() == 8 => u32::from_str_radix(hex, 16)
                .map(|c| PassKey::Color(Some(c)))
                .map_err(|_| format!("'{s}' has a colour that is not 8 hex digits (RRGGBBAA)")),
            ("color", _) => Err(format!("'{s}' has a colour that is not 8 hex digits (RRGGBBAA)")),
            // An empty id is the one tail that would make two keys indistinguishable.
            ("preset", "") => Err(format!("'{s}' names an empty preset id")),
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

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. Nothing consumes `PassKey` yet.

- [ ] **Step 5: Commit**

```bash
git add crates/cutplan/src/pass_key.rs crates/cutplan/src/lib.rs
git commit -m "Give a pass a name that is not a colour, in one spelling nothing else can write

#148 needs a key that can also be a material preset, and the CLI, the IPC
payloads, the dialog's row keys and the e2e fake must agree on what a pass is
called. Absence is its own token rather than a reserved value inside a mode:
preset ids are unrestricted operator strings, so a preset called `none` would
otherwise write the same string as no preset at all."
```

---

### Task 2: A Node carries a material assignment

**Files:**
- Modify: `crates/document/src/node.rs:44-63` (`Node` and its two constructors), plus `NodeWire` and its `From` impl
- Modify: `crates/document/src/commands.rs` (add `set_material_preset` after `set_cut_line_type`, `:55-81`)
- Test: `crates/document/src/node.rs` and `crates/document/src/commands.rs` (existing `mod tests`), `crates/fileio/src/project.rs` (existing `mod tests`, beside `a_project_saved_before_cuttability_derives_it_from_stroke`)

**Interfaces:**
- Consumes: nothing.
- Produces: `document::PresetAssignment { Inherit, Unassigned, Preset(String) }`, `Node::material_preset: PresetAssignment`, and `document::commands::set_material_preset(&Document, &[NodeId], PresetAssignment) -> Result<Delta, CmdError>`. Task 3 resolves it in the walk, Task 5 exposes the command over IPC, Task 8 renders its control.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/document/src/node.rs`:

```rust
    /// A new Node inherits. There is no import default to argue about: a material is the
    /// operator's choice per shape or per Layer, and inheriting is the state that lets a
    /// Layer's choice reach the shapes under it.
    #[test]
    fn a_new_node_inherits_its_material() {
        let mut ids = IdGen::default();
        assert_eq!(Node::shape(ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 }).material_preset,
            PresetAssignment::Inherit);
        assert_eq!(Node::container(ids.next(), NodeKind::Group).material_preset,
            PresetAssignment::Inherit);
    }

    /// Three states, and the wire form each takes. `Unassigned` is the one the two-state
    /// spelling could not express: a shape deliberately carrying no material *inside* an
    /// assigned Layer, which is a pass an operator can otherwise never reach.
    #[test]
    fn a_material_assignment_round_trips_in_all_three_states() {
        for (value, json) in [
            (PresetAssignment::Inherit, r#"{"state":"inherit"}"#),
            (PresetAssignment::Unassigned, r#"{"state":"unassigned"}"#),
            (PresetAssignment::Preset("cameo5-htv".into()), r#"{"state":"preset","id":"cameo5-htv"}"#),
        ] {
            assert_eq!(serde_json::to_string(&value).unwrap(), json);
            assert_eq!(serde_json::from_str::<PresetAssignment>(json).unwrap(), value);
        }
    }

    /// A document written before the field existed had no way to assign a material, so
    /// absence means inherit — and an explicit `null` means the same, which serde gives for
    /// free and which is deliberate: nothing this workspace writes can produce one, so it
    /// only ever arrives from a hand-edited file.
    #[test]
    fn a_node_saved_without_a_material_assignment_inherits() {
        let json = r#"{"id":7,"kind":{"Shape":{"Rect":{"w":1.0,"h":1.0}}},
                       "transform":[1.0,0.0,0.0,1.0,0.0,0.0],
                       "style":{"stroke":255,"fill":null},
                       "cut_line_type":"Cut","children":[]}"#;
        let node: Node = serde_json::from_str(json).unwrap();
        assert_eq!(node.material_preset, PresetAssignment::Inherit);
        assert_eq!(node.cut_line_type, CutLineType::Cut, "premise: the other attribute still decodes");

        let nulled = json.replace(r#""cut_line_type":"Cut""#, r#""cut_line_type":"Cut","material_preset":null"#);
        assert_eq!(serde_json::from_str::<Node>(&nulled).unwrap().material_preset,
            PresetAssignment::Inherit);
    }

    /// Written on every save, so the field stops being absent the first time a document is
    /// written by this version and is never ambiguous again.
    #[test]
    fn a_material_assignment_is_always_written() {
        let mut node = Node::shape(NodeId(1), ShapeKind::Rect { w: 1.0, h: 1.0 });
        node.material_preset = PresetAssignment::Preset("cameo5-htv".into());
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains(r#""material_preset":{"state":"preset","id":"cameo5-htv"}"#), "{json}");
        assert_eq!(serde_json::from_str::<Node>(&json).unwrap(), node);
    }
```

Append to `mod tests` in `crates/fileio/src/project.rs`, beside the cuttability migration test #144 added — serde in isolation cannot see the container an operator's file actually is:

```rust
    /// The migration at the level an operator experiences it: a real `.cut` written before
    /// the field existed. The manifest is pruned rather than hand-written, so the fixture
    /// cannot drift from `Document`'s real shape.
    #[test]
    fn a_project_saved_before_material_assignments_inherits() {
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
        assert_eq!(back.get(shape_id).unwrap().material_preset, document::PresetAssignment::Inherit);
        assert_eq!(back.get(shape_id).unwrap().cut_line_type, document::CutLineType::Cut,
            "premise: the neighbouring migration still runs");
    }
```

`design.svg` is deliberately absent: `load_project` reads only `manifest.json` (`crates/fileio/src/project.rs:29`), so a container without it is a valid old file for this test's purpose.

Append to `mod tests` in `crates/document/src/commands.rs`:

```rust
    /// Writes the selection and nothing else. This is the opposite of `set_cut_line_type`,
    /// which descends — and the difference is the whole point: a `CutLineType` does not
    /// inherit, so a value on a Group would be inert, while a material *does*. Descending
    /// here would set today's shapes and leave the Layer holding nothing, after which a
    /// shape added to it would disagree with its siblings.
    #[test]
    fn set_material_preset_writes_the_selected_layer_and_not_its_children() {
        let mut doc = Document::new();
        let layer = Node::container(doc.ids.next(), NodeKind::Layer);
        let layer_id = layer.id;
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![
            NodeOp::Add { parent: doc.root, node: layer, index: usize::MAX },
            NodeOp::Add { parent: layer_id, node: shape, index: usize::MAX },
        ]));

        let delta = set_material_preset(&doc, &[layer_id],
            PresetAssignment::Preset("cameo5-htv".into())).unwrap();
        doc.apply(delta);
        assert_eq!(doc.get(layer_id).unwrap().material_preset,
            PresetAssignment::Preset("cameo5-htv".into()));
        assert_eq!(doc.get(shape_id).unwrap().material_preset, PresetAssignment::Inherit,
            "the child still inherits — resolution is the planner's, not a stored copy's");
    }

    /// A container and a shape inside it, both selected: both get the value. Nothing about
    /// the overlap is special, because nothing descends.
    #[test]
    fn set_material_preset_writes_every_selected_node_once() {
        let mut doc = Document::new();
        let layer = Node::container(doc.ids.next(), NodeKind::Layer);
        let layer_id = layer.id;
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![
            NodeOp::Add { parent: doc.root, node: layer, index: usize::MAX },
            NodeOp::Add { parent: layer_id, node: shape, index: usize::MAX },
        ]));

        let delta = set_material_preset(&doc, &[layer_id, shape_id, layer_id],
            PresetAssignment::Unassigned).unwrap();
        assert_eq!(delta.0.len(), 2, "one op per distinct node, duplicates ignored");
        doc.apply(delta);
        assert_eq!(doc.get(layer_id).unwrap().material_preset, PresetAssignment::Unassigned);
        assert_eq!(doc.get(shape_id).unwrap().material_preset, PresetAssignment::Unassigned);
    }

    /// `Unassigned` is a value, not a clear: it stops inheritance, where `Inherit` restores
    /// it. Both are reachable, because the panel offers both.
    #[test]
    fn set_material_preset_can_stop_or_restore_inheritance() {
        let mut doc = Document::new();
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        doc.apply(set_material_preset(&doc, &[shape_id], PresetAssignment::Unassigned).unwrap());
        assert_eq!(doc.get(shape_id).unwrap().material_preset, PresetAssignment::Unassigned);
        doc.apply(set_material_preset(&doc, &[shape_id], PresetAssignment::Inherit).unwrap());
        assert_eq!(doc.get(shape_id).unwrap().material_preset, PresetAssignment::Inherit);
    }

    /// Re-picking the value a selection already has emits nothing, so it cannot land an undo
    /// step that undoes nothing — the same rule `set_cut_line_type` follows.
    #[test]
    fn set_material_preset_emits_nothing_for_an_unchanged_selection() {
        let mut doc = Document::new();
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        assert!(set_material_preset(&doc, &[shape_id], PresetAssignment::Inherit).unwrap().0.is_empty());
        assert_eq!(set_material_preset(&doc, &[], PresetAssignment::Inherit),
            Err(CmdError::EmptySelection));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p document material` and `cargo test -p fileio material`

Expected: compile errors — `cannot find type 'PresetAssignment'`, `no field 'material_preset' on type 'Node'`, `cannot find function 'set_material_preset'`.

- [ ] **Step 3: Write the implementation**

In `crates/document/src/node.rs`, above `Node`:

```rust
/// Which `MaterialPreset` a Node's geometry is cut with, or where to look for one.
///
/// A sibling of `cut_line_type`, and not on `Style`, for the reason #68 settled: production
/// intent is not paint.
///
/// Three states rather than `Option<String>`, because the two-state spelling cannot say
/// "deliberately no material, do not inherit". With absence meaning inherit, a shape inside an
/// HTV Layer could never reach the no-preset pass — a pass that exists and resolves to the
/// operator's own settings. `cutplan::plan_passes_with` resolves the chain; nothing stores a
/// resolved value, so reparenting cannot leave a stale one.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "state", content = "id", rename_all = "kebab-case")]
pub enum PresetAssignment {
    /// Take the nearest ancestor's assignment; no ancestor means no material.
    #[default]
    Inherit,
    /// No material, whatever any ancestor says.
    Unassigned,
    /// This `MaterialPreset::id`. Never validated here: presets are machine-scoped and a
    /// user entry can be deleted, so an id that resolves to nothing is a real state.
    Preset(String),
}
```

Add the field to `Node` (after `cut_line_type` at `:51`) and to both constructors:

```rust
    pub cut_line_type: CutLineType,
    pub material_preset: PresetAssignment,
    pub children: Vec<NodeId>,
```

```rust
    pub fn shape(id: NodeId, kind: ShapeKind) -> Node {
        Node { id, kind: NodeKind::Shape(kind), transform: Affine::identity(),
               style: Style::default(), cut_line_type: CutLineType::Cut,
               material_preset: PresetAssignment::Inherit, children: vec![] }
    }
    pub fn container(id: NodeId, kind: NodeKind) -> Node {
        Node { id, kind, transform: Affine::identity(),
               style: Style::default(), cut_line_type: CutLineType::Cut,
               material_preset: PresetAssignment::Inherit, children: vec![] }
    }
```

In `NodeWire`, beside `cut_line_type`:

```rust
    /// A plain `#[serde(default)]`, unlike `cut_line_type` above: a document written before
    /// this field existed had no way to assign a material, so absence *is* `Inherit`. There
    /// is nothing to derive and no old behaviour to preserve.
    #[serde(default)]
    material_preset: Option<PresetAssignment>,
```

and in `From<NodeWire> for Node`: `material_preset: w.material_preset.unwrap_or_default(),`.

In `crates/document/src/commands.rs`, after `set_cut_line_type` (`:81`):

```rust
/// Assign `value` to every Node in `ids`, and to nothing else.
///
/// Deliberately *not* `set_cut_line_type`'s walk. That command descends into containers
/// because a `CutLineType` is read only on the shape that carries it, so a value on a Group
/// would be inert. A material assignment is the opposite: `cutplan::plan_passes_with`
/// resolves it down the tree, so writing a Layer's value is what makes every shape under it
/// cut with that material — including shapes added or reparented later, which a descent would
/// have left behind while making the Layer itself look assigned.
pub fn set_material_preset(doc: &Document, ids: &[NodeId], value: PresetAssignment)
    -> Result<Delta, CmdError> {
    if ids.is_empty() { return Err(CmdError::EmptySelection); }
    let mut ops = vec![];
    let mut seen = HashSet::new();
    for &id in ids {
        // A selection can name a node twice (a Layer and its shape are both ordinary
        // selections); one op each, or the inverse delta would undo through a duplicate.
        if !seen.insert(id) { continue; }
        let node = doc.get(id).ok_or(CmdError::NotFound)?;
        if node.material_preset == value { continue; }
        let before = node.clone();
        let mut after = before.clone();
        after.material_preset = value.clone();
        ops.push(NodeOp::Update { id, before, after });
    }
    Ok(Delta(ops))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. Every `Node` is built through the two constructors, so the new field needs no other edit; if a struct literal exists somewhere, the compiler names it.

- [ ] **Step 5: Commit**

```bash
git add crates/document/src/node.rs crates/document/src/commands.rs crates/fileio/src/project.rs
git commit -m "Let a Node say which material it is cut with, or that it inherits one

#148 groups passes by material, which needs somewhere to put one. Three states,
not an Option: with absence meaning inherit, a shape inside an assigned Layer
could never reach the no-preset pass. Assignment writes only the selection —
the neighbouring cut-line-type command descends precisely because that
attribute does not inherit, and copying it here would cancel out the
inheritance the planner does."
```

---

### Task 3: The planner and selection speak in keys

**Files:**
- Modify: `crates/cutplan/src/passes.rs:14-21` (`ColorPass` → `DocumentPass`), `:31-32`, `:67-100` (`pass_key`, `Grouping`, `plan_passes`), `:100-169` (the walk), `:175` (`travel_moves`), and its `mod tests` from `:193`
- Modify: `crates/cutplan/src/plan.rs:15-22`, `:38-44`, `:58-99`, `:125-152`, and its `mod tests` from `:154`
- Modify: `crates/cutplan/src/preflight.rs:5-11`, and its test helpers at `:227` and `:238`

**Interfaces:**
- Consumes: `PassKey` (Task 1), `PresetAssignment` (Task 2).
- Produces: `DocumentPass { key, shapes }`, `Grouping { Single, Color, Stroke, Fill, Preset }`, `plan_passes_with(&Document, Grouping)`, `PassSelection { key, settings }`, `PlannedPass { key, job }`, `CutError::UnknownPass(PassKey)` with code `unknown_pass`. Tasks 4–9 consume all of it.

**This task is one commit on purpose.** The type change cannot be split: `plan.rs` and `preflight.rs` name `ColorPass` and match on `color`, and Cargo compiles every library test module before filtering by test name, so no smaller boundary is even buildable.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/cutplan/src/passes.rs`:

```rust
    /// One document, five modes, and the key set each produces. The point of the table is
    /// that the modes differ only in what they key on: the same shapes are cut, in the same
    /// document order, and only the split changes.
    #[test]
    fn every_grouping_keys_the_same_shapes_differently() {
        let mut doc = Document::new();
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
            vec!["color:ff0000ff", "color:00ff00ff", "color:0000ffff", "no-color"]);
        // Strict: a shape with no visible stroke keys on no colour at all, which is the same
        // bucket a shape with no paint whatsoever lands in.
        assert_eq!(keys(Grouping::Stroke), vec!["color:ff0000ff", "color:00ff00ff", "no-color"]);
        assert_eq!(keys(Grouping::Fill), vec!["color:00ff00ff", "color:0000ffff", "no-color"]);
        assert_eq!(keys(Grouping::Preset), vec!["no-preset"]);

        for g in [Grouping::Single, Grouping::Color, Grouping::Stroke, Grouping::Fill, Grouping::Preset] {
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

        assert_eq!(plan_passes(&doc).unwrap().passes,
            plan_passes_with(&doc, Grouping::Color).unwrap().passes);
        assert_eq!(plan_passes(&doc).unwrap().passes[0].key, PassKey::Color(Some(RED)));
    }

    /// The three assignment states, resolved down the tree. `Unassigned` is the one that
    /// earns the enum: without it the second shape could not leave its Layer's pass.
    #[test]
    fn a_material_resolves_from_the_nearest_assigned_ancestor() {
        let mut doc = Document::new();
        let mut layer = Node::container(doc.ids.next(), NodeKind::Layer);
        layer.material_preset = PresetAssignment::Preset("cameo5-htv".into());
        let layer_id = layer.id;
        let inherits = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let mut refuses = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        refuses.material_preset = PresetAssignment::Unassigned;
        let mut overrides = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        overrides.material_preset = PresetAssignment::Preset("cameo5-vinyl-adhesive".into());
        let outside = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        doc.apply(Delta(vec![
            NodeOp::Add { parent: doc.root, node: layer, index: usize::MAX },
            NodeOp::Add { parent: layer_id, node: inherits, index: usize::MAX },
            NodeOp::Add { parent: layer_id, node: refuses, index: usize::MAX },
            NodeOp::Add { parent: layer_id, node: overrides, index: usize::MAX },
            NodeOp::Add { parent: doc.root, node: outside, index: usize::MAX },
        ]));

        let keys: Vec<String> = plan_passes_with(&doc, Grouping::Preset).unwrap()
            .passes.iter().map(|p| p.key.to_string()).collect();
        assert_eq!(keys, vec!["preset:cameo5-htv", "no-preset", "preset:cameo5-vinyl-adhesive"]);
    }

    /// Resolution lives in the walk, so a shape moved into an assigned Layer picks that
    /// Layer's material up with no edit of its own. A stored resolved value would have gone
    /// stale here, silently, and only shown up as the wrong settings on real material.
    #[test]
    fn a_reparented_shape_inherits_without_being_edited() {
        let mut doc = Document::new();
        let mut layer = Node::container(doc.ids.next(), NodeKind::Layer);
        layer.material_preset = PresetAssignment::Preset("cameo5-htv".into());
        let layer_id = layer.id;
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![
            NodeOp::Add { parent: doc.root, node: layer, index: usize::MAX },
            NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX },
        ]));
        assert_eq!(plan_passes_with(&doc, Grouping::Preset).unwrap().passes.len(), 2,
            "premise: outside the Layer it is its own pass");

        let moved = doc.get(shape_id).unwrap().clone();
        doc.apply(Delta(vec![
            NodeOp::Remove { parent: doc.root, node: moved.clone(), index: 1 },
            NodeOp::Add { parent: layer_id, node: moved, index: usize::MAX },
        ]));

        let keys: Vec<String> = plan_passes_with(&doc, Grouping::Preset).unwrap()
            .passes.iter().map(|p| p.key.to_string()).collect();
        assert_eq!(keys, vec!["preset:cameo5-htv"]);
    }

    /// An id no preset file resolves still keys a pass. Refusing here would put a
    /// settings-file concern behind `plan_cut`, and a user preset can be deleted while a
    /// document still names it.
    #[test]
    fn an_unknown_preset_id_still_keys_a_pass() {
        let mut doc = Document::new();
        let mut shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        shape.material_preset = PresetAssignment::Preset("deleted-by-the-operator".into());
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        assert_eq!(plan_passes_with(&doc, Grouping::Preset).unwrap().passes[0].key,
            PassKey::Preset(Some("deleted-by-the-operator".into())));
    }

    /// The predicate is still `cut_line_type`, still checked before the outline is resolved
    /// (#139) — a grouping mode changes the key, never that order.
    #[test]
    fn a_no_cut_shape_is_counted_under_every_grouping() {
        let mut doc = Document::new();
        let mut shape = Node::shape(doc.ids.next(), ShapeKind::Text {
            family: "no such family".into(), size_mm: 10.0, text: "x".into() });
        shape.cut_line_type = CutLineType::NoCut;
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        for g in [Grouping::Single, Grouping::Color, Grouping::Stroke, Grouping::Fill, Grouping::Preset] {
            let planned = plan_passes_with(&doc, g).unwrap();
            assert_eq!(planned.skipped_not_cut, 1, "{g:?}");
            assert!(planned.passes.is_empty(), "{g:?}");
        }
    }
```

Add `const GREEN: u32 = 0x00FF00FF;` beside the module's existing colour constants, and extend its `use` line with `CutLineType`, `NodeKind` and `PresetAssignment` as needed.

In `crates/cutplan/src/plan.rs`'s `mod tests`, replace `unknown_pass_color_is_refused_not_dropped` (`:213`) and the `UnknownPassColor` arms of `every_refusal_has_a_sentence_and_a_code` (`:292`):

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
```

and inside `every_refusal_has_a_sentence_and_a_code`, replacing its three `UnknownPassColor` assertions:

```rust
        for (key, sentence) in [
            (PassKey::Color(Some(0xFF0000FF)), "no planned pass is called color:ff0000ff"),
            (PassKey::Color(None), "no planned pass is called no-color"),
            (PassKey::All, "no planned pass is called all"),
            (PassKey::Preset(Some("cameo5-htv".into())), "no planned pass is called preset:cameo5-htv"),
            (PassKey::Preset(None), "no planned pass is called no-preset"),
        ] {
            let err = CutError::UnknownPass(key);
            assert_eq!(err.code(), "unknown_pass");
            assert_eq!(err.to_string(), sentence);
        }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cutplan`

Expected: compile errors — `no variant 'Stroke' found for enum 'Grouping'`, `no field 'key' on type 'ColorPass'`.

- [ ] **Step 3: Write the implementation**

Replace `crates/cutplan/src/passes.rs:14-21`:

```rust
/// All shapes cut together as one pass, and the key that says which pass it is. What the key
/// means is the `Grouping`'s business: a colour, a material preset, or `All` for the single
/// pass a `Grouping::Single` plan holds.
///
/// Named for the Document rather than for a colour because a colour is now one of three
/// things a pass can be keyed on — the type was `ColorPass` while it was the only one.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DocumentPass { pub key: PassKey, pub shapes: Vec<PlannedShape> }
```

Update `DocumentPasses::passes` to `Vec<DocumentPass>` (`:31-32`) and `travel_moves`'s parameter to `&[&DocumentPass]` (`:175`).

Replace `pass_key`, `Grouping` and `plan_passes` (`:67-97`):

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
        // Not reachable: the caller asks for a colour only under a colour mode.
        Grouping::Single | Grouping::Preset => None,
    }
}

/// How `plan_passes` splits cut shapes into passes.
///
/// `Color` is today's rule — stroke where visible, else fill — and stays the default, so a
/// caller that names no mode plans exactly what it planned before #148. `Single` is one pass
/// in document order, which is what `cuthulhu cut` without `--group-by` has always meant.
///
/// There is no line-type mode: `CutLineType` is `{Cut, NoCut}` and a `NoCut` shape never
/// reaches a pass, so such a mode would be `Single` under another name while carrying
/// different skip/order semantics. #56 adds it with `CutEdge`, the member that makes it split
/// anything.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Grouping { Single, Color, Stroke, Fill, Preset }

/// Walk the document in preorder from `doc.root`, group the shapes whose `CutLineType` is
/// `Cut` by the key `grouping` asks for, and flatten each shape's outline under its
/// accumulated world transform. A `NoCut` shape is counted, not cut. Iterative (explicit
/// stack) so depth is not bounded by the Rust call stack; a `visited` set catches cycles in
/// malformed docs.
pub fn plan_passes(doc: &Document) -> Result<DocumentPasses, PlanError> {
    plan_passes_with(doc, Grouping::Color)
}
```

In `plan_passes_with`, the stack carries the resolved material beside the world transform:

```rust
    // The nearest assigned ancestor's material rides down the walk beside the world
    // transform. Storing a resolved value on each shape instead would go stale the moment a
    // node is reparented — silently, and only visible as the wrong settings on real material.
    let mut stack: Vec<(NodeId, Affine, Option<&str>)> = vec![(doc.root, Affine::identity(), None)];
```

with `while let Some((id, parent_world, inherited)) = stack.pop()`, and immediately after `let world = …`:

```rust
        // Resolved for this node and everything under it. `Unassigned` is what stops the
        // chain — the state an `Option<String>` could not express.
        let material: Option<&str> = match &node.material_preset {
            PresetAssignment::Inherit => inherited,
            PresetAssignment::Unassigned => None,
            PresetAssignment::Preset(id) => Some(id.as_str()),
        };
```

the container arm pushes `material`:

```rust
            NodeKind::Group | NodeKind::Layer => {
                // Push in reverse so preorder visits children left-to-right.
                for &child in node.children.iter().rev() {
                    stack.push((child, world, material));
                }
            }
```

and the `CutLineType::Cut` arm replaces the `let color = match grouping { … }` block and the pass lookup:

```rust
                        let key = match grouping {
                            // One bucket, and a key that says so: `Color(None)` is the pass
                            // of unpainted shapes, which is a different fact.
                            Grouping::Single => PassKey::All,
                            Grouping::Color | Grouping::Stroke | Grouping::Fill =>
                                PassKey::Color(color_key(&node.style, grouping)),
                            // Not checked against the preset file: a deleted user preset is a
                            // real state, and refusing a cut over a settings lookup is not
                            // `plan_cut`'s job.
                            Grouping::Preset => PassKey::Preset(material.map(String::from)),
                        };
                        match passes.iter_mut().find(|p| p.key == key) {
                            Some(pass) => pass.shapes.push(shape),
                            None => passes.push(DocumentPass { key, shapes: vec![shape] }),
                        }
```

Add `PassKey` and `PresetAssignment` to the module's imports.

**The mechanical migrations in this file and its neighbours — none optional, all named:**

- `crates/cutplan/src/passes.rs:317` — `assert_eq!(red.color, Some(RED))` becomes `assert_eq!(red.key, PassKey::Color(Some(RED)))`.
- `crates/cutplan/src/passes.rs:463` — the `ColorPass { … }` literal in the travel test becomes `DocumentPass { key: …, shapes: … }`.
- Every other `.color` on a pass, and every `Grouping::ByColor`, in that module's tests: `Grouping::ByColor` no longer exists — it is `Grouping::Color`.
- `crates/cutplan/src/preflight.rs:5` — `use crate::passes::DocumentPass;`, and `ConfiguredPass::pass` becomes `&'a DocumentPass`.
- `crates/cutplan/src/preflight.rs:227` — `fn make_pass(color: Option<u32>, shapes: Vec<PlannedShape>) -> ColorPass` becomes `fn make_pass(key: PassKey, shapes: Vec<PlannedShape>) -> DocumentPass`; its call sites pass `PassKey::Color(Some(…))`.
- `crates/cutplan/src/preflight.rs:238` — `make_configured_pass`'s `&'a ColorPass` becomes `&'a DocumentPass`.
- `crates/cutplan/src/plan.rs:188` — `fn select(colors: &[u32]) -> Vec<PassSelection>` becomes `fn select(keys: &[PassKey]) -> Vec<PassSelection>`; its callers at `:215`, `:223` and `:231` pass keys.

Then the selection types in `plan.rs`:

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
/// One pass, ready to encode. Keeps its key attached to the geometry it belongs to so callers
/// can label passes without index-matching a second list.
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

In `Display`, the two colour arms collapse into one, because the key knows how to name itself:

```rust
            // The key's own `Display`, not a re-spelling of it: a caller who typed
            // `--skip-pass preset:cameo5-htv` must read that string back verbatim.
            CutError::UnknownPass(key) => write!(f, "no planned pass is called {key}"),
```

In `code()`: `CutError::UnknownPass(_) => "unknown_pass"`. In `plan_cut`:

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

Expected: PASS. The workspace still fails to build in `cli` and `apps/desktop`, which Tasks 4 and 5 own — this is the one boundary that cannot be green, and it is why those two tasks follow immediately.

- [ ] **Step 5: Commit**

```bash
git add crates/cutplan/src
git commit -m "Split passes by what the caller asked for, and name them by key

Grouping gains strict stroke, strict fill and material preset beside the
stroke-else-fill rule, which stays the default so nothing existing re-plans
differently. A Single plan's pass is keyed All rather than sharing the
colourless key with unpainted shapes, which is what lets a refusal say which
pass was missing. A material resolves down the walk, so a reparented shape
inherits without an edit and Unassigned can stop the chain."
```

---

### Task 4: The CLI chooses a grouping and names passes by key

**Files:**
- Modify: `crates/cli/src/main.rs:44-54` (the three flags), `:113-131` (dispatch), `:155-188` (`cut_by_color`)
- Modify: `crates/cli/src/pipeline.rs:71-96` (`pass_order`), `:98-151` (both planning entry points), `:168-205` (`parse_hex_color`, the scope check, `check_interactive`)
- Modify: `crates/cli/src/cut.rs:44-50` (`format_pass_color`), `:88-106` (`pause_prompt`, `pass_color`)
- Modify: `crates/cli/tests/plain_cut.rs`, `crates/cli/tests/dry_run.rs` (the `plan_cut_from_svg` call sites)
- Test: the existing `mod tests` in `pipeline.rs` (from `:207`) and `cut.rs` (from `:241`)

**Interfaces:**
- Consumes: `PassKey`, `Grouping`, `PassSelection { key, .. }`, `PlannedPass { key, .. }`.
- Produces: `--group-by`, repeatable `--skip-pass` and `--order`; `pipeline::plan_cut_from_svg(svg, driver, settings, grouping, skip_passes, order, allow_out_of_bounds)`, `pipeline::pass_order(&[DocumentPass], &[String], &[String])`, `pipeline::check_pass_flag_scope`, `cut::format_pass_key`.

- [ ] **Step 1: Write the failing tests**

In `crates/cli/src/pipeline.rs`'s `mod tests`, replace `by_color_plans_from_svg_respects_skip_and_order` (`:226-233`), `parse_hex_color_requires_eight_digits` (`:297-301`), `noninteractive_multicolor_is_error` (`:291-299`) and `colour_flags_are_refused_without_by_color` (`:364-373`) with:

```rust
    fn planned_two_colours() -> cutplan::DocumentPasses {
        cutplan::plan_passes(&doc_from_svg(two_color_svg()).unwrap()).unwrap()
    }

    /// `--order` puts named passes first in the order given, then everything else in planned
    /// order; `--skip-pass` removes. Keys, not colours, so a preset-grouped cut can be
    /// sequenced exactly as a colour-grouped one always could.
    #[test]
    fn pass_order_sequences_and_skips_by_key() {
        let planned = planned_two_colours();
        let blue_first = pass_order(&planned.passes, &[], &["color:0000ffff".into()]).unwrap();
        assert_eq!(blue_first,
            vec![PassKey::Color(Some(0x0000FFFF)), PassKey::Color(Some(0xFF0000FF))]);

        let without_red = pass_order(&planned.passes, &["color:ff0000ff".into()], &[]).unwrap();
        assert_eq!(without_red, vec![PassKey::Color(Some(0x0000FFFF))]);

        // Order is applied before the skip filter, as it always was.
        let both = pass_order(&planned.passes, &["color:ff0000ff".into()],
            &["color:0000ffff".into(), "color:ff0000ff".into()]).unwrap();
        assert_eq!(both, vec![PassKey::Color(Some(0x0000FFFF))]);
    }

    /// `--order` is repeatable rather than comma-separated, because a preset id may contain a
    /// comma and a split list would make such a pass unnameable — an operator's own string
    /// deciding whether a flag can address a pass.
    #[test]
    fn order_is_repeatable_and_keeps_the_order_given() {
        let planned = planned_two_colours();
        let keys = pass_order(&planned.passes, &[],
            &["color:0000ffff".into(), "color:ff0000ff".into()]).unwrap();
        assert_eq!(keys, vec![PassKey::Color(Some(0x0000FFFF)), PassKey::Color(Some(0xFF0000FF))]);
    }

    /// Both flags refuse a key that names no planned pass. `--order` used to drop one
    /// silently and `--skip-pass`'s predecessor still did: with four spellings of a key a
    /// typo is likelier than it was, and a skipped pass that was never there means cutting a
    /// colour the operator believed they had excluded.
    #[test]
    fn a_key_that_names_no_planned_pass_is_refused() {
        let planned = planned_two_colours();
        let err = pass_order(&planned.passes, &[], &["no-preset".into()]).unwrap_err();
        assert!(err.contains("no-preset"), "{err}");

        let err = pass_order(&planned.passes, &["preset:cameo5-htv".into()], &[]).unwrap_err();
        assert!(err.contains("preset:cameo5-htv"), "{err}");
    }

    /// A malformed key is `PassKey`'s own error, surfaced unchanged: one grammar means one
    /// message, and the CLI is where a person types it.
    #[test]
    fn a_malformed_pass_key_is_refused_with_the_grammar() {
        let planned = planned_two_colours();
        let err = pass_order(&planned.passes, &["ff0000ff".into()], &[]).unwrap_err();
        assert!(err.contains("is not a pass key"), "{err}");
    }

    /// A single-pass cut has one pass whose name nobody needs, so these flags cannot do
    /// anything and are refused rather than ignored.
    #[test]
    fn pass_flags_are_refused_for_a_single_pass_cut() {
        assert_eq!(
            check_pass_flag_scope(&["color:ff0000ff".into()], &[], Grouping::Single),
            Err("--skip-pass applies to a grouped cut; --group-by single is one pass over every shape".into())
        );
        assert_eq!(
            check_pass_flag_scope(&[], &["color:ff0000ff".into()], Grouping::Single),
            Err("--order applies to a grouped cut; --group-by single is one pass over every shape".into())
        );
        for g in [Grouping::Color, Grouping::Stroke, Grouping::Fill, Grouping::Preset] {
            assert!(check_pass_flag_scope(&["color:ff0000ff".into()], &["color:ff0000ff".into()], g).is_ok());
        }
    }

    /// The plain path still means one pass over everything, and now says so with a mode.
    #[test]
    fn the_default_grouping_plans_one_pass_named_all() {
        let two_fills = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <rect width="5" height="5" fill="#ff0000"/><rect x="6" width="5" height="5" fill="#00ff00"/></svg>"##;
        let plan = plan_cut_from_svg(two_fills, cameo5().as_ref(), &cut_settings(),
            Grouping::Single, &[], &[], false).unwrap();
        assert_eq!(plan.passes.len(), 1);
        assert_eq!(plan.passes[0].key, PassKey::All);

        let by_colour = plan_cut_from_svg(two_fills, cameo5().as_ref(), &cut_settings(),
            Grouping::Color, &[], &[], false).unwrap();
        assert_eq!(by_colour.passes.len(), 2, "the same file, split by its fills");
    }

    /// Two different empty cuts, two different sentences. "no cuttable paths in SVG" used to
    /// cover both, which told an operator their file was empty when in fact their own
    /// `--skip-pass` had emptied the selection.
    #[test]
    fn an_empty_file_and_an_emptied_selection_read_differently() {
        let empty = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"></svg>"##;
        let err = plan_cut_from_svg(empty, cameo5().as_ref(), &cut_settings(),
            Grouping::Color, &[], &[], false).unwrap_err();
        assert_eq!(err, "no cuttable paths in SVG");

        let err = plan_cut_from_svg(two_color_svg(), cameo5().as_ref(), &cut_settings(),
            Grouping::Color, &["color:ff0000ff".into(), "color:0000ffff".into()], &[], false)
            .unwrap_err();
        assert_eq!(err, "every pass in this file was skipped; nothing is left to cut");
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

In `crates/cli/src/cut.rs`'s `mod tests`, replace the colour assertions in `a_prompt_takes_both_halves_of_the_position_from_the_status` (`:281-298`) and `a_colourless_pass_is_named_none_in_the_prompt` (`:304-314`). The module's `plan(&[…])` helper now takes keys:

```rust
    /// The prompt names the pass the way every other surface does. `#0000ff` was a second
    /// spelling of one key, invented here and nowhere else.
    #[test]
    fn a_prompt_takes_both_halves_of_the_position_from_the_status() {
        let plan = plan(&[PassKey::Color(Some(0xff0000ff)), PassKey::Color(Some(0x0000ffff)), PassKey::All]);
        let at_second = status(
            Actions { cancel: true, resume: true, ..Actions::default() },
            Phase::AwaitingColorSwap,
            Some(PassPosition { index: 1, total: 3 }),
        );

        let swap = pause_prompt(Pause::Swap, &plan, &at_second);
        assert!(swap.contains("Pass 2/3"), "counts from 1, out of the job's own total: {swap}");
        assert!(swap.contains("color:0000ffff"), "names the pass being swapped to: {swap}");
        assert!(swap.contains("swap tool"), "says what to do: {swap}");

        let confirm = pause_prompt(Pause::Confirm, &plan, &at_second);
        assert!(confirm.contains("Pass 2/3"), "{confirm}");
        assert!(confirm.contains("once the machine finishes"), "{confirm}");
    }

    /// A single-pass cut's pass is named for what it is rather than for a colour it does not
    /// have. The prompt used to read `#000000` — the invented stroke the plain path stamped
    /// on every path before #144 — and then `none`, which said nothing.
    #[test]
    fn the_single_pass_is_named_all_in_the_prompt() {
        let plan = plan(&[PassKey::All]);
        let parked = status(
            Actions { cancel: true, confirm: true, ..Actions::default() },
            Phase::AwaitingConfirmation,
            Some(PassPosition { index: 0, total: 1 }),
        );
        assert!(pause_prompt(Pause::Confirm, &plan, &parked).contains("(all)"));
    }

    /// A pass index the plan does not have degrades to a label rather than panicking a live
    /// cut — the prompt is cosmetic, the cut is not.
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

Expected: compile errors — `cannot find function 'check_pass_flag_scope'`, `plan_cut_from_svg` taking the wrong number of arguments.

- [ ] **Step 3: Write the implementation**

`crates/cli/src/main.rs` — a clap-side enum beside `Command`, so kebab-case flag values are clap's business and the planner's enum stays free of presentation:

```rust
/// The `--group-by` spellings. Separate from `cutplan::Grouping` so the flag's vocabulary and
/// the planner's are free to differ; the `From` below is the only place they meet.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum GroupBy { Single, Color, Stroke, Fill, Preset }

impl From<GroupBy> for cutplan::Grouping {
    fn from(g: GroupBy) -> cutplan::Grouping {
        match g {
            GroupBy::Single => cutplan::Grouping::Single,
            GroupBy::Color => cutplan::Grouping::Color,
            GroupBy::Stroke => cutplan::Grouping::Stroke,
            GroupBy::Fill => cutplan::Grouping::Fill,
            GroupBy::Preset => cutplan::Grouping::Preset,
        }
    }
}
```

Replace the three flags (`:44-54`):

```rust
        /// How to split the cut into passes: single (one pass over every cut shape), color
        /// (stroke where visible, else fill), stroke, fill, or preset
        #[arg(long, value_enum, default_value_t = GroupBy::Single)]
        group_by: GroupBy,
        /// Do not cut the pass with this key (all, color:RRGGBBAA, no-color, preset:<id>,
        /// no-preset); may be repeated
        #[arg(long = "skip-pass")]
        skip_pass: Vec<String>,
        /// Cut this pass first, by key; repeat to sequence more, and the rest follow in
        /// planned order
        #[arg(long)]
        order: Vec<String>,
```

Rewrite the dispatch arm (`:115-132`) so one call serves both paths — the old `if !by_color` split existed only because the plain path did its own planning:

```rust
        Command::Cut { file, device, dry_run, speed, force, port, baud, group_by, skip_pass, order, allow_out_of_bounds } => {
            let driver = driver_for(&device)?;
            let grouping: cutplan::Grouping = group_by.into();
            check_pass_flag_scope(&skip_pass, &order, grouping)?;
            let svg = std::fs::read(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
            let settings = Settings { speed, force, repeat_count: 1 };
            cut_planned(&svg, driver.as_ref(), &device, &settings, grouping, &skip_pass, &order,
                        dry_run, port, baud, allow_out_of_bounds)
        }
```

Rename `cut_by_color` to `cut_planned`, give it `grouping: cutplan::Grouping` and `order: &[String]`, forward them to `plan_cut_from_svg`, and print headers by mode:

```rust
    if dry_run {
        for (i, pass) in passes.iter().enumerate() {
            // A header names a pass among several. `single` has none to name, and a bare
            // `cuthulhu cut --dry-run` has always printed bytes and nothing else — scripts
            // parse that output, so the merge of the two paths must not add a line to it.
            if grouping != cutplan::Grouping::Single {
                println!("-- pass {}/{} ({}) --", i + 1, passes.len(), pass.key);
            }
            let bytes = dry_run_pass_bytes(driver, &pass.job, i, passes.len())?;
            print_hex_ascii(&bytes);
        }
        return Ok(());
    }
```

`crates/cli/src/pipeline.rs` — `pass_order` over keys, validating both flags:

```rust
/// The passes to cut, in cut order: apply `--order` (named passes to the front, in the order
/// given; the rest keep their planned order) and then `--skip-pass`.
///
/// A key that names no planned pass is refused, for either flag. `--order` used to drop one
/// silently and `--skip-color` still did, which made a typo indistinguishable from a colour
/// the document did not contain — and a silently ignored skip means cutting a pass the
/// operator believed they had excluded.
pub fn pass_order(
    planned: &[cutplan::DocumentPass],
    skip_passes: &[String],
    order: &[String],
) -> Result<Vec<cutplan::PassKey>, String> {
    let mut keys: Vec<cutplan::PassKey> = planned.iter().map(|p| p.key.clone()).collect();
    let parse = |s: &String| s.trim().parse::<cutplan::PassKey>();

    let mut front = vec![];
    for key in order.iter().map(parse).collect::<Result<Vec<_>, _>>()? {
        let Some(i) = keys.iter().position(|k| *k == key) else {
            return Err(format!("--order names {key}, which is not a pass this file plans"));
        };
        front.push(keys.remove(i));
    }
    front.extend(keys);
    keys = front;

    for key in skip_passes.iter().map(parse).collect::<Result<Vec<_>, _>>()? {
        let Some(i) = keys.iter().position(|k| *k == key) else {
            return Err(format!("--skip-pass names {key}, which is not a pass this file plans"));
        };
        keys.remove(i);
    }
    Ok(keys)
}
```

One planning entry point; `plan_plain_cut` and `parse_hex_color` are deleted — the plain cut is `Grouping::Single`, and colour parsing lives in `PassKey::from_str`:

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
    order: &[String],
    allow_out_of_bounds: bool,
) -> Result<cutplan::CutPlan, String> {
    let doc = doc_from_svg(svg)?;
    // Planned once: the flags name passes, so the keys have to be known before a selection
    // can be built, and `plan_cut` cuts the very passes handed to it here.
    let planned = cutplan::plan_passes_with(&doc, grouping).map_err(|e| e.to_string())?;
    // Two different empty cuts, told apart here because only this caller knows an SVG was
    // imported and what the operator asked to skip. Left to `plan_cut`, both would arrive as
    // an unmatched selection or `NothingToCut`, and one sentence would have to cover both.
    if planned.passes.is_empty() {
        return Err("no cuttable paths in SVG".into());
    }
    let keys = pass_order(&planned.passes, skip_passes, order)?;
    if keys.is_empty() {
        return Err("every pass in this file was skipped; nothing is left to cut".into());
    }

    // ponytail: one `--speed`/`--force` pair applies to every pass; the CLI has no per-pass
    // settings and no presets. Per-pass settings need a flag that names a pass key.
    let passes = keys
        .into_iter()
        .map(|key| cutplan::PassSelection { key, settings: settings.clone() })
        .collect();

    // No revision to be stale against: the document was imported a few lines ago.
    let opts = cutplan::PlanOptions { passes, expect_revision: None, allow_out_of_bounds };
    cutplan::plan_cut(&planned, driver.profile(), &driver.caps(), &opts).map_err(describe_cut_error)
}
```

The scope check and the TTY check stop naming flags that no longer exist:

```rust
/// `--skip-pass` and `--order` name passes, which only a grouped cut has more than one of. A
/// single-pass cut puts every cut shape in one pass, so these flags cannot do anything there
/// and are refused rather than ignored.
pub fn check_pass_flag_scope(
    skip_passes: &[String],
    order: &[String],
    grouping: cutplan::Grouping,
) -> Result<(), String> {
    if grouping != cutplan::Grouping::Single {
        return Ok(());
    }
    if !skip_passes.is_empty() {
        return Err("--skip-pass applies to a grouped cut; --group-by single is one pass over every shape".into());
    }
    if !order.is_empty() {
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

`crates/cli/src/cut.rs` — delete `format_pass_color` and `pass_color`, and add:

```rust
/// The pass the job is paused on, named as every other surface names it.
///
/// A bad index cannot happen on the normal path (the reported position indexes the same
/// plan), but the prompt is cosmetic, so a mismatch degrades to a label rather than panicking
/// a process mid-cut.
pub fn format_pass_key(plan: &cutplan::CutPlan, status: &CutStatus) -> String {
    let index = status.pass.map(|p| p.index).unwrap_or(0);
    match plan.passes.get(index) {
        Some(pass) => pass.key.to_string(),
        None => "unknown pass".into(),
    }
}
```

with `pause_prompt` interpolating it:

```rust
fn pause_prompt(pause: Pause, plan: &cutplan::CutPlan, status: &CutStatus) -> String {
    let (pass, total) = pass_position(status);
    let key = format_pass_key(plan, status);
    match pause {
        Pause::Swap => format!("Pass {pass}/{total} ({key}): swap tool, press Enter to resume"),
        Pause::Confirm => {
            format!("Pass {pass}/{total} ({key}) cutting; press Enter once the machine finishes")
        }
    }
}
```

Update the two integration tests' `plan_cut_from_svg` calls: `crates/cli/tests/dry_run.rs:35` and any in `crates/cli/tests/plain_cut.rs` gain `cutplan::Grouping::Color` (or `Single`, matching what each test's comment says it exercises) plus `&[], &[]`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cli --locked`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cli
git commit -m "Let the CLI ask for a grouping, and name a pass the way everything else does

--group-by replaces --by-color and --skip-pass replaces --skip-color, both over
pass keys, so a preset-grouped cut can be sequenced the way a colour one always
could. --order is repeatable rather than comma-separated, because a preset id
may contain a comma and a split list would make that pass unnameable. Both
flags now refuse a key that names no pass, and an emptied selection stops
reporting itself as an empty file. A single-pass dry run still prints bytes and
no header, which is what scripts parse."
```

---

### Task 5: The desktop threads the grouping through all three planner calls

**Files:**
- Modify: `apps/desktop/src/device.rs:50-65`, `:840-874` (`prepare_cut`), `:1103-1142` (response DTOs and `plan_cut_response`), `:1147-1203` (`TravelPassDto`, `travel_for_order`)
- Modify: `apps/desktop/src/state.rs` (add `set_material_preset` after `set_cut_line_type`, `:66-73`), `apps/desktop/src/ipc.rs:53-57` and `:137-145`, `apps/desktop/src/main.rs:47-92`
- Test: `apps/desktop/src/device.rs`'s `mod tests` — the helpers at `:1295-1307` and `:1363-1364`, and the tests at `:1331-1340`, `:1366-1439`

**Interfaces:**
- Consumes: `PassKey`, `Grouping`, `DocumentPass`, `PassSelection { key, .. }`, `document::PresetAssignment`, `document::commands::set_material_preset`.
- Produces: `plan_cut(state, grouping)`, `travel_for_order(state, doc_revision, grouping, passes)`, `cut(state, dev, request)` with `CutRequest::grouping`, `set_material_preset(state, ids, value)`, and `device::plan_cut_response(doc, grouping)`. Task 6 calls all of them.

- [ ] **Step 1: Write the failing tests**

First the existing helpers, which every test in the module goes through (`:1299-1307`, `:1363-1364`):

```rust
    fn request_from(plan: DocumentPasses) -> CutRequest {
        CutRequest {
            device_instance_id: test_instance().instance_id,
            doc_revision: plan.doc_revision.to_string(),
            // The mode the passes were planned under. `plan_for` uses the default, so this
            // must too, or every request in this module would be refused as unknown keys.
            grouping: Grouping::Color,
            passes: plan.passes.iter().map(|p| ConfiguredPassDto {
                key: p.key.clone(), enabled: true, preset_id: None,
                speed: None, force: None, repeat_count: None,
            }).collect(),
        }
    }

    fn on(key: PassKey) -> TravelPassDto { TravelPassDto { key, enabled: true } }
    fn off(key: PassKey) -> TravelPassDto { TravelPassDto { key, enabled: false } }
    fn colour(c: u32) -> PassKey { PassKey::Color(Some(c)) }
```

Every `on(RED)`/`off(BLUE)` call in `:1366-1425` becomes `on(colour(RED))`/`off(colour(BLUE))`; `:1406`'s unknown colour becomes `TravelPassDto { key: colour(0xDEADBEEF), enabled: true }`; the two `plan_cut_response(&app.editor.doc)` calls at `:1372` and `:1430` gain `Grouping::Color`; `:1436`'s `p.color == Some(BLUE)` becomes `p.key == colour(BLUE)`; and the code assertions at `:1339` and `:1408` become `"unknown_pass"`.

Then the new cases:

```rust
    /// The grouping the dialog asked for is the grouping that gets cut. Without it the
    /// operator could preview a fill-grouped plan and cut a stroke-grouped one, because each
    /// command plans the document itself.
    #[test]
    fn a_cut_honours_the_grouping_it_was_sent() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        // A red stroke over a green fill: the two colour modes key this shape differently,
        // so the request's grouping is observable in what matches.
        app.add_rect_with_style(10.0, 10.0, Some(RED), Some(GREEN));
        let revision = plan_cut_response(&app.editor.doc, Grouping::Fill).unwrap().doc_revision;

        let request = CutRequest {
            device_instance_id: dev_instance_id(&dev),
            doc_revision: revision,
            grouping: Grouping::Fill,
            passes: vec![ConfiguredPassDto {
                key: colour(RED), enabled: true, preset_id: None,
                speed: None, force: None, repeat_count: None }],
        };
        // Fill grouping keys that shape on its fill, so the stroke's key names nothing.
        assert_eq!(dev.cut_from_request(&app, request).unwrap_err().code, "unknown_pass");
    }

    /// Travel is replanned with the same grouping, for the same reason.
    #[test]
    fn travel_honours_the_grouping_it_was_sent() {
        let mut app = AppState::new();
        app.add_rect_with_style(10.0, 10.0, Some(RED), Some(GREEN));
        let revision = plan_cut_response(&app.editor.doc, Grouping::Fill).unwrap().doc_revision;

        assert!(travel_for_order(&app.editor.doc, &revision, Grouping::Fill,
            &[on(colour(GREEN))]).is_ok());
        assert_eq!(travel_for_order(&app.editor.doc, &revision, Grouping::Fill,
            &[on(colour(RED))]).unwrap_err().code, "unknown_pass");
    }

    /// The response names its passes in the spelling a request must send back.
    #[test]
    fn a_plan_response_names_its_passes_by_key() {
        let mut app = AppState::new();
        app.add_rect(10.0, 10.0);
        let response = plan_cut_response(&app.editor.doc, Grouping::Single).unwrap();
        assert_eq!(response.passes[0].key, PassKey::All);
    }

    /// A preset-keyed pass is cut with that preset's settings. This is the whole point of
    /// grouping by material: `prepare_cut` reads only `preset_id`, so a row that arrives with
    /// none is cut with defaults no matter what its key says.
    #[test]
    fn a_preset_keyed_pass_cuts_with_that_presets_settings() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        let id = app.add_rect(10.0, 10.0);
        app.set_material_preset(vec![id], PresetAssignment::Preset("cameo5-htv".into())).unwrap();
        let revision = plan_cut_response(&app.editor.doc, Grouping::Preset).unwrap().doc_revision;

        let request = CutRequest {
            device_instance_id: dev_instance_id(&dev),
            doc_revision: revision,
            grouping: Grouping::Preset,
            passes: vec![ConfiguredPassDto {
                key: PassKey::Preset(Some("cameo5-htv".into())),
                enabled: true,
                // What the dialog now sends for a preset-keyed row: the key's own id.
                preset_id: Some("cameo5-htv".into()),
                speed: None, force: None, repeat_count: None }],
        };
        let (_, passes) = dev.prepare_cut(&app, request).unwrap();
        let builtin = cutplan::presets::builtin_presets().into_iter()
            .find(|p| p.id == "cameo5-htv").expect("premise: the builtin exists");
        assert_eq!(passes[0].job.settings.speed, builtin.settings.speed);
        assert_eq!(passes[0].job.settings.force, builtin.settings.force);
    }
```

Add the two helpers these need beside the existing ones: `dev_instance_id(&DeviceManagerHandle) -> String` (or reuse `test_instance().instance_id` as `request_from` does), and `AppState::add_rect_with_style(w, h, stroke, fill) -> NodeId` modelled on the existing `add_rect`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cuthulhu-desktop` (the desktop crate's name is in `apps/desktop/Cargo.toml`)

Expected: compile errors — `no field 'grouping' on type 'CutRequest'`, `plan_cut_response` taking one argument.

- [ ] **Step 3: Write the implementation**

```rust
#[derive(Deserialize)]
pub struct CutRequest {
    pub device_instance_id: String,
    pub doc_revision: String,
    /// How the dialog grouped the passes it is naming. Sent rather than remembered: the plan,
    /// the travel and the cut are three round trips, and a mode kept in `AppState` could be
    /// changed between them while the stale-plan check only guards the document.
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

In `prepare_cut` (`:850-873`): `PassSelection { key: dto.key.clone(), settings: resolve_settings(preset, &override_) }`, and `plan_passes_with(&app.editor.doc, request.grouping)`.

```rust
#[derive(Debug, Serialize)]
pub struct PlanCutPassSummary {
    /// The pass's key, as the canonical string the dialog keys its rows on and sends back. A
    /// string rather than a tagged object so the CLI, this DTO and the dialog hold one
    /// spelling.
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
    let planned = plan_passes_with(doc, grouping)
        .map_err(|e| IpcError::new("plan_error", e.to_string()))?;
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

`TravelPassDto::color` becomes `pub key: PassKey`; `travel_for_order` gains `grouping: Grouping` after `doc_revision`, plans with `plan_passes_with(doc, grouping)`, holds `Vec<&DocumentPass>`, and keeps its duplicate-versus-unknown distinction on keys:

```rust
        let Some(i) = remaining.iter().position(|p| p.key == pass.key) else {
            return Err(if planned.passes.iter().any(|p| p.key == pass.key) {
                IpcError::new("plan_mismatch", "the requested pass list does not name every planned pass exactly once")
            } else {
                map_cut_error(CutError::UnknownPass(pass.key.clone()))
            });
        };
```

`state.rs`, mirroring `set_cut_line_type` (`:66-73`) — the whole method:

```rust
    pub fn set_material_preset(&mut self, ids: Vec<NodeId>, value: PresetAssignment)
        -> Result<Delta, CmdError> {
        let d = commands::set_material_preset(&self.editor.doc, &ids, value)?;
        // Same rule as `set_cut_line_type`: an empty delta is a no-op the operator asked for,
        // and committing it would clear the redo stack and add an undo step that does nothing.
        if d.0.is_empty() { return Ok(d); }
        Ok(self.editor.commit(d))
    }
```

`ipc.rs` — one thin command, and the two planner commands gain the parameter:

```rust
#[tauri::command]
pub fn set_material_preset(state: tauri::State<AppStateHandle>, ids: Vec<NodeId>, value: PresetAssignment)
    -> Result<Delta, String> {
    state.lock().unwrap().set_material_preset(ids, value).map_err(|e| format!("{e:?}"))
}

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

Register `ipc::set_material_preset` in `main.rs`'s `generate_handler!` list, immediately after `ipc::set_cut_line_type` (`:56`).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. This is the first task since Task 3 where the whole workspace builds.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src
git commit -m "Send the grouping with every cut request, and cut a preset pass with its preset

plan_cut, travel_for_order and cut each plan the document themselves, so a mode
kept in AppState could change between them with the stale-plan check none the
wiser: the preview would show one arrangement and the machine cut another. The
preset test pins the thing grouping by material exists for — prepare_cut
resolves settings from preset_id alone, so a preset-keyed pass whose row
carried none was cut with defaults."
```

---

### Task 6: The TypeScript wire speaks in keys

**Files:**
- Modify: `apps/desktop/ui/src/ipc.ts:148-162`, `:209-225`, and the `setCutLineType` neighbourhood at `:41-43`
- Modify: `apps/desktop/ui/src/cut/viewmodel.ts:8-38`, `:166-170`, `:232-249`, adding `parsePassKey`, `passRowLabel` and `presetIdForKey`
- Test: `apps/desktop/ui/src/cut/viewmodel.test.ts`

**Interfaces:**
- Consumes: the Rust DTOs from Task 5.
- Produces: `PassKey` (string alias), `Grouping`, `PresetAssignmentJson`, `ParsedPassKey`, `parsePassKey`, `passRowLabel`, `presetIdForKey`, `PassVm.key`, `planCut(grouping)`, `travelForOrder(docRevision, grouping, passes)`, `toCutRequest(dev, revision, grouping, rows)`, `setMaterialPreset(args)`. Tasks 7 and 8 consume them.

- [ ] **Step 1: Write the failing tests**

Add to `viewmodel.test.ts`, and update every existing `PassVm` literal — `reorderPass` (`:129-205`), `reorderForReplan` (`:209-231`), `toTravelPasses` (`:236-247`), `effectiveSettings` (`:250-338`), `toCutRequest` (`:383-430` **and the multi-pass literals at `:437-456`**) — to carry `key: "color:ff0000ff"`-style values instead of `color: 0xff0000ff`:

```ts
describe("parsePassKey", () => {
  // The same table as crates/cutplan/src/pass_key.rs's round-trip test. These two tables are
  // the only thing keeping the dialog and the planner agreed on what a pass is called.
  it.each([
    ["all", { kind: "all" }],
    ["color:ff0000ff", { kind: "color", color: 0xff0000ff }],
    ["no-color", { kind: "color", color: null }],
    ["preset:cameo5-htv", { kind: "preset", presetId: "cameo5-htv" }],
    ["no-preset", { kind: "preset", presetId: null }],
  ])("parses %s", (key, expected) => {
    expect(parsePassKey(key)).toEqual(expected);
  });

  it("keeps a colon inside a preset id", () => {
    expect(parsePassKey("preset:vinyl:thin")).toEqual({ kind: "preset", presetId: "vinyl:thin" });
  });

  // The collision the grammar exists to avoid: a preset actually called "none" is not the
  // absence of a preset.
  it("tells a preset called none from no preset at all", () => {
    expect(parsePassKey("preset:none")).toEqual({ kind: "preset", presetId: "none" });
    expect(parsePassKey("no-preset")).toEqual({ kind: "preset", presetId: null });
  });

  // A key the backend produced that this cannot read is a version mismatch, not operator
  // input: it renders as itself rather than throwing, because a dialog that crashes mid-cut
  // is worse than one showing a string nobody recognises.
  it("returns the raw key it cannot parse", () => {
    expect(parsePassKey("line-type:cut")).toEqual({ kind: "unknown", raw: "line-type:cut" });
    expect(parsePassKey("preset:")).toEqual({ kind: "unknown", raw: "preset:" });
  });
});

describe("passRowLabel", () => {
  const presets = [{ id: "cameo5-htv", name: "HTV", machine_id: "cameo5",
                     settings: { speed: 5, force: 20, repeat_count: 1 }, builtin: true }];

  it("names a colour pass by its swatch, not by words", () => {
    expect(passRowLabel("color:ff0000ff", presets, "Color")).toEqual({ swatch: "#ff0000", text: null });
  });

  // Grouping-aware, because `no-color` means something different in each colour mode: under
  // Stroke it can hold brightly filled shapes, so "no visible paint" would be false.
  it.each([
    ["Color", "No visible paint"],
    ["Stroke", "No visible stroke"],
    ["Fill", "No visible fill"],
  ])("says what the colourless pass holds under %s", (grouping, text) => {
    expect(passRowLabel("no-color", presets, grouping as Grouping)).toEqual({ swatch: null, text });
  });

  // Not "every shape": a NoCut shape is excluded and counted as skipped.
  it("names the single pass for what it holds", () => {
    expect(passRowLabel("all", presets, "Single")).toEqual({ swatch: null, text: "Every cut shape" });
  });

  it("resolves a preset to its name", () => {
    expect(passRowLabel("preset:cameo5-htv", presets, "Preset")).toEqual({ swatch: null, text: "HTV" });
  });

  // A preset a document names but the file no longer has: the planner keys the pass anyway,
  // so the dialog has to render one.
  it("shows an unresolved preset id as unknown", () => {
    expect(passRowLabel("preset:deleted", presets, "Preset"))
      .toEqual({ swatch: null, text: "deleted (unknown preset)" });
  });

  it("names the pass that resolves to no material", () => {
    expect(passRowLabel("no-preset", presets, "Preset")).toEqual({ swatch: null, text: "No preset" });
  });
});

describe("presetIdForKey", () => {
  // What makes grouping by material do the thing it exists for: the pass's own preset
  // supplies its settings, instead of the operator re-picking it once per pass.
  it("takes the preset a preset-keyed pass names", () => {
    expect(presetIdForKey("preset:cameo5-htv")).toBe("cameo5-htv");
  });

  // Kept even when it resolves to nothing: prepare_cut falls back to the override-or-default
  // path, and clearing it here would silently drop what the document said.
  it("keeps an id that may not resolve", () => {
    expect(presetIdForKey("preset:deleted")).toBe("deleted");
  });

  it.each(["all", "no-color", "no-preset", "color:ff0000ff"])("has nothing to take from %s", (key) => {
    expect(presetIdForKey(key)).toBeNull();
  });
});

describe("toCutRequest", () => {
  it("sends the grouping alongside the keyed passes", () => {
    const rows: PassVm[] = [
      { key: "preset:cameo5-htv", shapeCount: 2, enabled: true, presetId: "cameo5-htv",
        speed: null, force: null, repeatCount: null },
    ];
    expect(toCutRequest("dev-1", "42", "Preset", rows)).toEqual({
      device_instance_id: "dev-1",
      doc_revision: "42",
      grouping: "Preset",
      passes: [{ key: "preset:cameo5-htv", enabled: true, preset_id: "cameo5-htv",
                 speed: null, force: null, repeat_count: null }],
    });
  });
});

describe("toTravelPasses", () => {
  it("names every row by key, disabled ones included", () => {
    expect(toTravelPasses([
      { key: "no-color", enabled: false },
      { key: "all", enabled: true },
    ])).toEqual([
      { key: "no-color", enabled: false },
      { key: "all", enabled: true },
    ]);
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm --prefix apps/desktop/ui test -- viewmodel`

Expected: FAIL — `parsePassKey is not a function`, and type errors on `key`.

- [ ] **Step 3: Write the implementation**

`apps/desktop/ui/src/ipc.ts`:

```ts
/** A pass's name, in the canonical form `cutplan::PassKey` writes: `all`, `color:ff0000ff`,
 *  `no-color`, `preset:<id>`, `no-preset`. Sent back verbatim in a travel or cut request —
 *  the string *is* the identity. Absence has its own token because a preset id is an
 *  unrestricted operator string, so `preset:none` would collide with a preset called `none`. */
export type PassKey = string;

/** How the planner splits shapes into passes. Mirrors `cutplan::Grouping`. */
export type Grouping = "Single" | "Color" | "Stroke" | "Fill" | "Preset";

/** Mirrors `document::PresetAssignment`'s adjacently-tagged JSON. */
export type PresetAssignmentJson =
  | { state: "inherit" }
  | { state: "unassigned" }
  | { state: "preset"; id: string };

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

`apps/desktop/ui/src/cut/viewmodel.ts` — the types, then the three pure helpers:

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

/** What a `PassKey` says, for the two things the UI needs from inside one: a swatch needs the
 *  RGBA, a row label and a row's settings need the preset id. The mirror of
 *  `cutplan::PassKey::from_str`; the example table in `viewmodel.test.ts` keeps the two
 *  agreed. */
export type ParsedPassKey =
  | { kind: "all" }
  | { kind: "color"; color: number | null }
  | { kind: "preset"; presetId: string | null }
  | { kind: "unknown"; raw: string };

export function parsePassKey(key: PassKey): ParsedPassKey {
  if (key === "all") return { kind: "all" };
  if (key === "no-color") return { kind: "color", color: null };
  if (key === "no-preset") return { kind: "preset", presetId: null };
  // First separator only, so a preset id may contain one — same rule as the Rust parser.
  const at = key.indexOf(":");
  if (at === -1) return { kind: "unknown", raw: key };
  const mode = key.slice(0, at);
  const value = key.slice(at + 1);
  if (mode === "color") {
    // Eight digits exactly: anything shorter would parse to a colour no shape carries.
    if (/^[0-9a-fA-F]{8}$/.test(value)) return { kind: "color", color: parseInt(value, 16) };
    return { kind: "unknown", raw: key };
  }
  // An empty id is refused for the same reason the Rust grammar refuses it.
  if (mode === "preset" && value !== "") return { kind: "preset", presetId: value };
  return { kind: "unknown", raw: key };
}

/** How a pass row identifies itself: a swatch when the key is a colour, words otherwise.
 *  Grouping-aware because `no-color` means something different per mode — under Stroke it can
 *  hold brightly filled shapes, so calling it "no visible paint" would be false. */
export function passRowLabel(
  key: PassKey,
  presets: Preset[],
  grouping: Grouping,
): { swatch: string | null; text: string | null } {
  const parsed = parsePassKey(key);
  switch (parsed.kind) {
    case "color":
      if (parsed.color !== null) {
        // Drop the alpha byte: a swatch is a colour, and 0-alpha keys never reach here.
        return { swatch: `#${(parsed.color >>> 8).toString(16).padStart(6, "0")}`, text: null };
      }
      return {
        swatch: null,
        text: grouping === "Stroke" ? "No visible stroke"
            : grouping === "Fill" ? "No visible fill"
            : "No visible paint",
      };
    // Not "every shape": a NoCut shape is excluded from it and counted as skipped.
    case "all":
      return { swatch: null, text: "Every cut shape" };
    case "preset": {
      if (parsed.presetId === null) return { swatch: null, text: "No preset" };
      const preset = presets.find((p) => p.id === parsed.presetId);
      // An id the preset file no longer resolves is a real state: presets are machine-scoped
      // and a user entry can be deleted while a document still names it.
      return { swatch: null, text: preset ? preset.name : `${parsed.presetId} (unknown preset)` };
    }
    case "unknown":
      return { swatch: null, text: parsed.raw };
  }
}

/** The preset a pass is keyed on, which is the preset it must be cut with. `prepare_cut`
 *  resolves settings from `preset_id` alone, so a preset-keyed row that arrives without one is
 *  cut with defaults — the operator groups by material and gets none of that material's
 *  settings. Kept even when it resolves to nothing, so the request still says what the
 *  document said. */
export function presetIdForKey(key: PassKey): string | null {
  const parsed = parsePassKey(key);
  return parsed.kind === "preset" ? parsed.presetId : null;
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

Import `PassKey`, `Grouping` from `../ipc` in `viewmodel.ts`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npm --prefix apps/desktop/ui test -- viewmodel`

Expected: PASS. `CutDialog`-level type errors are Task 7's.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src/ipc.ts apps/desktop/ui/src/cut/viewmodel.ts apps/desktop/ui/src/cut/viewmodel.test.ts
git commit -m "Carry a pass key across IPC as the one string both sides write

The wire type is the canonical spelling rather than a mirrored union, so the
dialog keys rows on what the planner named and sends it back verbatim. Labels
are grouping-aware because the colourless pass holds something different in
each colour mode, and presetIdForKey exists so a preset-keyed pass is cut with
its own preset rather than with defaults."
```

---

### Task 7: The cut dialog offers the choice, atomically

**Files:**
- Modify: `apps/desktop/ui/src/cut/CutDialog.tsx:105-109` (state), `:167-202` (`replan`), `:314-331` (`startCut`), `:354-394` (row edits and travel), `:539-546` (the stale-plan Replan button), `:548-625` (rows and the skipped sentence)
- Modify: `apps/desktop/ui/src/cut/CutPreview.tsx:19-25` (`PreviewPass.color` → `key`), `:75-83`, `:118-130`

**Interfaces:**
- Consumes: `parsePassKey`, `passRowLabel`, `presetIdForKey`, `planCut(grouping)`, `travelForOrder(docRevision, grouping, passes)`, `toCutRequest(dev, revision, grouping, rows)`.
- Produces: the operator-visible grouping control. Nothing else consumes this task.

**The state change is the substance of this task, not the picker.** Today the mode's three companions are independent `useState` values (`:105-109`) read separately by `startCut` (`:315-316`) and `refreshTravel` (`:365-368`). Adding a mode beside them means that between selecting a new mode and its plan arriving, the *old* rows are sendable under the *new* mode — and where the two key sets overlap, the backend accepts them and cuts the wrong shapes. So the plan, its mode, its revision and its rows install together or not at all.

- [ ] **Step 1: Write the failing test**

The behaviour is a dialog-level interleaving, so it is pinned in the e2e suite (Task 9) where the fake can hold a reply, and by types here. Add the type-level guard to `viewmodel.test.ts`:

```ts
describe("installed plan", () => {
  // The rows and the mode that produced them travel together. A row list is only ever sent
  // with the grouping of the plan it came from, which is what this shape enforces: there is
  // no way to build a request from rows without naming their plan's grouping.
  it("builds a request only from a plan's own grouping and rows", () => {
    const plan = { grouping: "Fill" as Grouping, revision: "7", skippedNotCut: 0, rows: [
      { key: "color:00ff00ff", shapeCount: 1, enabled: true, presetId: null,
        speed: null, force: null, repeatCount: null },
    ] };
    const request = toCutRequest("dev-1", plan.revision, plan.grouping, plan.rows);
    expect(request.grouping).toBe("Fill");
    expect(request.passes.map((p) => p.key)).toEqual(["color:00ff00ff"]);
  });
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `npm --prefix apps/desktop/ui test -- viewmodel`

Expected: FAIL — `toCutRequest` takes three arguments until Task 6 is in place; if Task 6 is already committed this passes immediately and the real proof of this task is Task 9's e2e case plus `tsc`.

- [ ] **Step 3: Write the implementation**

Replace the four independent pieces of plan state (`:105-109` keeps `capsFor`; `rows`, `travel`, `skippedNotCut`, `planRevision` go):

```tsx
  /** A plan and everything derived from it, installed as one value. The mode belongs here
   *  rather than beside it: rows keyed under one grouping must never be sent under another,
   *  and the stale-plan check guards the document, not the mode. */
  type InstalledPlan = {
    grouping: ipc.Grouping;
    revision: string;
    rows: PassRow[];
    skippedNotCut: number;
  };

  const [plan, setPlan] = useState<InstalledPlan | null>(null);
  const [travel, setTravel] = useState<[number, number, number, number][]>([]);
  /** The mode the operator has chosen, which is `plan.grouping` except while a replan is in
   *  flight. Cut and the row controls are unavailable in that window. */
  const [grouping, setGrouping] = useState<ipc.Grouping>("Color");
  const [replanning, setReplanning] = useState(false);
```

`replan` takes the mode explicitly and installs everything at once:

```tsx
  const replan = (mode: ipc.Grouping = grouping) => {
    const seq = ++planSeq.current;
    // A fresh plan orphans every reorder request: once when it is asked for (a reply landing
    // during the fetch would redraw travel the incoming plan replaces) and again when it
    // installs (a move made while the fetch was out carries the old revision, and its late
    // stale_plan rejection would re-raise the banner this plan just cleared — Greptile drove
    // exactly that interleaving on PR #142).
    travelSeq.current++;
    setReplanning(true);
    // Travel from the previous mode describes an arrangement that no longer exists.
    setTravel([]);
    ipc
      .planCut(mode)
      .then((response) => {
        if (seq !== planSeq.current) return; // a newer Replan owns the dialog now
        travelSeq.current++;
        setPlan({
          grouping: mode,
          revision: response.doc_revision,
          skippedNotCut: response.skipped_not_cut,
          rows: response.passes.map((p) => ({
            key: p.key,
            shapeCount: p.shape_count,
            nodeIds: p.node_ids,
            starts: p.starts,
            enabled: true,
            // A preset-keyed pass starts with the preset it is keyed on, or it would be cut
            // with defaults — the one thing grouping by material exists to avoid.
            presetId: presetIdForKey(p.key),
            speed: null,
            force: null,
            repeatCount: null,
          })),
        });
        setTravel(response.travel);
        setStalePlan(false);
      })
      .catch((e) => {
        if (seq !== planSeq.current) return; // superseded: its failure is no longer news
        onError(ipc.ipcErrorMessage(e));
      })
      .finally(() => {
        if (seq === planSeq.current) setReplanning(false);
      });
  };
```

The control sits above the pass list:

```tsx
        <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
          Group passes by
          <select
            aria-label="Group passes by"
            value={grouping}
            disabled={replanning}
            onChange={(e) => {
              const next = e.target.value as ipc.Grouping;
              setGrouping(next);
              replan(next);
            }}
          >
            <option value="Color">Colour (stroke, else fill)</option>
            <option value="Stroke">Stroke colour</option>
            <option value="Fill">Fill colour</option>
            <option value="Preset">Material preset</option>
            <option value="Single">One pass over everything</option>
          </select>
        </label>
```

Every remaining read follows mechanically, and each one is why the state was merged:

- `startCut` (`:314-316`): `if (!connected || plan === null || replanning) return;` then
  `toCutRequest(connected.instance_id, plan.revision, plan.grouping, plan.rows)`.
- The Cut button gains `disabled={… || plan === null || replanning}`, and the row controls
  (`Enabled`, preset, speed, force, repeat, Up, Down) gain `disabled={replanning}`.
- `refreshTravel(next)` (`:363-368`): returns early when `plan === null`, and calls
  `ipc.travelForOrder(plan.revision, plan.grouping, toTravelPasses(next))`.
- `updateRow`, `movePass`, `setPassEnabled` (`:354-394`) operate on `plan.rows` and write back
  with `setPlan((p) => (p === null ? p : { ...p, rows: next }))`.
- The stale-plan button (`:542`): `onClick={() => replan()}` — bare `onClick={replan}` would pass
  the mouse event as the mode.
- Rows (`:549-620`): `rows` becomes `plan?.rows ?? []`, `key={row.key}`, and the swatch/label come
  from `passRowLabel(row.key, presets, plan.grouping)`:

```tsx
                {label.swatch !== null ? (
                  <span style={{ width: 12, height: 12, display: "inline-block", background: label.swatch }} />
                ) : null}
                {label.text !== null ? <span>{label.text}</span> : null}
                <span>{row.shapeCount} shape(s)</span>
```

- The skipped sentence (`:623-625`): `plan?.skippedNotCut ?? 0`.
- `<CutPreview … passes={plan?.rows ?? []} … />`.

In `CutPreview.tsx`, `PreviewPass.color: number | null` becomes `key: PassKey`, and both draw sites derive the colour from the parsed key, keeping today's fallback for every non-colour pass:

```tsx
      const parsed = parsePassKey(pass.key);
      const color = parsed.kind === "color" && parsed.color !== null ? cssColor(parsed.color) : textColor;
```

- [ ] **Step 4: Verify**

Run: `npm --prefix apps/desktop/ui test && npm --prefix apps/desktop/ui run build`

Expected: PASS and a clean build. `tsc` is the real gate here: it names every `rows`/`planRevision` read the merge missed.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src/cut apps/desktop/ui/dist
git commit -m "Install a plan, its mode, its revision and its rows as one value

A grouping in the payload is necessary and not sufficient: while a replan was in
flight the previous mode's rows were still sendable under the new one, and where
two key sets overlap the backend accepts them and cuts the wrong shapes. Cut and
the row controls are unavailable until the new plan lands. A row keyed on a
preset now starts with that preset, and a row that has no swatch says what it
holds instead."
```

---

### Task 8: The operator can assign a material

**Files:**
- Create: `apps/desktop/ui/src/panels/materialPreset.ts`, `apps/desktop/ui/src/panels/materialPreset.test.ts`
- Modify: `apps/desktop/ui/src/panels/PropertiesPanel.tsx:1-51`, `apps/desktop/ui/src/App.tsx` (the `DocNode` type at `:19-45`, the preset list, and the panel's props)
- Modify: `apps/desktop/ui/src/cut/CutDialog.tsx:106` (stop owning the preset list)

**Interfaces:**
- Consumes: `ipc.setMaterialPreset`, `ipc.PresetAssignmentJson`, `Preset` from `viewmodel.ts`.
- Produces: `selectionAssignment(nodes, selected)`, `effectiveMaterials(nodes, root)`, and the panel control.

**The preset list moves up.** `presets` is local to the cut dialog (`CutDialog.tsx:106`), which is mounted only while the dialog is open (`App.tsx:480`), so the panel cannot read it there. `App` loads it and passes it to both. The machine it loads for is the document's, falling back to the connected device's — the same choice the dialog already makes for caps.

- [ ] **Step 1: Write the failing tests**

Create `apps/desktop/ui/src/panels/materialPreset.test.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { effectiveMaterials, selectionAssignment } from "./materialPreset";

const shape = (id: number, material_preset: unknown) => ({
  id, kind: { Shape: { Rect: { w: 1, h: 1 } } }, transform: [1, 0, 0, 1, 0, 0],
  style: { stroke: 255, fill: null }, cut_line_type: "Cut" as const,
  material_preset, children: [] as number[],
});
const layer = (id: number, material_preset: unknown, children: number[]) => ({
  id, kind: "Layer" as const, transform: [1, 0, 0, 1, 0, 0],
  style: { stroke: null, fill: null }, cut_line_type: "Cut" as const,
  material_preset, children,
});
const INHERIT = { state: "inherit" } as const;
const UNASSIGNED = { state: "unassigned" } as const;
const htv = { state: "preset", id: "cameo5-htv" } as const;

describe("selectionAssignment", () => {
  it("reports the one local assignment a selection agrees on", () => {
    const nodes = { "1": shape(1, htv), "2": shape(2, htv) };
    expect(selectionAssignment(nodes as never, [1, 2])).toEqual(htv);
  });

  // Mixed on the *local* assignment, even when both resolve to the same id: the control
  // edits local values, and saying otherwise would misreport what a click overwrites.
  it("reports mixed when a Layer's own value differs from its child's", () => {
    const nodes = { "1": layer(1, htv, [2]), "2": shape(2, INHERIT) };
    expect(selectionAssignment(nodes as never, [1, 2])).toBe("mixed");
  });

  // No descent: unlike the cuttability helper, a selected Layer speaks for itself, because
  // that is what the command writes.
  it("reads a selected Layer's own value, not its children's", () => {
    const nodes = { "1": layer(1, htv, [2]), "2": shape(2, UNASSIGNED) };
    expect(selectionAssignment(nodes as never, [1])).toEqual(htv);
  });

  it("returns undefined when nothing is selected", () => {
    expect(selectionAssignment({} as never, [])).toBeUndefined();
  });
});

describe("effectiveMaterials", () => {
  // Mirrors the planner's walk (crates/cutplan/src/passes.rs): nearest assigned ancestor
  // wins, and Unassigned stops the chain rather than deferring up it.
  it("resolves each node the way the planner will", () => {
    const nodes = {
      "1": layer(1, htv, [2, 3, 4]),
      "2": shape(2, INHERIT),
      "3": shape(3, UNASSIGNED),
      "4": shape(4, { state: "preset", id: "cameo5-vinyl-adhesive" }),
    };
    expect(effectiveMaterials(nodes as never, 1)).toEqual({
      1: "cameo5-htv", 2: "cameo5-htv", 3: null, 4: "cameo5-vinyl-adhesive",
    });
  });

  it("resolves to nothing when no ancestor assigns one", () => {
    const nodes = { "1": layer(1, INHERIT, [2]), "2": shape(2, INHERIT) };
    expect(effectiveMaterials(nodes as never, 1)).toEqual({ 1: null, 2: null });
  });
});
```

- [ ] **Step 2: Run them to verify they fail**

Run: `npm --prefix apps/desktop/ui test -- materialPreset`

Expected: FAIL — module not found.

- [ ] **Step 3: Write the implementation**

Create `apps/desktop/ui/src/panels/materialPreset.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { DocNode } from "../App";
import type { PresetAssignmentJson } from "../ipc";

/// The local assignment a selection agrees on, `"mixed"` when it does not, or `undefined`
/// when nothing is selected.
///
/// Deliberately no descent, unlike `selectionCutLineType`: `commands::set_material_preset`
/// writes the selected Nodes themselves, because a material inherits and a Layer's own value
/// is what reaches the shapes under it. Reporting a child's value for a selected Layer would
/// describe something the control cannot write.
export function selectionAssignment(
  nodes: Record<string, DocNode>,
  selected: number[],
): PresetAssignmentJson | "mixed" | undefined {
  const seen = new Set<string>();
  let first: PresetAssignmentJson | undefined;
  for (const id of selected) {
    const node = nodes[String(id)];
    if (!node) continue;
    const assignment = node.material_preset;
    const fingerprint = JSON.stringify(assignment);
    if (seen.size === 0) first = assignment;
    seen.add(fingerprint);
    if (seen.size > 1) return "mixed";
  }
  return first;
}

/// What each Node's material resolves to, keyed by id — the planner's rule, mirrored so the
/// panel can say "Inherited — HTV" without guessing. Nearest assigned ancestor wins, and
/// `unassigned` stops the chain instead of deferring up it.
export function effectiveMaterials(
  nodes: Record<string, DocNode>,
  root: number,
): Record<number, string | null> {
  const resolved: Record<number, string | null> = {};
  const seen = new Set<number>();
  const stack: [number, string | null][] = [[root, null]];
  while (stack.length > 0) {
    const [id, inherited] = stack.pop()!;
    // A malformed document whose nodes contain each other would otherwise spin here.
    if (seen.has(id)) continue;
    seen.add(id);
    const node = nodes[String(id)];
    if (!node) continue;
    const assignment = node.material_preset;
    const material =
      assignment.state === "preset" ? assignment.id
      : assignment.state === "unassigned" ? null
      : inherited;
    resolved[id] = material;
    for (const child of node.children) stack.push([child, material]);
  }
  return resolved;
}
```

Add `material_preset: PresetAssignmentJson` to `DocNode` in `App.tsx:19-45`.

In `PropertiesPanel.tsx`, add the props and the control. The states are named, not prose:

```tsx
type Props = {
  bounds: Bounds | null;
  cutLineType: CutLineTypeJson | "mixed" | null;
  /** The selection's own assignment; `undefined` when there is no selection. */
  materialPreset: PresetAssignmentJson | "mixed" | undefined;
  /** What that selection resolves to, for the inherited case — `null` is "no material". */
  effectiveMaterial: string | null;
  presets: Preset[];
  onChangeX: (v: number) => void;
  onChangeY: (v: number) => void;
  onChangeW: (v: number) => void;
  onChangeH: (v: number) => void;
  onChangeCutLineType: (v: CutLineTypeJson) => void;
  onChangeMaterialPreset: (v: PresetAssignmentJson) => void;
};
```

```tsx
/** What the material row reads for each state. `Inherit` shows what it resolves to, because
 *  "inherit" alone does not tell an operator which material the blade will be set for. */
function materialLabel(
  assignment: PresetAssignmentJson | "mixed",
  effective: string | null,
  presets: Preset[],
): string {
  const name = (id: string) => presets.find((p) => p.id === id)?.name ?? `Unresolved (${id})`;
  if (assignment === "mixed") return "Mixed";
  switch (assignment.state) {
    case "preset": return name(assignment.id);
    case "unassigned": return "No preset";
    case "inherit": return effective === null ? "Inherited — No preset" : `Inherited — ${name(effective)}`;
  }
}
```

```tsx
      {materialPreset !== undefined ? (
        <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
          Material
          <select
            aria-label="Material preset"
            value={materialPreset === "mixed" ? "" :
                   materialPreset.state === "preset" ? `preset:${materialPreset.id}` : materialPreset.state}
            onChange={(e) => {
              const v = e.target.value;
              onChangeMaterialPreset(
                v === "inherit" ? { state: "inherit" }
                : v === "unassigned" ? { state: "unassigned" }
                : { state: "preset", id: v.slice("preset:".length) },
              );
            }}
          >
            {/* A mixed selection shows this inert option as selected rather than picking a
                side; every other option commits, which one undo reverses. */}
            {materialPreset === "mixed" ? <option value="" disabled>Mixed</option> : null}
            <option value="inherit">Inherit</option>
            <option value="unassigned">No preset</option>
            {presets.map((p) => (
              <option key={p.id} value={`preset:${p.id}`}>{p.name}</option>
            ))}
          </select>
          <span style={{ color: "var(--muted)" }}>
            {materialLabel(materialPreset, effectiveMaterial, presets)}
          </span>
        </label>
      ) : null}
```

Update the "No selection" condition to `bounds === null && cutLineType === null && materialPreset === undefined`.

In `App.tsx`: load the preset list where the machine is known (mirroring how the dialog does it via `ipc.listPresets`), pass it to both `PropertiesPanel` and `CutDialog`, compute `selectionAssignment(nodes, selection)` and `effectiveMaterials(nodes, doc.root)[selection[0]] ?? null` for the props, and wire `onChangeMaterialPreset` exactly as `onChangeCutLineType` is wired — `ipc.setMaterialPreset({ ids: selection, value })`, apply the returned `Delta`, refresh the snapshot. In `CutDialog.tsx`, delete the local `presets` state (`:106`) and take it as a prop.

- [ ] **Step 4: Verify**

Run: `npm --prefix apps/desktop/ui test && npm --prefix apps/desktop/ui run build`

Expected: PASS, clean build.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src apps/desktop/ui/dist
git commit -m "Let the operator put a material on a Node, and say where it came from

Three states need three options, and Inherit has to show what it resolves to —
'inherit' alone does not tell an operator which material the blade will be set
for. The panel reads the selection's own value with no descent, because that is
what the command writes. The preset list moves to App: it was local to the cut
dialog, which only exists while that dialog is open."
```

---

### Task 9: The e2e fake tells the truth again

**Files:**
- Modify: `apps/desktop/ui/e2e/smoke.spec.ts:23-66` (fixtures), `:84`, `:95`, `:146` (the three Node constructors), `:302-341` (`planFromDoc`), `:459-505` (the three handlers), `:468-472` (the stale comment, #143), and the pass/travel/cut assertions at `:707`, `:733`, `:774`, `:798`, `:823`, `:842`, `:864`

**Interfaces:**
- Consumes: the real DTO shapes from Tasks 5 and 6.
- Produces: nothing — this is the suite catching up to the backend it mirrors.

- [ ] **Step 1: Write the failing assertions**

The fake's `Node` type gains `material_preset`, and every constructor — `freshDoc` (`:34`), the seeded rects (`:48-64`), and the three at `:84`, `:95`, `:146` — carries `material_preset: { state: "inherit" }`. Then the pass assertions move to keys, and two new cases:

```ts
  test("changing the grouping replans and renames the passes", async ({ page }) => {
    await openCutDialog(page);
    await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

    await page.getByLabel("Group passes by").selectOption("Single");
    // One pass, named for what it holds rather than for a colour it does not have.
    await expect(page.getByTestId("cut-pass-row")).toHaveCount(1);
    await expect(page.getByText("Every cut shape")).toBeVisible();

    const last = await page.evaluate(() => (window as unknown as {
      __travelRequests: { key: string; enabled: boolean }[][] }).__travelRequests.at(-1));
    expect(last?.map((p) => p.key)).toEqual(["all"]);
  });

  test("a cut cannot be sent with rows from the previous grouping", async ({ page }) => {
    await openCutDialog(page);
    await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());

    await page.getByLabel("Group passes by").selectOption("Single");
    // The replan is parked, so the old rows are still on screen — and must not be sendable.
    await expect(page.getByRole("button", { name: "Cut" })).toBeDisabled();

    await page.evaluate(() => (window as unknown as { __releasePlans: () => void }).__releasePlans());
    await expect(page.getByRole("button", { name: "Cut" })).toBeEnabled();
    await expect(page.getByTestId("cut-pass-row")).toHaveCount(1);
  });
```

- [ ] **Step 2: Run the suite to verify it fails**

Run: `npm --prefix apps/desktop/ui run e2e`

Expected: FAIL — the fake still returns `color`, and its handlers ignore the grouping.

- [ ] **Step 3: Update the fake**

`planFromDoc` takes the grouping and keys the way the planner does, resolving materials down the tree:

```ts
  // Mirrors crates/cutplan/src/passes.rs's plan_passes_with: preorder walk, skip Shape leaf
  // nodes whose CutLineType is NoCut, and key the rest as the grouping asks — a colour
  // (stroke where visible, else fill; strict under Stroke and Fill, with 0-alpha counting as
  // absent), the resolved material, or `all` for one pass. Absence is its own token
  // (`no-color`, `no-preset`) because a preset id may be any string.
  function planFromDoc(grouping: Grouping = "Color") {
    const byKey = new Map<string, { key: string; node_ids: number[] }>();
    let skipped = 0;
    const visible = (c: number | null | undefined) => (((c ?? 0) & 0xff) !== 0 ? c! : null);
    const colorKey = (n: FakeNode) => {
      const stroke = visible(n.style.stroke);
      const fill = visible(n.style.fill);
      const c = grouping === "Stroke" ? stroke : grouping === "Fill" ? fill : stroke ?? fill;
      return c === null ? "no-color" : `color:${(c >>> 0).toString(16).padStart(8, "0")}`;
    };
    const walk = (id: number, inherited: string | null) => {
      const n = doc.nodes[id];
      if (!n) return;
      const a = n.material_preset;
      const material = a.state === "preset" ? a.id : a.state === "unassigned" ? null : inherited;
      const isShape = typeof n.kind === "object" && n.kind !== null && "Shape" in (n.kind as object);
      if (isShape) {
        if (n.cut_line_type === "NoCut") {
          skipped++;
        } else {
          const key =
            grouping === "Single" ? "all"
            : grouping === "Preset" ? (material === null ? "no-preset" : `preset:${material}`)
            : colorKey(n);
          const existing = byKey.get(key);
          if (existing) existing.node_ids.push(id);
          else byKey.set(key, { key, node_ids: [id] });
        }
      }
      for (const c of n.children) walk(c, material);
    };
    walk(doc.root, null);
    // starts is all-null on purpose: the fake carries no geometry to flatten, and null is the
    // real backend's no-outline case — so e2e renders exercise the preview's bounds-corner
    // badge fallback rather than a fixture pretending to be a blade path.
    const passes = [...byKey.values()].map((p) => ({
      key: p.key,
      shape_count: p.node_ids.length,
      node_ids: p.node_ids,
      starts: p.node_ids.map(() => null),
    }));
    // The snapshot itself is the revision, mirroring cutplan::doc_revision hashing
    // snapshot_json: a doc edited back to a previous state is not stale.
    return { passes, skipped_not_cut: skipped, doc_revision: JSON.stringify(doc),
             travel: [] as [number, number, number, number][] };
  }
```

All three handlers honour the grouping, and travel and cut **mirror the backend's exact-once
identity check** — without it the fake accepts stale keys and the suite stays green on a frontend
that cannot work:

```ts
    plan_cut: (a) => {
      const plan = planFromDoc(a.grouping as Grouping);
      if (!holding) return plan;
      return new Promise((resolve) => heldPlans.push(() => resolve(plan)));
    },
    // Mirrors device::travel_for_order's contract, not its geometry: the same stale-plan
    // refusal, the same exact-once identity check over the requested keys, then synthetic
    // segments — one per adjacent pair of *enabled* passes, x encoding the position in the
    // order, because the real command does not route the head to a pass that will not be cut.
    // Requests are recorded on `window.__travelRequests` so a test can assert what the dialog
    // asked for; travel itself lands on a canvas Playwright cannot read.
    travel_for_order: (a) => {
      const passes = a.passes as { key: string; enabled: boolean }[];
      const grouping = a.grouping as Grouping;
      (window as unknown as { __travelRequests: typeof passes[] }).__travelRequests ??= [];
      (window as unknown as { __travelRequests: typeof passes[] }).__travelRequests.push(passes);
      const settle = () => {
        const plan = planFromDoc(grouping);
        // Decided at settle time, like the real command — a request issued before a replan is
        // stale even if it settles after one.
        if (plan.doc_revision !== a.docRevision) {
          throw ipcError("stale_plan", "document changed since the cut was planned; replan");
        }
        const remaining = plan.passes.map((p) => p.key);
        for (const pass of passes) {
          const i = remaining.indexOf(pass.key);
          if (i === -1) {
            throw plan.passes.some((p) => p.key === pass.key)
              ? ipcError("plan_mismatch", "the requested pass list does not name every planned pass exactly once")
              : ipcError("unknown_pass", `no planned pass is called ${pass.key}`);
          }
          remaining.splice(i, 1);
        }
        if (remaining.length > 0) {
          throw ipcError("plan_mismatch", "the requested pass list does not name every planned pass exactly once");
        }
        const cut = passes.filter((p) => p.enabled);
        return cut.slice(1).map((_, i) => [i, 0, i + 1, 0] as [number, number, number, number]);
      };
      if (!holding) return settle();
      return new Promise((resolve, reject) => heldTravel.push(() => {
        try { resolve(settle()); } catch (e) { reject(e); }
      }));
    },
    cut: (a) => {
      const request = a.request as { device_instance_id: string; doc_revision: string;
        grouping: Grouping; passes: { key: string; enabled: boolean }[] };
      if (!connected) throw ipcError("not_connected", "no device connected");
      if (connected.instance_id !== request.device_instance_id) {
        throw ipcError("device_mismatch", "connected device changed since planning");
      }
      const plan = planFromDoc(request.grouping);
      if (plan.doc_revision !== request.doc_revision) {
        throw ipcError("stale_plan", "document changed since the cut was planned; replan");
      }
      if (doc.machine && doc.machine.id !== connected.machine_id) {
        throw ipcError("machine_mismatch", "document is set up for a different machine");
      }
      // A key the plan does not have is refused here too, so rows from a previous grouping
      // cannot cut the wrong shapes just because the fake was more forgiving than Rust.
      for (const pass of request.passes) {
        if (!plan.passes.some((p) => p.key === pass.key)) {
          throw ipcError("unknown_pass", `no planned pass is called ${pass.key}`);
        }
      }
      planPasses = request.passes;
      …
    },
```

Add a `set_material_preset` handler beside `set_cut_line_type`'s, writing the assignment to each
named node and returning the same `Delta` shape the real command does — an unimplemented command
throws (`:68-70`), so the panel control cannot be exercised without it.

- [ ] **Step 4: Run the suite to verify it passes**

Run: `npm --prefix apps/desktop/ui run e2e` then `cargo test --workspace --locked`

Expected: PASS on both.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/e2e/smoke.spec.ts
git commit -m "Make the e2e fake as strict as the backend it stands in for

A fake that ignores the grouping and accepts any key stays green while the
frontend sends rows from one mode under another — the exact failure the dialog
change prevents. It now keys passes, resolves materials down the tree, and
mirrors the exact-once identity check. Also settles #143: the comment named a
hook renamed to __travelRequests and a rule that had become enabled-only."
```

---

### Task 10: The vocabulary and the checklists say what the code does

**Files:**
- Modify: `CONTEXT.md:40-47` (the ColorPass entry), `:59-62` (PassSelection), adding PassKey, Grouping and PresetAssignment
- Modify: `CLAUDE.md:79-99` (the cut-path section), `:154-160` (the vocabulary list)
- Modify: `apps/desktop/MANUAL-CHECKLIST.md:91`, `:102` (two live checks naming removed flags)
- Modify: `CHANGELOG.md`

**Interfaces:** consumes everything above; produces no code.

- [ ] **Step 1: Update `CONTEXT.md`**

Replace the **ColorPass** entry with **DocumentPass**, and add the three new terms:

```markdown
**DocumentPass**:
Every shape in a Document cut in a single run of the blade, together with the PassKey that
says which pass it is. What its shapes share is whatever the Grouping asked for — a colour, a
material preset — or nothing but the operator's request, for the single pass. Which shapes are
cut at all is their CutLineType's business, not their paint's.
_Avoid_: ColorPass (retired with #148), layer, colour group, batch

**PassKey**:
What a DocumentPass is called: `all` for the single pass, a colour, or a MaterialPreset id,
each with one canonical spelling (`color:ff0000ff`, `preset:cameo5-htv`) that the CLI, the IPC
payloads and the cut dialog all use. Absence is its own token — `no-color` for a shape with no
visible paint, `no-preset` for one with no material — never a reserved value inside a mode,
because a preset id is an unrestricted operator string.
_Avoid_: pass id, pass name, colour

**Grouping**:
How a Document's cut shapes are split into DocumentPasses — by stroke colour, fill colour,
stroke-else-fill, material preset, or not at all. A request, not a property of the Document:
the same Document plans differently under two Groupings, so the mode travels with every cut
request, and the dialog holds it with the rows it produced.
_Avoid_: mode, split, pass strategy

**PresetAssignment**:
What a Node says about its material: inherit from the nearest assigned ancestor, no material
at all, or a named MaterialPreset. Resolved during planning rather than stored, so moving a
Node between Layers changes what it is cut with and nothing has to be rewritten.
_Avoid_: material, preset id, optional preset
```

Update the **PassSelection** entry: it names a pass "by its PassKey", not by colour.

- [ ] **Step 2: Update `CLAUDE.md`**

In the cut-path section, the plain-CLI paragraph (`:90-97`) reads as `--group-by single` rather than "no `--by-color`", and the pass-identity sentence names the key. In the conventions list (`:156-160`), replace `ColorPass` with `DocumentPass`, `PassKey`, `Grouping` and `PresetAssignment`.

- [ ] **Step 3: Update the manual checklist**

`apps/desktop/MANUAL-CHECKLIST.md:91` and `:102` name flags this change removes, and a live operator check that cannot be run is worse than none:

```markdown
- [ ] `cuthulhu cut a.svg --skip-pass color:FF0000FF` — refused, naming `--group-by single`.
```

```markdown
- [ ] `cuthulhu cut --group-by color` on the Puma still prompts per pass and completes.
```

Add one check for the new capability, since it is operator-visible and only real material proves it:

```markdown
- [ ] `cuthulhu cut two-materials.cut --group-by preset` — one pass per material, each cut with
      that preset's settings, prompting between them.
```

- [ ] **Step 4: Update `CHANGELOG.md`**

Following the file's existing format, the operator-visible facts: grouping is selectable in the cut dialog and with `--group-by`; a Node can carry a material preset that Layers pass down; `--by-color` and `--skip-color` are gone, replaced by `--group-by` and `--skip-pass`; `--order` takes pass keys and is repeatable rather than comma-separated.

- [ ] **Step 5: Verify and commit**

```bash
cargo test --workspace --locked && npm --prefix apps/desktop/ui test && npm --prefix apps/desktop/ui run e2e
# Live source and normative docs only: historical plans and specs record the old names on
# purpose, and CHANGELOG.md names the removed flags as removed. `!` because rg exits 1 when it
# finds nothing, which is the passing case here.
! rg -n 'ColorPass|--by-color|--skip-color|UnknownPassColor' crates apps CONTEXT.md CLAUDE.md \
    --glob '!apps/desktop/ui/dist/**'
```

Expected: tests PASS, and the `rg` line succeeds by finding nothing. A surviving `ColorPass` is a rename this plan missed.

```bash
git add CONTEXT.md CLAUDE.md CHANGELOG.md apps/desktop/MANUAL-CHECKLIST.md
git commit -m "Retire ColorPass from the vocabulary, since a pass is no longer a colour

CONTEXT.md is normative, so a term the code no longer has cannot stay in it:
DocumentPass, PassKey, Grouping and PresetAssignment replace it. The manual
checklist had two live operator checks running flags this change removes, which
is worse than having none."
```

---

## Verification

The plan is done when, from a clean tree on `vcolombo/pass-grouping`:

- `cargo test --workspace --locked` passes and `Cargo.lock` is unchanged.
- `npm --prefix apps/desktop/ui test` and `npm --prefix apps/desktop/ui run e2e` pass, with `apps/desktop/ui/dist` committed and current.
- `cuthulhu cut file.svg --device cameo5 --dry-run` prints bytes with no pass header, exactly as before this change; `--group-by color --dry-run` prints one labelled pass per visible paint; `--group-by preset --skip-pass no-preset --dry-run` on a file where nothing carries a material refuses by name rather than reporting an empty file.
- The `rg` assertion in Task 10 Step 5 finds nothing.
- Hardware verification is not required for correctness — every decision lands before a byte reaches a Transport, and the `MockTransport`/dry-run paths cover it — but `apps/desktop/MANUAL-CHECKLIST.md` gains the preset-grouping check, because "each pass cut with its own material's settings" is a claim only real material can settle.
