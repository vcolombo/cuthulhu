<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Cuttability as an explicit per-node attribute — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `Node` a `CutLineType`, default it to `Cut` at import, derive it from stroke for documents saved before it existed, and make `cutplan::plan_passes` filter on it instead of on the stroke — so fill-only art cuts, a stroked shape can be told not to, and stroke goes back to being paint plus a pass-grouping key. Closes issue #144 and unblocks #45 and #56, which both extend the enum this adds.

**Architecture:** `CutLineType { Cut, NoCut }` in `document`, a sibling field of `Node::style` rather than a member of `Style` — that separation *is* the #68 decision. Old manifests carry no such field and `manifest.json` has no schema version, so absence is the only migration signal: `#[serde(from = "NodeWire")]` on `Node` decodes an `Option<CutLineType>` and derives from `style.stroke` when it is `None`, which covers every path a `Node` is deserialized on (project load, snapshot, IPC) rather than only `fileio::load_project`. `plan_passes` swaps one predicate and gains a colour key that falls back to fill; the plain CLI cut stops overwriting strokes and asks for `Grouping::Single` instead. `trace::mirror_fill_onto_stroke` — a workaround whose stated premise this change removes — is deleted. The desktop gains its first non-transform per-node command, `set_cut_line_type`, and a control on the properties panel.

**Tech Stack:** Rust 2021 (`document`, `fileio`, `cutplan`, `trace`, `cli`, `apps/desktop`), React + TypeScript (`apps/desktop/ui`). **No new dependencies** in either language: `serde_json`, `zip` and `tempfile` are already dependencies of the crates whose tests need them.

**Spec:** `docs/superpowers/specs/2026-08-14-cuttability-attribute-design.md` — read it first. It holds the reference-application research, the three scoping decisions, and the rejected alternatives (notably why passes are not merged after the fact).

## Global Constraints

- **SPDX header on every file** — `// SPDX-License-Identifier: GPL-3.0-or-later` (`<!-- -->` in Markdown). Every file touched here already has one; do not remove it.
- **`cargo test --workspace --locked` is the gate**, and `--locked` is mandatory. This change adds no dependency, so `Cargo.lock` must not change; if it does, something was added that this plan did not intend.
- **`ui/dist` is committed.** Any task that edits `apps/desktop/ui/src` must end with `npm --prefix apps/desktop/ui run build` and commit `apps/desktop/ui/dist` in the same commit — CI rebuilds and fails on a stale bundle. Tasks 4 and 8 are the two that touch `ui/src`.
- **`CONTEXT.md` is normative vocabulary**, and this change edits it. Terms in play: **Node** (not "element"/"object"), **ColorPass** (not "layer"/"colour group"), **Preflight** (not "validation"), **CutPlan**. The new term is **CutLineType**; the ColorPass entry's second sentence ("A shape with no stroke belongs to no ColorPass and is not cut") stops being true in Task 3 and must change there, not later.
- **Comments explain why, not what.** Every comment specified below records a constraint, a trap, or a decision that was taken against an alternative. Do not add comments restating the code.
- **The `ponytail:` marker marks a deliberate simplification** with its ceiling and upgrade path. The marker is the word, not the comment form — the workspace carries it on both `//` lines (`crates/driver-core/src/manager.rs:174`) and `///` doc comments (`crates/cut-host/src/resolve.rs:92`, `:206`), and `grep -rn "ponytail:"` finds either. `NodeWire` gets one, on its doc comment, because it documents the type: it exists only for documents written before the attribute did.
- **The import default and the migration default are deliberately different values.** `Cut` for a new import, derived-from-stroke for an old file. Anywhere the two look like duplicated logic that could be unified, they must not be — a saved project must cut exactly what it cut before, on real material.
- **The e2e fake mirrors `Document::snapshot_json()`** (`CLAUDE.md:135-136`). `apps/desktop/ui/e2e/smoke.spec.ts` re-implements `plan_passes` at `:276-311`; when the predicate changes, so does the fake, or the suite lies.
- **Out of scope, and must not creep in:** a configurable import default (#54 owns it — the default ships hardcoded), `CutEdge` geometry (#56), production roles and grouping-mode pickers (#45), print-and-cut (#25), and any change to the outline-resolution ordering #139 established. The predicate swaps; the sequence does not.

## File Structure

| File | Responsibility after this change |
|---|---|
| `crates/document/src/node.rs` | Owns `CutLineType`, the `Node` field, the import default in both constructors, and `NodeWire` — the one place that knows what an old document's absent field means. |
| `crates/fileio/src/project.rs` | Unchanged code; gains the load-bearing test that a real `.cut` written before the attribute loads with derived values. |
| `crates/fileio/src/import.rs` | Unchanged code; gains the assertion that an imported path is cuttable regardless of paint. |
| `crates/cutplan/src/passes.rs` | Filters on `cut_line_type`; owns `pass_key` (stroke, else fill) and `Grouping`; `skipped_not_cut` replaces `skipped_no_stroke`. |
| `crates/cutplan/src/plan.rs` | Unchanged behaviour; its comment claiming `plan_passes` only builds `Some(color)` passes becomes false and is corrected. |
| `crates/trace/src/lib.rs` | Loses `mirror_fill_onto_stroke` and `attr_value`. Traced output carries fill only. |
| `crates/cli/src/pipeline.rs` | Plain cut imports without touching paint and asks for `Grouping::Single`; `doc_from_svg_all_cuttable` and `CUT_STROKE` are gone. |
| `apps/desktop/src/{state,ipc,main}.rs` | Gain `set_cut_line_type`, the first per-node command that is not a transform or a structure edit. |
| `apps/desktop/ui/src/panels/{PropertiesPanel.tsx,cutLineType.ts}` | The operator's control, plus the pure selection-value helper it renders from. |
| `apps/desktop/ui/src/cut/CutDialog.tsx` | The "not cut" readout says *why* alongside the count. |
| `apps/desktop/ui/e2e/smoke.spec.ts` | The fake's Nodes carry the attribute and its `planFromDoc` filters on it. |

**Task order is load-bearing.** Task 1 adds an attribute nothing reads yet. Task 2 proves the migration through a real project file. Task 3 makes the planner read it — the first behaviour change. Task 4 renames the count that Task 3 made a lie. Task 5 removes the CLI's stroke overwrite, which is only safe once cuttability no longer rides on the stroke. Task 6 deletes the trace workaround, which is only safe once fill-only geometry plans passes. Tasks 7 and 8 give the operator a way to set the attribute. Every task ends green on `cargo test --workspace --locked`.

---

### Task 1: `Node` carries a cut line type, and an old document derives it

**Files:**
- Modify: `crates/document/src/node.rs:14-48` (add the enum above `Node`, the field, the wire struct, and the constructors' default)
- Test: `crates/document/src/node.rs:50-62` (in the existing `mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces: `document::CutLineType`, `Node::cut_line_type`. Task 3 filters on it, Task 6's deletion depends on it existing, Task 7 edits it. Re-exported automatically — `crates/document/src/lib.rs` already does `pub use node::*`.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/document/src/node.rs`:

```rust
    /// The import default, asserted at the only two places a `Node` is built. Every
    /// imported path arrives cuttable (#68), which is what both reference applications
    /// do and what `cuthulhu cut` already meant.
    #[test]
    fn a_new_node_is_cut() {
        let mut ids = IdGen::default();
        let shape = Node::shape(ids.next(), ShapeKind::Rect { w: 10.0, h: 5.0 });
        let group = Node::container(ids.next(), NodeKind::Group);
        assert_eq!(shape.cut_line_type, CutLineType::Cut);
        assert_eq!(group.cut_line_type, CutLineType::Cut);
    }

    /// A document written before the attribute existed must cut exactly what it cut then.
    /// The three cases are the whole of the old rule (`plan_passes`' stroke filter): a
    /// stroke, no stroke, and a stroke nobody can see.
    #[test]
    fn a_node_saved_without_the_attribute_derives_it_from_its_stroke() {
        let node = |stroke: &str| -> Node {
            let json = format!(
                r#"{{"id":7,"kind":{{"Shape":{{"Rect":{{"w":1.0,"h":1.0}}}}}},
                     "transform":[1.0,0.0,0.0,1.0,0.0,0.0],
                     "style":{{"stroke":{stroke},"fill":null}},"children":[]}}"#
            );
            serde_json::from_str(&json).unwrap()
        };
        // 0x000000FF — opaque black, the old `Style::default()`.
        assert_eq!(node("255").cut_line_type, CutLineType::Cut);
        assert_eq!(node("null").cut_line_type, CutLineType::NoCut);
        // 0xFF000000 — red at alpha 0, which `plan_passes` skipped exactly like `None`.
        assert_eq!(node("4278190080").cut_line_type, CutLineType::NoCut);
    }

    /// The value on the wire wins over the derivation, or a file saved after this change
    /// would lose an operator's `NoCut` the next time it was opened. `Serialize` stays
    /// derived so a document round-trips once and is never ambiguous again.
    #[test]
    fn an_explicit_attribute_survives_and_is_always_written() {
        let mut node = Node::shape(NodeId(1), ShapeKind::Rect { w: 1.0, h: 1.0 });
        node.cut_line_type = CutLineType::NoCut;
        assert_eq!(node.style.stroke, Some(0x000000FF), "premise: the stroke says Cut");
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains(r#""cut_line_type":"NoCut""#), "{json}");
        assert_eq!(serde_json::from_str::<Node>(&json).unwrap(), node);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p document node::tests`

Expected: compile errors — `cannot find type 'CutLineType' in this scope` and `no field 'cut_line_type' on type 'Node'`.

- [ ] **Step 3: Write the implementation**

Insert above `Node` (after `crates/document/src/node.rs:29`):

```rust
/// Whether a Node's geometry is cut, and how. A sibling of `Style`, never a member of it:
/// paint is display and a pass-grouping key, cuttability is production intent, and #68
/// settled that inferring one from the other is what made fill-only art uncuttable and a
/// stroked shape impossible to exclude.
///
/// Two members ship. The others are named here so nobody invents a parallel attribute:
/// `CutEdge` is #56's, and `Draw` / `Score` / `PrintCutCut` / `PrintCutPrint` /
/// `ColorLayerAlignment` are #45's. Adding one is a variant here plus a match arm in
/// `cutplan::plan_passes` — which is the point of an enum over a bool, since "draw with a
/// pen" and "print, do not cut" are states a bool cannot hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum CutLineType { Cut, NoCut }
```

Replace `Node` and its `impl` (`crates/document/src/node.rs:31-48`):

```rust
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(from = "NodeWire")]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub transform: Affine,   // relative to parent
    pub style: Style,
    pub cut_line_type: CutLineType,
    pub children: Vec<NodeId>,
}
impl Node {
    pub fn shape(id: NodeId, kind: ShapeKind) -> Node {
        Node { id, kind: NodeKind::Shape(kind), transform: Affine::identity(),
               style: Style::default(), cut_line_type: CutLineType::Cut, children: vec![] }
    }
    pub fn container(id: NodeId, kind: NodeKind) -> Node {
        Node { id, kind, transform: Affine::identity(),
               style: Style::default(), cut_line_type: CutLineType::Cut, children: vec![] }
    }
}

/// `Node` as it may appear on disk. `manifest.json` is a bare `serde_json::to_string` of
/// `Document` with no schema version (`Document::snapshot_json`), so a missing field is the
/// only migration signal there is — and `#[serde(default)]` cannot serve as one twice over:
/// it cannot tell an absent field from an explicit `Cut`, and it cannot see the node's
/// stroke, which is the only thing that says what an old document used to cut.
///
/// This sits on `Node` rather than in `fileio::load_project`, where the legacy-machine-id
/// migration lives, because a `Node` is also deserialized through `Document::snapshot_json`
/// and across IPC; confining it to project load would leave those paths to guess.
///
/// ponytail: exists only for documents written before the attribute did. Once no such file
/// is expected in the wild, delete this and derive `Deserialize` on `Node` again.
#[derive(Deserialize)]
struct NodeWire {
    id: NodeId,
    kind: NodeKind,
    transform: Affine,
    style: Style,
    #[serde(default)]
    cut_line_type: Option<CutLineType>,
    children: Vec<NodeId>,
}

impl From<NodeWire> for Node {
    fn from(w: NodeWire) -> Node {
        let cut_line_type = w.cut_line_type.unwrap_or_else(|| match &w.kind {
            // A container's attribute is never read — `plan_passes` reads it only under
            // `NodeKind::Shape` — so match a freshly built container rather than derive a
            // value that would differ from one for no observable reason.
            NodeKind::Group | NodeKind::Layer => CutLineType::Cut,
            // The old rule, verbatim: `plan_passes` cut a shape whose stroke was present
            // and not fully transparent, and skipped every other shape. Deriving it here
            // is what keeps a saved project cutting what it cut before — a plain default
            // of `Cut` would silently start cutting shapes it never cut, on material.
            NodeKind::Shape(_) => match w.style.stroke {
                Some(c) if c & 0xFF != 0 => CutLineType::Cut,
                _ => CutLineType::NoCut,
            },
        });
        Node { id: w.id, kind: w.kind, transform: w.transform, style: w.style,
               cut_line_type, children: w.children }
    }
}
```

Field position matters only in that it changes the serialized JSON and therefore every document's `doc_revision` (`cutplan::passes::doc_revision` hashes the snapshot). That is expected and harmless: a revision is only ever compared against one taken in the same process run.

This shape was prototyped against serde 1 before the plan was written, so three things it
depends on are known rather than assumed: `#[serde(from = "…")]` coexists with `Deserialize` in
the derive list, `#[serde(default)]` on an `Option` field accepts an absent key, and an explicit
value on the wire wins over the `From` impl's derivation. A `Node` serializes as
`{"id":…,"kind":…,"transform":…,"style":{…},"cut_line_type":"Cut","children":[]}` — one bare
string for the enum, matching how `NodeKind`'s unit variants already appear (`"Group"`, `"Layer"`)
and what the TypeScript mirror in Task 8 declares.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. Nothing reads the new field yet, so `plan_passes` still filters on stroke and every existing test holds — including `snapshot_round_trips_through_json` (`crates/document/src/snapshot.rs:17`) and `save_then_load_round_trips_document` (`crates/fileio/src/project.rs:60`), both of which compare a `Document` written *after* this change.

- [ ] **Step 5: Commit**

```bash
git add crates/document/src/node.rs
git commit -m "Give a Node an explicit cut line type, and derive it for documents that predate it

#68 settled that cuttability is a per-node attribute rather than something
read off the stroke. The value a new import gets and the value an old file
gets are deliberately different: a saved project must cut exactly what it
cut before, so absence derives from the stroke while a fresh Node is Cut."
```

---

### Task 2: A project written before the attribute loads unchanged

**Files:**
- Test: `crates/fileio/src/project.rs` (in the existing `mod tests`, after `legacy_machine_ids_migrate_on_load` at `:57`)
- Test: `crates/fileio/src/import.rs` (in the existing `mod tests`, after `import_svg_produces_one_add_per_path`)

**Interfaces:**
- Consumes: `document::CutLineType` and the `NodeWire` migration from Task 1.
- Produces: no code. This is the migration's load-bearing proof at the level an operator experiences it — a real `.cut` file — because Task 1's tests exercise serde in isolation and cannot see the container.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/fileio/src/project.rs`:

```rust
    /// The migration through a real project file, which is the level it matters at:
    /// `legacy_machine_ids_migrate_on_load` above could plant a legacy *value* in a live
    /// `Document`, but an absent field cannot be planted that way — `save_project` always
    /// writes it. So the manifest is pruned rather than hand-written: everything except
    /// `cut_line_type` is exactly what `save_project` emits today, so the fixture cannot
    /// drift from `Document`'s real shape.
    #[test]
    fn a_project_saved_before_cuttability_derives_it_from_stroke() {
        let mut doc = document::Document::new();
        let mut stroked = document::Node::shape(doc.ids.next(),
            document::ShapeKind::Rect { w: 10.0, h: 10.0 });
        stroked.style = document::Style { stroke: Some(0xFF0000FF), fill: None };
        let stroked_id = stroked.id;
        let mut fill_only = document::Node::shape(doc.ids.next(),
            document::ShapeKind::Rect { w: 10.0, h: 10.0 });
        fill_only.style = document::Style { stroke: None, fill: Some(0x00FF00FF) };
        let fill_only_id = fill_only.id;
        doc.apply(document::Delta(vec![
            document::NodeOp::Add { parent: doc.root, node: stroked, index: usize::MAX },
            document::NodeOp::Add { parent: doc.root, node: fill_only, index: usize::MAX },
        ]));

        let mut manifest: serde_json::Value = serde_json::from_str(&doc.snapshot_json()).unwrap();
        for node in manifest["nodes"].as_object_mut().unwrap().values_mut() {
            assert!(node.as_object_mut().unwrap().remove("cut_line_type").is_some(),
                "premise: every node is written with the field, so pruning it makes an old file");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.cut");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.finish().unwrap();

        let back = load_project(&path).unwrap();
        assert_eq!(back.get(stroked_id).unwrap().cut_line_type, document::CutLineType::Cut);
        assert_eq!(back.get(fill_only_id).unwrap().cut_line_type, document::CutLineType::NoCut);
    }
```

`design.svg` is deliberately absent from the fixture: `load_project` reads only `manifest.json` (`crates/fileio/src/project.rs:29`), so a container without it is a valid old file for this test's purpose and one fewer thing to keep in step.

Append to `mod tests` in `crates/fileio/src/import.rs`:

```rust
    /// Import defaults to cuttable whatever the paint says — the fill-only clipart that
    /// used to import as "nothing to cut" is the case this exists for. The value is the
    /// constructor's; this pins that `import_svg` does not overwrite it from the stroke,
    /// which is the mistake the old `plan_passes` rule would invite.
    #[test]
    fn an_imported_path_is_cut_even_with_no_stroke() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"
            viewBox="0 0 10 10"><rect width="5" height="5" fill="#00ff00"/></svg>"#;
        let mut ids = document::IdGen::default();
        let (delta, _skipped) = import_svg(svg, &mut ids, document::NodeId(1)).unwrap();
        let nodes: Vec<&document::Node> = delta.0.iter()
            .filter_map(|op| match op { document::NodeOp::Add { node, .. } => Some(node), _ => None })
            .collect();
        assert!(!nodes.is_empty(), "premise: the rect imported");
        for node in nodes {
            assert_eq!(node.style.stroke, None, "premise: fill-only art has no stroke");
            assert_eq!(node.cut_line_type, document::CutLineType::Cut);
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p fileio cuttability` then `cargo test -p fileio an_imported_path_is_cut`

Expected: the first fails at the pruning assertion or the final `assert_eq!` only if Task 1 is absent; with Task 1 in place both must **pass**. If either fails, the migration is wrong — do not adjust the test, fix `From<NodeWire>`.

That makes this task's steps 1–2 a verification rather than a red-then-green pair, and that is deliberate: the behaviour was implemented in Task 1, and its proof belongs in the crate that owns the file format. If a red bar is wanted first, run these tests against `HEAD~1`.

- [ ] **Step 3: Run the whole workspace gate**

Run: `cargo test --workspace --locked`

Expected: PASS, 17 tests in `fileio` (15 before, 2 added).

- [ ] **Step 4: Commit**

```bash
git add crates/fileio/src/project.rs crates/fileio/src/import.rs
git commit -m "Pin the migration through a real project file, and the import default through a real import

A pruned manifest is the only way to build a file that predates the field:
save_project always writes it. Pruning what save_project emits, rather than
hand-writing a fixture, keeps the test from drifting from Document's shape."
```

---

### Task 3: `plan_passes` filters on the attribute, and a pass falls back to fill

**Files:**
- Modify: `crates/cutplan/src/passes.rs:6` (import `CutLineType`, `Style`), `:14-16` (`ColorPass` doc), `:62-66` (`plan_passes` doc), `:90-114` (the predicate and the grouping key), and add `pass_key` above `plan_passes`
- Modify: `crates/cutplan/src/passes.rs:188-197` (test helper), and three fixtures that made a shape skipped by removing its stroke — `:239-264`, `:341` and `:362-363`
- Modify: `crates/cli/src/pipeline.rs:280-286` (the test whose premise inverts)
- Modify: `CONTEXT.md:40-42` (ColorPass), and add the CutLineType term
- Modify: `docs/superpowers/specs/2026-07-22-editor-shell-design.md:85` (mark superseded, do not edit silently)
- Test: `crates/cutplan/src/passes.rs` (in `mod tests`)

**Interfaces:**
- Consumes: `Node::cut_line_type` (Task 1).
- Produces: `pass_key`, private to `cutplan`. `DocumentPasses::skipped_no_stroke` keeps its name for one more commit — Task 4 renames it, because renaming it here would drag the desktop and TypeScript into a commit about the planner.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/cutplan/src/passes.rs`:

```rust
    /// The point of the whole change: geometry with no stroke is cut when it says it is,
    /// and its pass is keyed on the fill so an operator can still tell passes apart.
    #[test]
    fn a_fill_only_shape_that_is_cut_plans_into_a_pass_keyed_on_its_fill() {
        let mut doc = Document::new();
        let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 10.0, h: 10.0 });
        node.style = Style { stroke: None, fill: Some(0x00FF00FF) };
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));

        let planned = plan_passes(&doc).unwrap();
        assert_eq!(planned.skipped_no_stroke, 0);
        assert_eq!(planned.passes.len(), 1);
        assert_eq!(planned.passes[0].color, Some(0x00FF00FF));
    }

    /// The other direction, which the old rule could not express at all: a shape with a
    /// perfectly good stroke that the operator has marked not to cut.
    #[test]
    fn a_stroked_shape_marked_no_cut_plans_into_nothing() {
        let mut doc = Document::new();
        let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 10.0, h: 10.0 });
        node.style = Style { stroke: Some(0xFF0000FF), fill: None };
        node.cut_line_type = CutLineType::NoCut;
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));

        let planned = plan_passes(&doc).unwrap();
        assert!(planned.passes.is_empty());
        assert_eq!(planned.skipped_no_stroke, 1);
    }

    /// Neither paint, and cut anyway. `ColorPass::color` has always been `Option<u32>`;
    /// this is the first thing that can make it `None`, so every consumer that renders a
    /// swatch or prints a header now has a case that reaches it.
    #[test]
    fn a_cut_shape_with_no_paint_lands_in_the_colorless_pass() {
        let mut doc = Document::new();
        let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 10.0, h: 10.0 });
        node.style = Style { stroke: None, fill: None };
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));

        let planned = plan_passes(&doc).unwrap();
        assert_eq!(planned.passes.len(), 1);
        assert_eq!(planned.passes[0].color, None);
    }

    /// Alpha-0 paint is not a colour to group by, in either channel — a fully transparent
    /// stroke used to mean "not cut", and the fallback must not resurrect it as a key.
    #[test]
    fn transparent_paint_is_not_a_pass_key() {
        let mut doc = Document::new();
        let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 10.0, h: 10.0 });
        node.style = Style { stroke: Some(0xFF000000), fill: Some(0x00FF0000) };
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));

        let planned = plan_passes(&doc).unwrap();
        assert_eq!(planned.passes.len(), 1);
        assert_eq!(planned.passes[0].color, None, "both paints are invisible, so neither keys the pass");
    }
```

Check the imports at the top of `mod tests` before running: the module uses `use super::*;`, which brings `Style` in only once `passes.rs` itself imports it in Step 3. `CutLineType`, `Delta` and `NodeOp` need adding to that `use` list if absent.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cutplan passes::tests`

Expected: the four new tests fail — a fill-only shape reports `skipped_no_stroke: 1` and no passes, and the `NoCut` case plans a pass it should not.

- [ ] **Step 3: Write the implementation**

In `crates/cutplan/src/passes.rs`, extend the `document` import at `:6` with `CutLineType` and `Style`, and insert above `plan_passes`:

```rust
/// The colour a cut shape's pass is keyed on: its stroke if it has a visible one, else its
/// fill. Alpha-0 counts as absent in both, exactly as a 0-alpha stroke did when the stroke
/// decided cuttability.
///
/// A fallback is needed because a shape with no stroke can be cut since #144 — traced and
/// fill-only art is the common case — and a pass with no colour at all is something an
/// operator cannot recognise in a pass list. `None` remains possible for a shape with no
/// visible paint whatsoever.
fn pass_key(style: &Style) -> Option<u32> {
    style.stroke.filter(|c| c & 0xFF != 0).or(style.fill.filter(|c| c & 0xFF != 0))
}
```

Replace the `NodeKind::Shape(_)` arm's predicate and grouping (`crates/cutplan/src/passes.rs:90-114`) with:

```rust
            NodeKind::Shape(_) => {
                // The predicate #144 moved here off the stroke. The *ordering* is #139's and
                // must not move with it: the outline stays unresolved until the shape is
                // known to be cut, so a font or path-data failure on a shape nobody cuts
                // cannot refuse the whole plan.
                match node.cut_line_type {
                    CutLineType::NoCut => skipped_no_stroke += 1,
                    CutLineType::Cut => {
                        // `None` here is `shape_outline`'s container signal, which `NodeKind`
                        // has already ruled out, so no `ShapeKind` reaches this today. A new
                        // one added without its own arm there would fall into its catch-all
                        // and land here — refuse rather than skip, because this branch is
                        // past the cut filter: the shape *is* being cut, and quietly dropping
                        // it would send a partial plan to the blade. Same reason a shape whose
                        // outline fails to parse refuses instead of being skipped.
                        let Some(path) = shape_outline(node).map_err(|e| PlanError::BadShape(id, e))?
                        else {
                            return Err(PlanError::BadShape(
                                id, "this kind of shape cannot be resolved to an outline".into()));
                        };
                        let polylines = path.transformed(&world).flatten(0.1);
                        let shape = PlannedShape { node_id: id, polylines };
                        let color = pass_key(&node.style);
                        match passes.iter_mut().find(|p| p.color == color) {
                            Some(pass) => pass.shapes.push(shape),
                            None => passes.push(ColorPass { color, shapes: vec![shape] }),
                        }
                    }
                }
            }
```

Update the two doc comments the change falsifies. `ColorPass` (`:14`):

```rust
/// All shapes cut together as one pass, keyed on the colour they share (0xRRGGBBAA) —
/// their stroke where they have a visible one, else their fill. `None` is a pass of shapes
/// with no visible paint at all.
```

`plan_passes` (`:62-65`), first sentence:

```rust
/// Walk the document in preorder from `doc.root`, group the shapes whose `CutLineType` is
/// `Cut` by the colour `pass_key` gives them, and flatten each shape's outline under its
/// accumulated world transform. A `NoCut` shape is counted, not cut. Iterative (explicit
/// stack) so depth is not bounded by the Rust call stack; a `visited` set catches cycles in
/// malformed docs.
```

Migrate **three** existing fixtures whose premise was the stroke. The helper at `:188-197` keeps `with_stroke` (colour grouping is still real) and gains a sibling:

```rust
    /// Mark a node not-cut. Since #144 a strokeless shape is cut by default, so a test that
    /// wants a skipped shape has to say so rather than leaving the stroke off.
    fn with_no_cut(mut node: document::Node) -> document::Node {
        node.cut_line_type = CutLineType::NoCut;
        node
    }
```

Three call sites need it, and two of them are #139's own tests — the plan's earlier claim that all three #139 tests pass untouched was wrong, and this is the correction:

- `plans_group_by_stroke_rgba_with_single_traversal_transforms` (`:239`): the strokeless node that made the skipped count 1 (`:264`) becomes `with_no_cut(...)`; leave the assertion at 1.
- `a_skipped_text_that_cannot_resolve_does_not_refuse_the_plan` (`:310`): the "same text, strokeless" node at `:341` is `with_stroke(…, None)`, which after this task is a *cut* shape whose outline cannot resolve — so `plan_passes` would refuse and the `.expect()` at `:344` would panic. It becomes `with_no_cut(Node::shape(text, unresolvable))`. The premise block at `:326-335` is untouched: it makes the same text refuse by giving it a stroke, and a stroked shape is still `Cut` by default.
- `a_skipped_path_with_unreadable_data_does_not_refuse_the_plan` (`:353`): same shape of change at `:362-363`.

`a_cut_shape_with_unreadable_data_still_refuses_the_plan` (`:374`) is genuinely untouched — its shape is stroked and cut, and it must keep refusing.

What the three tests pin does not change: they pin *when* the outline resolves, and after this task they say it in the vocabulary that now decides it. A fixture that still said "strokeless" would be asserting the old rule.

Then the CLI test whose premise inverts, `an_svg_with_nothing_stroked_is_refused_by_name` (`crates/cli/src/pipeline.rs:280-286`). Fill-only art is now cuttable, so `--by-color` plans it instead of refusing. Replace it with the truth it becomes, **through the same production entry point it called** — `plan_cut_from_svg`, not a hand-rolled `doc_from_svg` + `plan_passes`, or the test would stop covering the caller whose behaviour it is about:

```rust
/// A fill-only SVG used to be refused by name here, because `plan_passes` cut only stroked
/// shapes. Since #144 `--by-color` plans it, keyed on the fill, and the refusal it used to
/// produce belongs to an SVG with no geometry at all — which
/// `plain_cut_of_an_empty_svg_says_nothing_to_cut` covers.
#[test]
fn a_fill_only_svg_is_planned_by_color_on_its_fill() {
    let plan = plan_cut_from_svg(FILL_ONLY, &driver(), &settings(), &[], None, false).unwrap();
    assert_eq!(plan.passes.len(), 1);
    assert_eq!(plan.passes[0].color, Some(0x00FF00FF));
}
```

Reuse the deleted test's own fixture, driver and settings helpers rather than introducing `FILL_ONLY`, and take the expected colour from that fixture's fill.

Finally the two documents that state the old rule. `CONTEXT.md:40-42`, the ColorPass entry:

```markdown
**ColorPass**:
Every shape in a Document cut in a single run of the blade, grouped by the colour they
share — their stroke where they have one, otherwise their fill. Which shapes are cut at all
is their CutLineType's business, not their paint's.
_Avoid_: layer, colour group, batch
```

and a new term in the same section:

```markdown
**CutLineType**:
Whether a Node is cut, and how — `Cut` or `NoCut` today. An explicit attribute of the Node
rather than something read off its stroke, defaulted to `Cut` at import.
_Avoid_: cut style, cuttable flag, cut attribute
```

`docs/superpowers/specs/2026-07-22-editor-shell-design.md:85` says "for a cutting tool, *stroke = the cut line*". Append a superseding note rather than editing the sentence — the spec is a record of what was believed in SP3:

```markdown
> Superseded by `docs/superpowers/specs/2026-08-14-cuttability-attribute-design.md` (#144):
> stroke is paint and a pass-grouping key; a Node's `CutLineType` decides what is cut.
```

**Consumers of a colourless pass — already audited, do not re-derive.** `ColorPass::color` has
always been `Option<u32>`, but until this task nothing could produce `None`, so the audit the spec
asks for was done while planning. All nine sites already model and branch on the absence:
`crates/cutplan/src/plan.rs:127` (selection match — `None == None` matches the colourless pass),
`:75-79` (refusal sentence), `crates/cli/src/cut.rs:45-50` (`format_pass_color` → `"none"`) and
`:103-106`, `crates/cli/src/main.rs:173` (dry-run header), `crates/cli/src/pipeline.rs:104-122`
(`pass_order` never reorders or skips a colourless pass, because `--order`/`--skip-color` name
colours), `apps/desktop/src/device.rs:1181-1185` (`travel_for_order` matches by colour),
`apps/desktop/ui/src/cut/CutPreview.tsx:76` and `:119`, `apps/desktop/ui/src/cut/CutDialog.tsx:564`
(swatch falls back to `var(--muted)`). `apps/desktop/ui/src/cut/viewmodel.test.ts:212` already
drives a `{ color: null }` row through `toTravelPasses`, so the TypeScript layer has a live
fixture. **Nothing needs changing here** — but one latent defect is worth knowing about and is
deliberately left alone: `CutDialog.tsx:555` keys its React list on `row.color ?? "none"`, which
is unique only while colours are. Two colourless passes would collide, and no grouping mode in
this plan produces two (`ByColor` merges them into one; `Single` produces exactly one, and the
desktop never asks for `Single`). #45, which adds grouping modes the desktop *can* select, is
where that key must become the row index.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. The three #139 ordering tests (`crates/cutplan/src/passes.rs:310`, `:353`, `:374`) must still pass, and two of them only after their fixtures move from a missing stroke to `with_no_cut` — what must not change is the assertion each makes about *when* the outline resolves. If a fixture change tempts you into weakening one of those assertions, stop: #139's contract is the reason this task exists in the order it does. The plain CLI path still plans exactly one pass because `doc_from_svg_all_cuttable` still stamps a uniform stroke; Task 5 removes it.

- [ ] **Step 5: Commit**

```bash
git add crates/cutplan/src/passes.rs crates/cli/src/pipeline.rs CONTEXT.md \
        docs/superpowers/specs/2026-07-22-editor-shell-design.md
git commit -m "Cut what the Node says to cut, and key its pass on stroke or fill

The predicate moves off the stroke; #139's ordering does not move with it.
Grouping needs an answer for shapes with no stroke now that they can be
cut, so a pass is keyed on the fill when there is no visible stroke —
which makes ColorPass::color None reachable for the first time."
```

---

### Task 4: The skipped count says what it now counts

**Files:**
- Modify: `crates/cutplan/src/passes.rs:28`, `:70`, `:93`, `:121`, and the assertions at `:264`, `:347`, `:368`
- Modify: `apps/desktop/src/device.rs:1106`, `:1138`
- Modify: `apps/desktop/ui/src/ipc.ts:155`
- Modify: `apps/desktop/ui/src/cut/CutDialog.tsx:109`, `:194`, `:623`
- Modify: `apps/desktop/ui/e2e/smoke.spec.ts:279`, `:287`, `:310`
- Build: `apps/desktop/ui/dist`

**Interfaces:**
- Consumes: Task 3's predicate.
- Produces: `DocumentPasses::skipped_not_cut`. One name across Rust, the IPC DTO, TypeScript and the e2e fake — it is a single commit because rustc and tsc both refuse the halfway state.

- [ ] **Step 1: Rename, mechanically and completely**

`skipped_no_stroke` → `skipped_not_cut`. Counted against the current tree: **13 source occurrences** — 7 in `crates/cutplan/src/passes.rs` (field, counter, increment, struct build, and the three test assertions), 2 in `apps/desktop/src/device.rs`, and 1 each in `apps/desktop/ui/src/ipc.ts`, `apps/desktop/ui/src/cut/CutDialog.tsx`, `apps/desktop/ui/e2e/smoke.spec.ts` and `crates/trace/src/lib.rs` (a doc comment, whose whole block Task 6 deletes — leave it to Task 6 rather than renaming a sentence that is about to go). Plus the compiled copy in `apps/desktop/ui/dist`, which this task's rebuild regenerates. The camelCase locals rename with the field: `skippedNoStroke`/`setSkippedNoStroke` → `skippedNotCut`/`setSkippedNotCut` (`CutDialog.tsx:109`, `:194`, `:623`), while the fake's accumulator stays `skipped` (`smoke.spec.ts:279`, `:287`) with only its returned key renamed at `:310`.

The e2e fake's `planFromDoc` (`smoke.spec.ts:276-311`) is a second implementation of `plan_passes` and must be brought in step with Task 3 in this commit — it still filters on the stroke:

- `:276-277` comment: say it mirrors the `CutLineType` filter and the stroke-else-fill key.
- `:285-287`: replace the stroke read and skip test with `if (n.cut_line_type === "NoCut") { skipped++; continue; }`.
- `:289-291`: key the grouping map on stroke-else-fill with alpha-0 counting as absent, mirroring `pass_key`.

The fake's Node *literals* do not carry `cut_line_type` until Task 8, which owns the fake's
document shape — but `planFromDoc` cannot read a property the fake's `Node` type does not declare,
so the declaration comes here, optional, and Task 8 makes it required once every literal stamps
it. Add to `smoke.spec.ts:10`:

```ts
// Optional only until Task 8 stamps it on every literal: an absent value reads as cut, which
// is the import default.
type Node = { …; cut_line_type?: "Cut" | "NoCut"; … };
```

- [ ] **Step 2: Verify no occurrence survives**

Run: `grep -rn "skipped_no_stroke\|skippedNoStroke" crates apps docs CONTEXT.md CLAUDE.md`

Expected: matches only in `docs/superpowers/plans/` and `docs/superpowers/specs/` (historical records of what the field was called, which stay as written) and in `crates/trace/src/lib.rs:326`, whose whole doc comment Task 6 deletes.

- [ ] **Step 3: Run the gates**

Run: `cargo test --workspace --locked`, then `npm --prefix apps/desktop/ui test`, then `npm --prefix apps/desktop/ui run build`, then `npm --prefix apps/desktop/ui run e2e`

Expected: all PASS. The e2e suite is the one that would catch a half-renamed wire field, since the fake and `ipc.ts` are the two ends of it.

- [ ] **Step 4: Commit**

```bash
git add crates/cutplan/src/passes.rs apps/desktop/src/device.rs apps/desktop/ui/src \
        apps/desktop/ui/e2e/smoke.spec.ts apps/desktop/ui/dist
git commit -m "Call the skipped count what it counts, and teach the e2e fake the new rule

After the predicate moved, skipped_no_stroke counted shapes excluded for a
reason that is no longer about stroke. The name crosses IPC, so the DTO,
the TypeScript mirror and the fake rename with it in one commit — and the
fake's own copy of plan_passes has to filter on the attribute too."
```

---

### Task 5: A plain cut asks for one pass instead of faking one

**Files:**
- Modify: `crates/cutplan/src/passes.rs` (add `Grouping`, split `plan_passes` into a delegating pair)
- Modify: `crates/cutplan/src/lib.rs` (export `Grouping`, if the crate re-exports item by item)
- Modify: `crates/cutplan/src/plan.rs:74-79` (the comment that says a colourless pass cannot exist)
- Modify: `crates/cli/src/pipeline.rs:71-94` (delete `CUT_STROKE` and `doc_from_svg_all_cuttable`), `:155-172` (`plan_plain_cut`), `:174-178` (`describe_cut_error`'s doc comment), and the tests at `:309-327`
- Modify: `CLAUDE.md:90-95` (the paragraph describing how a plain cut reaches `plan_cut`)
- Test: `crates/cutplan/src/passes.rs`, `crates/cli/src/pipeline.rs`, `crates/cli/src/cut.rs`

**Interfaces:**
- Consumes: Task 3's `pass_key`.
- Produces: `cutplan::Grouping { ByColor, Single }` and `plan_passes_with(doc, grouping)`; `plan_passes(doc)` keeps its signature and delegates with `ByColor`, so 17 of the 18 call sites are untouched. #45 extends the enum with fill, preset and line-type modes.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/cutplan/src/passes.rs`:

```rust
    /// `Single` exists so the plain CLI cut can stop overwriting the document's colours to
    /// get one pass. Document order is the substance of it: merging colour-grouped passes
    /// afterwards would have concatenated colour by colour and quietly reordered the cut
    /// (see the spec's rejected alternative), so the order is asserted, not just the count.
    #[test]
    fn single_grouping_yields_one_pass_in_document_order() {
        let mut doc = Document::new();
        let mut ids = vec![];
        for fill in [0xFF0000FF, 0x00FF00FF, 0xFF0000FF] {
            let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
            node.style = Style { stroke: None, fill: Some(fill) };
            ids.push(node.id);
            doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));
        }

        let by_color = plan_passes_with(&doc, Grouping::ByColor).unwrap();
        assert_eq!(by_color.passes.len(), 2, "premise: two fills, so colour grouping splits");

        let single = plan_passes_with(&doc, Grouping::Single).unwrap();
        assert_eq!(single.passes.len(), 1);
        assert_eq!(single.passes[0].color, None, "one pass of mixed paint has no colour to name");
        let planned: Vec<_> = single.passes[0].shapes.iter().map(|s| s.node_id).collect();
        assert_eq!(planned, ids, "document order, not colour-grouped order");
    }
```

Replace `fill_only_svg_plans_exactly_one_pass` (`crates/cli/src/pipeline.rs:309`) and `plain_cut_plans_one_pass` (`:319`) with versions that assert the new contract through the production callers they exercise today. Reaching for `plan_passes_with` directly would leave nothing checking that `plan_plain_cut` asks for `Single` and selects the colourless pass, which is the whole of what this task changes.

The "no invented stroke" half cannot be observed by importing the same SVG a second time — that inspects a different document than the one `plan_plain_cut` built, so a production path that still stamped strokes would pass. Nor is it observable from the plain plan's pass colour, which is `None` under `Single` whatever the paint is. What *is* observable is the same fixture through the other production caller: if the import preserved the document's real colours, `--by-color` splits it into one pass per fill. So the two callers pin each other:

```rust
/// A plain cut means everything in the file in one pass, and since #144 it says so with a
/// grouping mode instead of by overwriting every path's stroke.
///
/// The second half is what proves the overwrite is gone: the same fixture through
/// `--by-color` must still see two distinct fills. When the plain path stamped a uniform
/// stroke it did so on its own document, so this pairing is the only way to observe from
/// outside that the import stopped destroying paint — `CUT_STROKE`'s deletion is checked by
/// the plan's verification grep, not by a test.
#[test]
fn plain_cut_plans_one_pass_and_by_color_still_sees_both_fills() {
    let plain = plan_plain_cut(TWO_FILLS, &driver(), &settings(), false).unwrap();
    assert_eq!(plain.passes.len(), 1);
    assert_eq!(plain.passes[0].color, None, "one pass by request names no colour");

    let by_color = plan_cut_from_svg(TWO_FILLS, &driver(), &settings(), &[], None, false).unwrap();
    assert_eq!(by_color.passes.len(), 2, "the fixture's two fills survived the import");
    assert!(by_color.passes.iter().all(|p| p.color != Some(0x000000FF)),
        "and neither pass is keyed on the stroke the plain path used to stamp");
}

/// The fill-only-clipart case, stated as behaviour rather than as a consequence: paint that
/// nobody can see still cuts, because cuttability is the attribute and import defaults it to
/// `Cut`. Before #144 this SVG planned nothing at all.
#[test]
fn plain_cut_plans_invisible_paint() {
    let plan = plan_plain_cut(TRANSPARENT_FILL, &driver(), &settings(), false).unwrap();
    assert_eq!(plan.passes.len(), 1);
    assert_eq!(plan.passes[0].color, None);
}
```

Reuse the two-fill fixture and the `driver()`/`settings()` helpers the deleted tests already had rather than adding `TWO_FILLS`; it is the exact document this needs. `TRANSPARENT_FILL` is new and small — one rect with `fill="#00ff00" fill-opacity="0"`, in bounds — and `plain_cut_of_an_empty_svg_says_nothing_to_cut` (`:343`) keeps covering the genuinely empty file.

One observable change needs pinning where nothing pins it today, and it is **not** the dry-run
header: the plain branch (`crates/cli/src/main.rs:119-127`) prints encoded bytes and no header at
all — `-- pass i/n (color …) --` (`:173`) belongs exclusively to `cut_by_color`. The only place a
plain cut ever names its pass's colour is the operator prompt, `pause_prompt`
(`crates/cli/src/cut.rs:88-97`), reached when the machine requires a per-pass confirmation. It
becomes `(color none)` where it used to read `(color #000000)`.

`pause_prompt` returns its wording precisely so it can be asserted (`:85-87`), and
`a_prompt_takes_both_halves_of_the_position_from_the_status` (`:282`) already builds plans through
a `plan(&[…])` helper that accepts a `None` colour (`:283`). Add a case beside it:

```rust
    /// A plain cut's pass has no colour to name since #144 — it is one pass by request, not one
    /// colour's worth of shapes. The prompt used to read `#000000`, which was the invented stroke
    /// the plain path stamped on every path; nothing pinned it, so nothing would catch it
    /// changing.
    #[test]
    fn a_colourless_pass_is_named_none_in_the_prompt() {
        let plan = plan(&[None]);
        let parked = status(
            Actions { cancel: true, confirm: true, ..Actions::default() },
            Phase::AwaitingConfirmation,
            Some(PassPosition { index: 0, total: 1 }),
        );
        let confirm = pause_prompt(Pause::Confirm, &plan, &parked);
        assert!(confirm.contains("(color none)"), "{confirm}");
    }
```

Check the `Phase` variant name against `driver_core::Phase` before running; the sibling test uses
`Phase::AwaitingColorSwap` for the swap case.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cutplan single_grouping` then `cargo test -p cli plain_cut_plans_one_pass`

Expected: compile errors — `cannot find function 'plan_passes_with'`, `cannot find type 'Grouping'`.

- [ ] **Step 3: Write the implementation**

In `crates/cutplan/src/passes.rs`, above `plan_passes`:

```rust
/// How `plan_passes` splits cut shapes into passes.
///
/// `ByColor` is what every caller wants and stays the default. `Single` is one pass in
/// document order, which is what `cuthulhu cut` without `--by-color` has always meant — it
/// used to say it by giving every path the same stroke, which also destroyed the document's
/// real colours for no other purpose.
///
/// #45 extends this with fill, layer-preset and line-type modes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Grouping { ByColor, Single }
```

Keep `plan_passes`' signature and doc comment where they are, and make it delegate:

```rust
pub fn plan_passes(doc: &Document) -> Result<DocumentPasses, PlanError> {
    plan_passes_with(doc, Grouping::ByColor)
}

/// `plan_passes` with the grouping named explicitly. See `Grouping`.
pub fn plan_passes_with(doc: &Document, grouping: Grouping) -> Result<DocumentPasses, PlanError> {
```

— the existing body moves into `plan_passes_with` unchanged except for the key, which becomes:

```rust
                        let color = match grouping {
                            Grouping::ByColor => pass_key(&node.style),
                            // One bucket: `None` because a pass of mixed paint has no colour
                            // to name, and the caller asked for one pass rather than for a
                            // colour's pass.
                            Grouping::Single => None,
                        };
```

Correct the comment in `crates/cutplan/src/plan.rs:74-79`, which now says something false — `plan_passes` builds a `None` pass under `Grouping::Single`, and a colourless `ColorPass` under `ByColor` for shapes with no visible paint. The `UnknownPassColor(None)` sentence itself stays accurate; only the reasoning above it changes:

```rust
            // A `None` selection names the colourless pass — the one holding shapes with no
            // visible paint, or the single pass a `Grouping::Single` plan contains. This is
            // reached when no such pass was planned at all.
```

In `crates/cli/src/pipeline.rs`, delete `CUT_STROKE` and `doc_from_svg_all_cuttable` (`:71-94`) and rewrite `plan_plain_cut`'s first half (`:160-169`):

```rust
    let doc = doc_from_svg(svg)?;
    // One pass, in document order, whatever each path is painted. Cuttability no longer
    // rides on the stroke (#144), so there is nothing to overwrite to say "cut all of
    // this" — the grouping mode says it, and the document keeps its real colours.
    let planned = cutplan::plan_passes_with(&doc, cutplan::Grouping::Single)
        .map_err(|e| e.to_string())?;
    // Checked here rather than left to `plan_cut`: with no passes at all, asking for the
    // colourless pass is an unmatched selection, and "no pass matches color" describes the
    // request instead of the file.
    if planned.passes.is_empty() {
        return Err("no cuttable paths in SVG".into());
    }
    let passes = vec![cutplan::PassSelection { color: None, settings: settings.clone() }];
```

`describe_cut_error`'s doc comment (`:174-178`) says `NothingToCut` is special-cased because "only this caller knows an SVG was imported and that none of its paths were stroked". Drop the stroke clause: none of its paths were *cut*.

Two visible consequences to accept deliberately, not to paper over:

1. The operator prompt for a plain cut that needs confirmation reads `(color none)` instead of `(color #000000)` — `format_pass_color` (`crates/cli/src/cut.rs:45-50`) already renders `None` as `none`. That is the only colour a plain cut ever prints: its `--dry-run` branch emits bytes only. The old value was an invented stroke, so printing it was the lie, and `a_colourless_pass_is_named_none_in_the_prompt` pins the new one.
2. A plain cut over an SVG whose paths are all invisible-painted still plans one pass, because cuttability is the attribute and import defaults it to `Cut`. That is the intended behaviour — the fill-only-clipart case — and `plain_cut_plans_invisible_paint` above pins it through the production caller, while `plain_cut_of_an_empty_svg_says_nothing_to_cut` (`:343`) keeps covering the genuinely empty file.

Update `CLAUDE.md:90-95`, whose paragraph describes the overwrite as the plain path's mechanism, to describe `Grouping::Single` instead, and drop the sentence deferring to #68 — #68 is decided and #144 implemented it.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS, including the five plain-cut tests the scouted map flagged (`crates/cli/src/pipeline.rs:329`, `:343`; `crates/cli/tests/plain_cut.rs:168`, `:193`, `:232`) — each cuts a single-shape fixture, so one pass is still one pass — and `crates/cli/tests/dry_run.rs:9`, `:26`.

- [ ] **Step 5: Commit**

```bash
git add crates/cutplan/src/passes.rs crates/cutplan/src/plan.rs crates/cli/src/pipeline.rs \
        crates/cli/src/cut.rs CLAUDE.md
git commit -m "Ask the planner for one pass, instead of overwriting every stroke to get one

The plain cut's uniform stroke did two jobs: made geometry cuttable and
collapsed it into one bucket. #144 took the first job away, which left the
overwrite destroying the document's real colours for nothing — and worse,
with colours preserved it would have planned a pass per fill. Grouping is
a planning input now, which is the direction #45 already prescribes."
```

---

### Task 6: Trace stops mirroring fill onto stroke

**Files:**
- Modify: `crates/trace/src/lib.rs` — delete `mirror_fill_onto_stroke` (`:322-349`) and `attr_value` (`:315-320`), and the call at `:304`
- Modify: `crates/trace/src/lib.rs:737` — delete `traced_paths_are_stroked_so_they_can_be_cut`, replace with its opposite
- Modify: `crates/trace/tests/roundtrip.rs:18-42` — the stroke assertions (`:26-40`) become fill assertions
- Modify: seven stroke-premise comments in `crates/trace/src/lib.rs`, listed in Step 3 — each explains a real danger with the mirror as its mechanism

**Interfaces:**
- Consumes: Task 3 (fill-only geometry plans passes) and Task 1 (import defaults to `Cut`). Deleting this before either is in place makes every trace uncuttable.
- Produces: no public change. `trace`'s exported surface is untouched; only `TraceResult::svg`'s content changes, and `path_count` is counted before the mirror ever ran.

- [ ] **Step 1: Write the failing test**

Replace `traced_paths_are_stroked_so_they_can_be_cut` (`crates/trace/src/lib.rs:732-757`, doc comment included) with:

```rust
    /// vtracer describes a region by the colour that fills it, and since #144 that is
    /// enough: an imported path is cuttable because its `CutLineType` says so, and its pass
    /// is keyed on the fill. The mirror that used to copy fill onto stroke was a workaround
    /// for the old rule and double-applies under the new one, so a traced path must carry
    /// no stroke at all — an invented one would key a pass on a colour the user never chose.
    #[test]
    fn traced_paths_carry_a_fill_and_no_stroke() {
        for mode in [TraceMode::Binary, TraceMode::Color] {
            let opts = TraceControls { mode, speckle: 0, ..TraceControls::default() };
            let r = trace(&png_bytes(&quadrants()), &opts).unwrap();
            let paths: Vec<&str> =
                r.svg.lines().filter(|l| l.trim_start().starts_with("<path")).collect();
            assert!(!paths.is_empty(), "{mode:?}: no paths emitted");
            for p in paths {
                assert!(p.contains("fill=\""), "{mode:?}: path carries no fill: {p}");
                assert!(!p.contains("stroke=\""), "{mode:?}: path still carries a stroke: {p}");
            }
        }
    }
```

The local `attr` helper the old test defined goes with it; nothing else uses it.

In `crates/trace/tests/roundtrip.rs`, replace the stroke block (`:26-40`) with:

```rust
        // Importing cleanly is enough now. `cutplan` cuts a shape because its `CutLineType`
        // says `Cut` — which is what import defaults to — and keys its pass on the fill when
        // there is no stroke, so fill-only trace output plans a pass per traced colour. The
        // planning half of that contract is pinned in `cutplan`
        // (`a_fill_only_shape_that_is_cut_plans_into_a_pass_keyed_on_its_fill`); what this
        // test owns is that the trace really does arrive as fill-only geometry.
        for (i, (_, hint)) in imp.paths.iter().enumerate() {
            assert_eq!(hint.stroke, None, "{mode:?}: imported path {i} carries an invented stroke");
            let fill = hint.fill.unwrap_or_else(|| {
                panic!("{mode:?}: imported path {i} has no fill, so nothing keys its pass")
            });
            assert!(fill & 0xFF != 0,
                "{mode:?}: imported path {i} has a fully transparent fill ({fill:#010x})");
        }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace`

Expected: both new tests fail — the mirror is still stamping `stroke="…"` on every path.

- [ ] **Step 3: Write the implementation**

Delete `mirror_fill_onto_stroke` and `attr_value` from `crates/trace/src/lib.rs`, and at the call site (`:304`) inside `strip_empty_paths`, push the line through unchanged:

```rust
            out.push_str(line);
```

Nothing else in the crate calls either function. `strip_empty_paths`' own doc comment must lose any claim about stroking; check it before committing.

**Seven** comments elsewhere in the crate reach a correct conclusion through the mirror, and must
keep the conclusion while losing the mechanism — the danger each documents is real and survives
this change, which is exactly why none can be left saying something false. The check that none was
missed is `grep -n "is stroked\|are stroked\|being stroked\|strokes" crates/trace/src/lib.rs`
returning nothing — not a search for `strok`, which the replacement test legitimately contains in
`assert!(!p.contains("stroke=\""), …)`:

- `flatten_onto_white`'s doc comment (`crates/trace/src/lib.rs:367-374`) ends "Because every
  emitted path is stroked, that rectangle is a cut line, so an invisible image would put a
  rectangle through the material." Replace the causal clause: a traced path is cut because
  import defaults its `CutLineType` to `Cut`, not because anything stroked it.
- `trace`'s own pre-flatten comment (`:411-414`) says colour mode "clusters that white into a path
  of its own, which `strip_empty_paths` strokes" — after this change `strip_empty_paths` strokes
  nothing; the white path is cut because it is a path.
- `trace`'s binary-mode comment (`:420-424`) says "the manufactured white would come back as a
  stroked path the cut planner reports as a pass". Filled, and keyed on that fill.
- The `should_key_image` comment (`:392-396`) says a `(0,0,0,0)` island "traces to a black shape
  and, being stroked, cuts". Same correction.
- `a_fully_transparent_image_traces_to_nothing` (`:760-765`) says "Since every traced path is now
  stroked, that phantom shape is cut geometry" and closes on "a stroked, cuttable canvas
  rectangle".
- `a_transparent_background_is_not_a_path_in_color_mode` (`:780-783`) says the flattened
  background "comes back as a stroked path, which the cut planner then reports as a pass". It
  comes back as a filled path.
- `a_transparent_island_is_keyed_out_below_the_keying_threshold` (`:800-805`) says the island
  "becomes a stroked black shape that cuts".

Only the two tests this task rewrites change their assertions — `traced_paths_carry_a_fill_and_no_stroke`
and the trace round trip, both of which invert a stroke assertion into a fill one. The five
comments above them are comments: the tests they sit on keep asserting exactly what they assert
today, all of it about `path_count`, emitted colours, or `TraceError::EmptyResult`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. `strip_empty_paths_counts_only_real_geometry` (`crates/trace/src/lib.rs:867`) passes unchanged — it only ever asserted on `d`. `crates/cli/tests/trace.rs`' five tests never read `stroke`. `fileio`'s `write_then_import_round_trips_stroke_and_fill` (`crates/fileio/src/lib.rs:276`) already covers the fill-only round trip, so `design.svg` needs no change: `paint_attrs` writes `stroke="none"` for absent paint, which imports back as `None`.

- [ ] **Step 5: Commit**

```bash
git add crates/trace/src/lib.rs crates/trace/tests/roundtrip.rs
git commit -m "Stop mirroring a traced fill onto a stroke, since the rule it worked around is gone

Its own doc comment stated the premise #144 removed: that fill-only meant
do-not-cut downstream, so the colour had to be promoted where it was known
to describe cuttable geometry. Cuttability is explicit now, so the mirror
double-applies and invents a stroke colour the user never chose."
```

---

### Task 7: The document can be told what to cut

**Files:**
- Modify: `crates/document/src/commands.rs` (add `set_cut_line_type` after `transform_nodes` at `:43`)
- Modify: `apps/desktop/src/state.rs` (add a method after `reorder` at `:64`)
- Modify: `apps/desktop/src/ipc.rs` (add a command after `reorder` at `:51`)
- Modify: `apps/desktop/src/main.rs:47-91` (register it)
- Test: `crates/document/src/commands.rs` (in `mod tests`)

**Interfaces:**
- Consumes: `CutLineType` (Task 1).
- Produces: `commands::set_cut_line_type(doc, ids, value) -> Result<Delta, CmdError>`, `AppState::set_cut_line_type`, and the `set_cut_line_type` Tauri command. Task 8 calls it from the properties panel.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/document/src/commands.rs`:

```rust
    /// Selecting a container and marking it `NoCut` has to reach the shapes, because the
    /// attribute does not inherit: `plan_passes` reads it only on shapes, so setting it on
    /// a Group alone would be a control that visibly does nothing.
    #[test]
    fn setting_a_cut_line_type_reaches_the_shapes_under_a_container() {
        let mut doc = Document::new();
        let group = Node::container(doc.ids.next(), NodeKind::Group);
        let group_id = group.id;
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![
            NodeOp::Add { parent: doc.root, node: group, index: usize::MAX },
            NodeOp::Add { parent: group_id, node: shape, index: usize::MAX },
        ]));

        let d = set_cut_line_type(&doc, &[group_id], CutLineType::NoCut).unwrap();
        doc.apply(d);
        assert_eq!(doc.get(shape_id).unwrap().cut_line_type, CutLineType::NoCut);
    }

    /// A selection that already has the value produces no ops, so it cannot land an undo
    /// step that undoes nothing — the panel dispatches on every click, including the one
    /// that re-picks what is already set.
    #[test]
    fn setting_the_value_a_node_already_has_produces_no_ops() {
        let mut doc = Document::new();
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));
        assert_eq!(doc.get(shape_id).unwrap().cut_line_type, CutLineType::Cut, "premise");

        let d = set_cut_line_type(&doc, &[shape_id], CutLineType::Cut).unwrap();
        assert_eq!(d, Delta(vec![]));
    }

    /// Overlapping selections are one edit per shape, and an empty selection is refused
    /// the same way every other command refuses it.
    #[test]
    fn a_shape_selected_twice_over_is_updated_once_and_nothing_is_refused_twice() {
        let mut doc = Document::new();
        let group = Node::container(doc.ids.next(), NodeKind::Group);
        let group_id = group.id;
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![
            NodeOp::Add { parent: doc.root, node: group, index: usize::MAX },
            NodeOp::Add { parent: group_id, node: shape, index: usize::MAX },
        ]));

        let d = set_cut_line_type(&doc, &[group_id, shape_id], CutLineType::NoCut).unwrap();
        assert_eq!(d.0.len(), 1, "the shape is reached twice and updated once");
        assert_eq!(set_cut_line_type(&doc, &[], CutLineType::NoCut), Err(CmdError::EmptySelection));
        assert_eq!(set_cut_line_type(&doc, &[NodeId(9999)], CutLineType::NoCut), Err(CmdError::NotFound));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p document commands::tests::setting`

Expected: compile error — `cannot find function 'set_cut_line_type'`.

- [ ] **Step 3: Write the implementation**

Insert after `transform_nodes` in `crates/document/src/commands.rs`:

```rust
/// Mark every shape in `ids` — and every shape beneath a container in `ids` — with `value`.
///
/// Descends where `transform_nodes` suppresses: a transform is inherited through the tree,
/// so applying it to a node *and* its selected ancestor would move it twice, but a
/// `CutLineType` is read only on the shape that carries it (`cutplan::plan_passes`). Setting
/// it on a Group alone would be a control that visibly does nothing, so the container's
/// selection means its shapes.
///
/// Unchanged shapes emit no op, so re-picking the value a selection already has cannot land
/// an undo step that undoes nothing.
pub fn set_cut_line_type(doc: &Document, ids: &[NodeId], value: CutLineType)
    -> Result<Delta, CmdError> {
    if ids.is_empty() { return Err(CmdError::EmptySelection); }
    let mut ops = vec![];
    let mut seen = HashSet::new();
    let mut stack: Vec<NodeId> = ids.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let node = doc.get(id).ok_or(CmdError::NotFound)?;
        // Also the cycle guard: a malformed document whose nodes contain each other would
        // otherwise spin here, the way `plan_passes` guards its own walk.
        if !seen.insert(id) { continue; }
        match &node.kind {
            NodeKind::Group | NodeKind::Layer => stack.extend(node.children.iter().rev().copied()),
            NodeKind::Shape(_) => {
                if node.cut_line_type == value { continue; }
                let before = node.clone();
                let mut after = before.clone();
                after.cut_line_type = value;
                ops.push(NodeOp::Update { id, before, after });
            }
        }
    }
    Ok(Delta(ops))
}
```

`NodeOp::Update` carries whole `Node`s, so the inverse comes free from `Document::apply` (`crates/document/src/delta.rs:70-72`) and no new `NodeOp` variant is needed.

In `apps/desktop/src/state.rs`, after `reorder` (`:64`):

```rust
    pub fn set_cut_line_type(&mut self, ids: Vec<NodeId>, value: CutLineType)
        -> Result<Delta, CmdError> {
        let d = commands::set_cut_line_type(&self.editor.doc, &ids, value)?;
        // An empty delta is a no-op the operator asked for; committing it would clear the
        // redo stack and add an undo step that does nothing.
        if d.0.is_empty() { return Ok(d); }
        Ok(self.editor.commit(d))
    }
```

Add `CutLineType` to the `document::{…}` import at `state.rs:3`, and in `apps/desktop/src/ipc.rs` after `reorder` (`:51`):

```rust
#[tauri::command]
pub fn set_cut_line_type(state: tauri::State<AppStateHandle>, ids: Vec<NodeId>, value: CutLineType)
    -> Result<Delta, String> {
    state.lock().unwrap().set_cut_line_type(ids, value).map_err(|e| format!("{e:?}"))
}
```

with `CutLineType` added to `ipc.rs:4`'s import. Register it in `apps/desktop/src/main.rs`'s `generate_handler!` list, next to `ipc::reorder`.

The `format!("{e:?}")` matches its neighbours and is deliberately not fixed here: issue #93 owns giving `CmdError` a `Display`, and fixing one call site while nine others print `Debug` would leave the operator with two conventions.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/document/src/commands.rs apps/desktop/src/state.rs apps/desktop/src/ipc.rs \
        apps/desktop/src/main.rs
git commit -m "Let a selection say whether it is cut, down to the shapes under a container

Descends where transform_nodes suppresses: a transform is inherited, so
applying it to a node and its selected ancestor moves it twice, but a
CutLineType is read only on the shape carrying it — setting it on a Group
alone would be a control that visibly does nothing."
```

---

### Task 8: The operator can see and set it

**Files:**
- Create: `apps/desktop/ui/src/panels/cutLineType.ts`, `apps/desktop/ui/src/panels/cutLineType.test.ts`
- Modify: `apps/desktop/ui/src/panels/PropertiesPanel.tsx` (the control)
- Modify: `apps/desktop/ui/src/App.tsx:38-43` (`DocNode` gains the field), `:458-464` (wire the panel)
- Modify: `apps/desktop/ui/src/ipc.ts` (a wrapper next to `reorder` at `:37`)
- Modify: `apps/desktop/ui/src/cut/CutDialog.tsx:623` (say why)
- Modify: `apps/desktop/ui/e2e/smoke.spec.ts:9-10`, `:23-25`, `:31`, `:49`, `:57`, `:73-78`, `:88`, `:117` (the fake's Nodes carry the attribute)
- Build: `apps/desktop/ui/dist`

**Interfaces:**
- Consumes: `set_cut_line_type` (Task 7), `skipped_not_cut` (Task 4).
- Produces: the operator-facing surface, and nothing richer — grouping-mode pickers and per-layer roles are #45's (spec scoping decision 2).

- [ ] **Step 1: Write the failing test**

Create `apps/desktop/ui/src/panels/cutLineType.test.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import type { DocNode } from "../App";
import { selectionCutLineType } from "./cutLineType";

// Annotated `DocNode` rather than inferred: `tsconfig.json` compiles `src/**/*.test.ts` under
// `strict`, and an inferred `as const` transform is a *readonly* tuple, which `Affine6`
// (`render/hittest.ts`) is not — the assignment fails to typecheck and takes `npm run build`
// with it.
const shape = (id: number, cut: "Cut" | "NoCut"): DocNode => ({
  id,
  kind: { Shape: { Rect: { w: 1, h: 1 } } },
  transform: [1, 0, 0, 1, 0, 0],
  children: [],
  cut_line_type: cut,
});
const group = (id: number, children: number[]): DocNode => ({
  id,
  kind: "Group",
  transform: [1, 0, 0, 1, 0, 0],
  children,
  cut_line_type: "Cut",
});

describe("selectionCutLineType", () => {
  it("is null with nothing selected, so the panel can hide the control", () => {
    expect(selectionCutLineType({ 1: shape(1, "Cut") }, [])).toBeNull();
  });

  it("reads the shapes under a selected container, since the attribute does not inherit", () => {
    const nodes = { 1: group(1, [2]), 2: shape(2, "NoCut") };
    expect(selectionCutLineType(nodes, [1])).toBe("NoCut");
  });

  it("is mixed when the selection disagrees, so neither value is shown as the truth", () => {
    const nodes = { 1: shape(1, "Cut"), 2: shape(2, "NoCut") };
    expect(selectionCutLineType(nodes, [1, 2])).toBe("mixed");
  });

  it("is null for a selection with no shapes under it at all", () => {
    expect(selectionCutLineType({ 1: group(1, []) }, [1])).toBeNull();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm --prefix apps/desktop/ui test -- cutLineType`

Expected: failure — cannot resolve `./cutLineType`.

- [ ] **Step 3: Write the implementation**

Create `apps/desktop/ui/src/panels/cutLineType.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { DocNode } from "../App";

export type CutLineTypeJson = "Cut" | "NoCut";

/// What to show for a selection: one value, "mixed" when the shapes disagree, or null when
/// there is no shape to speak for. Mirrors `commands::set_cut_line_type`, which walks into
/// containers because the attribute is read only on shapes — a panel that read the selected
/// node's own value would show a Group's inert one.
export function selectionCutLineType(
  nodes: Record<string, DocNode>,
  selected: number[],
): CutLineTypeJson | "mixed" | null {
  const values = new Set<CutLineTypeJson>();
  const seen = new Set<number>();
  const stack = [...selected];
  while (stack.length > 0) {
    const id = stack.pop()!;
    if (seen.has(id)) continue;
    seen.add(id);
    const node = nodes[String(id)];
    if (!node) continue;
    if (typeof node.kind === "object" && "Shape" in node.kind) values.add(node.cut_line_type);
    else stack.push(...node.children);
  }
  if (values.size === 0) return null;
  return values.size === 1 ? [...values][0] : "mixed";
}
```

The type-only import from `../App` is erased at compile time, so it introduces no import cycle.

Add the field to `DocNode` (`apps/desktop/ui/src/App.tsx:38-43`):

```ts
export type DocNode = {
  id: number;
  kind: NodeKindJson;
  transform: Affine6;
  cut_line_type: CutLineTypeJson;
  children: number[];
};
```

In `App.tsx`, add the import beside the other `./panels` imports —

```tsx
import { selectionCutLineType, type CutLineTypeJson } from "./panels/cutLineType";
```

`CutLineTypeJson` is defined once, in `panels/cutLineType.ts`, and imported from there by both
`App.tsx` and `PropertiesPanel.tsx` — no re-export. Then derive the value and dispatch the edit
next to `commitAxis`/`commitScale`:

```tsx
  const cutLineType = doc ? selectionCutLineType(doc.nodes, selected) : null;

  const setCutLineType = (value: CutLineTypeJson) => {
    if (selected.length === 0) return;
    run(() => ipc.setCutLineType({ ids: selected, value }));
  };
```

and pass both to the panel (`:458-464`), adding `cutLineType={cutLineType}` and `onChangeCutLineType={setCutLineType}`.

In `PropertiesPanel.tsx`, add `import type { CutLineTypeJson } from "./cutLineType";` beside the
existing `Bounds`/`NumberField` imports, extend `Props` with
`cutLineType: CutLineTypeJson | "mixed" | null` and `onChangeCutLineType: (v: CutLineTypeJson) => void`,
**and add both to the component's parameter destructuring** (`PropertiesPanel.tsx:13`), which names
every prop explicitly — extending the type alone leaves the two identifiers undefined in the body
and fails strict `tsc`:

```tsx
export function PropertiesPanel({ bounds, cutLineType, onChangeX, onChangeY, onChangeW, onChangeH,
                                  onChangeCutLineType }: Props) {
```

Render the control below the four `NumberField`s. It cannot live inside the `bounds ?` branch:
`selectedBounds` is null for every multi-node selection and for a selected container
(`App.tsx:326`), both of which do have a cuttability.

That also means the panel's `No selection` fallback (`PropertiesPanel.tsx:24-26`) stops being
true as written — it currently reads "no bounds" as "nothing selected", and would print it above a
live Cut control. Guard it on both:

```tsx
      {bounds ? (
        <>…the four NumberFields, unchanged…</>
      ) : null}
      {cutLineType !== null ? (
        <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
          <input
            type="checkbox"
            aria-label="Cut this shape"
            checked={cutLineType === "Cut"}
            // A mixed selection shows the browser's indeterminate mark rather than picking a
            // side; clicking it commits `Cut` for everything, which is the recoverable
            // direction (one undo, or one more click).
            ref={(el) => { if (el) el.indeterminate = cutLineType === "mixed"; }}
            onChange={(e) => onChangeCutLineType(e.target.checked ? "Cut" : "NoCut")}
          />
          Cut
        </label>
      ) : null}
      {bounds === null && cutLineType === null ? (
        <div style={{ fontSize: 12, color: "var(--muted)" }}>No selection</div>
      ) : null}
```

Add the wrapper in `apps/desktop/ui/src/ipc.ts`, next to `reorder`:

```ts
export async function setCutLineType(args: Args) {
  return invoke("set_cut_line_type", args);
}
```

Say why in the cut dialog (`CutDialog.tsx:623`) — spec scoping decision 2 asks the readout to name the reason, and after Task 3 "no stroke" is not it:

```tsx
        <div style={{ fontSize: 12, color: "var(--muted)" }}>
          Not cut: {skippedNotCut} shapes marked No Cut
        </div>
```

Then the e2e fake. It mirrors `Document::snapshot_json()`, so every fabricated Node gains the field:

- `:10` — make `cut_line_type` required on the fake's `Node` type by dropping the `?` Task 4 added, so a literal that forgets it fails to compile rather than silently reading as cut.
- `:23-25` — `DEFAULT_STYLE`'s comment currently claims a black stroke is what makes a shape "cuttable by default". Move that clause to the new field: paint no longer decides.
- `:31`, `:49`, `:57`, `:88`, `:117` — every Node literal gets `cut_line_type: "Cut"`, including the root Layer (inert, and matching `Node::container`).
- `:73-78` — `add_primitive`'s fake already accepts a test-only `a.stroke` override; add a `a.cut_line_type` override the same way, so a test can seed a `NoCut` node.
- `planFromDoc` (`:276-311`) was already brought in step in Task 4; re-read it here and confirm it reads the literal field rather than relying on `undefined`.

Add one e2e test asserting the round trip through the real command — the fake's `set_cut_line_type` must apply the value to the shapes under the selection, and the cut dialog's readout must then count it. Place it beside the existing cut-dialog tests, and register the command in the fake's `invoke` switch.

- [ ] **Step 4: Run the tests to verify they pass**

Run, in order:

```sh
npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run build
npm --prefix apps/desktop/ui run e2e
cargo test --workspace --locked
```

Expected: all PASS. `npm run build` must be run *before* `e2e` and its `dist/` output committed, or CI fails on a stale bundle.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src apps/desktop/ui/e2e/smoke.spec.ts apps/desktop/ui/dist
git commit -m "Give the operator one control for cuttability, and say why a shape was not cut

The panel reads the shapes under the selection rather than the selected
node's own value, because a container's attribute is inert — the same
reason set_cut_line_type descends. A mixed selection shows indeterminate
instead of picking a side."
```

---

## Verification (whole plan)

After Task 8, confirm the change did what it claims rather than assuming it:

- [ ] `cargo test --workspace --locked` passes, and `git status` shows **no** change to `Cargo.lock`.
- [ ] `npm --prefix apps/desktop/ui test`, `npm --prefix apps/desktop/ui run e2e` pass, and `npm --prefix apps/desktop/ui run build` leaves `dist/` unchanged (a dirty `dist/` after a fresh build means a commit shipped a stale bundle).
- [ ] `grep -rn "skipped_no_stroke" crates apps` returns nothing.
- [ ] `grep -rn "mirror_fill_onto_stroke\|CUT_STROKE\|doc_from_svg_all_cuttable" crates apps` returns nothing outside `docs/`.
- [ ] `grep -n "cut_line_type" crates/cutplan/src/passes.rs` shows exactly one read — the predicate. Cuttability is decided in one place.
- [ ] The three #139 ordering tests (`crates/cutplan/src/passes.rs`, names in Task 3) are unmodified in `git diff main...HEAD` apart from fixture marking: `git diff main...HEAD -- crates/cutplan/src/passes.rs | grep -c "^-.*shape_outline"` is 0.
- [ ] An end-to-end trace still reaches a plan: `cargo run -p cli -- trace <png> -o /tmp/t.svg && cargo run -p cli -- cut /tmp/t.svg --dry-run` prints one pass with a non-zero byte count. This is the whole chain Task 6 depends on — fill-only output, default `Cut`, fill-keyed grouping — and the case the deleted mirror existed to fake.
- [ ] A project file written before this change still cuts what it cut: covered by `a_project_saved_before_cuttability_derives_it_from_stroke`, and worth confirming by hand on any `.cut` file predating the branch if one exists locally.
- [ ] `docs/superpowers/specs/2026-08-14-cuttability-attribute-design.md` needs no correction. If implementation diverged from it — a different migration mechanism, a different grouping default — amend the spec in the final commit rather than leaving the two out of step.
- [ ] Hardware verification is **not** required by this change and nothing is added to `apps/desktop/MANUAL-CHECKLIST.md`: every behaviour here is decided before any byte reaches a Transport, and the `MockTransport`/dry-run paths cover it. A real cut with a `NoCut` shape in the document is worth doing opportunistically, not blocking on.
