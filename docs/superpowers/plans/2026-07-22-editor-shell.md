# Editor Shell (SP3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Blocked on SP2.** This plan assumes sub-project 2 (drivers + CLI) has created the Rust workspace and the `geometry` + `fileio` + `driver-core` crates. Do not start until SP2 is merged. See "Assumed SP2 interfaces" below — if SP2 shipped different signatures, reconcile there first.

**Goal:** A fast vector editor that opens/imports SVG, places and arranges shapes and text on the machine's real cuttable area, and saves to the open project container — proving cuthulhu's UX/performance thesis with no cutting UI yet.

**Architecture:** Hybrid model. A new Rust `document` crate owns the authoritative scene tree, commands, and undo/redo, mutating state exclusively through invertible **Deltas**. A Tauri app exposes IPC commands over it. A React+TS UI mirrors the document as a Canvas2D scene-graph, applies transforms optimistically during a drag, and commits on mouse-up. Everything testable lives in Rust or in pure TS logic.

**Tech Stack:** Rust (`document` crate; `serde`/`serde_json`), Tauri 2, TypeScript + React + Vite, Vitest (TS unit tests), a Canvas2D renderer. Rust tests via `cargo test`; UI logic via `vitest`.

## Global Constraints

- **License:** every new file is GPL-3.0-or-later (`# SPDX-License-Identifier: GPL-3.0-or-later` for scripts; `// SPDX-License-Identifier: GPL-3.0-or-later` top of each `.rs`/`.ts`/`.tsx`).
- **No AI attribution** in commits or code.
- **Import scope V1: SVG only.** No PDF/DXF.
- **No cutting in SP3.** The `cut` design token exists but is unused; no device IO.
- **Units:** the document stores geometry in **millimetres** (f64). Device-unit conversion is the drivers' job (SP2/SP4), never the editor's.
- **Design tokens (Workbench):** `workspace #17171A`, `panel #1F1F23`, `border #2E2E34`, `text #E7E7EA`, `muted #9A9AA2`, `accent #22D3EE`, `cut #FF4D4D` (reserved), `ready #34D399`. Cyan = selection; red = cut (unused here).
- **No silent failures:** IPC commands return typed `Result`; failures surface to the user and mutate nothing.

## Assumed SP2 interfaces

These are the exact signatures SP3 depends on. SP2 must provide them; this plan calls them as defined here.

```rust
// crate: geometry
pub struct Affine([f64; 6]);                 // 2D affine, row-major a b c d e f
impl Affine {
    pub fn identity() -> Affine;
    pub fn translate(dx: f64, dy: f64) -> Affine;
    pub fn then(&self, other: &Affine) -> Affine;
    pub fn inverse(&self) -> Affine;
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64);
}
pub struct Path;                              // sequence of sub-paths of Bézier/line segs, in mm
impl Path {
    pub fn bounds(&self) -> Rect;             // axis-aligned bounds
    pub fn transformed(&self, m: &Affine) -> Path;
}
pub struct Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }

pub enum BoolOp { Union, Subtract, Intersect, Exclude }
pub fn boolean(op: BoolOp, paths: &[Path]) -> Result<Path, GeomError>;
pub fn text_to_path(family: &str, size_mm: f64, text: &str) -> Result<Path, GeomError>;
pub fn rect_path(x: f64, y: f64, w: f64, h: f64) -> Path;
pub fn ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64) -> Path;
pub enum GeomError { Degenerate, NoFont, Other(String) }

// crate: fileio
pub fn svg_to_paths(svg_bytes: &[u8]) -> Result<SvgImport, IoError>;
pub struct SvgImport { pub paths: Vec<(Path, StyleHint)>, pub skipped: Vec<String> }
pub struct StyleHint { pub stroke: Option<u32>, pub fill: Option<u32> } // 0xRRGGBBAA
pub enum IoError { Parse(String), Io(String) }
```

If SP2's names differ, add thin adapter fns in `document` rather than editing every task.

---

### Task 1: `document` crate skeleton + core types

**Files:**
- Create: `crates/document/Cargo.toml`
- Create: `crates/document/src/lib.rs`
- Create: `crates/document/src/node.rs`

**Interfaces:**
- Consumes: `geometry::{Affine, Path, Rect, rect_path, ellipse_path}`.
- Produces: `NodeId`, `Style`, `ShapeKind`, `Node`, `NodeKind` used by every later task.

- [ ] **Step 1: Write the failing test**

`crates/document/src/node.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_shape_has_identity_transform_and_unique_ids() {
        let mut ids = IdGen::default();
        let a = Node::shape(ids.next(), ShapeKind::Rect { w: 10.0, h: 5.0 });
        let b = Node::shape(ids.next(), ShapeKind::Rect { w: 10.0, h: 5.0 });
        assert_ne!(a.id, b.id);
        assert_eq!(a.transform, geometry::Affine::identity());
        assert!(matches!(a.kind, NodeKind::Shape(_)));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p document node::`
Expected: FAIL — `document` crate / types not found.

- [ ] **Step 3: Write minimal implementation**

`crates/document/Cargo.toml`:
```toml
[package]
name = "document"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-or-later"

[dependencies]
geometry = { path = "../geometry" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`crates/document/src/node.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use geometry::{Affine, ShapeKind as _};
use serde::{Serialize, Deserialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct NodeId(pub u64);

#[derive(Default)]
pub struct IdGen(u64);
impl IdGen {
    pub fn next(&mut self) -> NodeId { self.0 += 1; NodeId(self.0) }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Style { pub stroke: Option<u32>, pub fill: Option<u32> } // 0xRRGGBBAA
impl Default for Style {
    fn default() -> Self { Style { stroke: Some(0x000000FF), fill: None } }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum ShapeKind {
    Rect { w: f64, h: f64 },
    Ellipse { rx: f64, ry: f64 },
    Text { family: String, size_mm: f64, text: String },
    Path { /* serialized outline in mm */ d: String },
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum NodeKind { Shape(ShapeKind), Group, Layer }

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub transform: Affine,   // relative to parent
    pub style: Style,
    pub children: Vec<NodeId>,
}
impl Node {
    pub fn shape(id: NodeId, kind: ShapeKind) -> Node {
        Node { id, kind: NodeKind::Shape(kind), transform: Affine::identity(),
               style: Style::default(), children: vec![] }
    }
    pub fn container(id: NodeId, kind: NodeKind) -> Node {
        Node { id, kind, transform: Affine::identity(),
               style: Style::default(), children: vec![] }
    }
}
```

`crates/document/src/lib.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
pub mod node;
pub use node::*;
```

(Note: `Affine` must derive `Serialize, Deserialize, PartialEq` in SP2's geometry crate. If not, that's the one adapter to add there.)

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p document node::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/document/
git commit -m "Add document crate skeleton with core node types"
```

---

### Task 2: Delta model (the single mutation + invert mechanism)

**Files:**
- Create: `crates/document/src/delta.rs`
- Modify: `crates/document/src/lib.rs` (add `pub mod delta;`)

**Interfaces:**
- Consumes: `Node`, `NodeId` (Task 1).
- Produces: `NodeOp`, `Delta`, and `Document::apply(&mut self, Delta) -> Delta` (returns the inverse). This is how ALL later mutations happen.

- [ ] **Step 1: Write the failing test**

`crates/document/src/delta.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::*;

    #[test]
    fn apply_then_apply_inverse_restores_state() {
        let mut doc = Document::new();
        let root = doc.root;
        let id = doc.ids.next();
        let node = Node::shape(id, ShapeKind::Rect { w: 4.0, h: 2.0 });

        let before = doc.clone();
        let add = Delta(vec![NodeOp::Add { parent: root, node, index: 0 }]);
        let inverse = doc.apply(add);
        assert!(doc.get(id).is_some());

        doc.apply(inverse);            // undo
        assert!(doc.get(id).is_none());
        assert_eq!(doc, before);       // full-state equality
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p document delta::`
Expected: FAIL — `Document`, `Delta`, `NodeOp` not found.

- [ ] **Step 3: Write minimal implementation**

`crates/document/src/delta.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::node::*;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum NodeOp {
    Add { parent: NodeId, node: Node, index: usize },
    Remove { parent: NodeId, id: NodeId },            // inverse carries the node via Add
    Update { id: NodeId, before: Node, after: Node }, // full-node swap → trivially invertible
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
pub struct Delta(pub Vec<NodeOp>);

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Document {
    pub nodes: HashMap<NodeId, Node>,
    pub root: NodeId,
    pub ids: IdGen,
}

impl Document {
    pub fn new() -> Document {
        let mut ids = IdGen::default();
        let root = ids.next();
        let mut nodes = HashMap::new();
        nodes.insert(root, Node::container(root, NodeKind::Layer));
        Document { nodes, root, ids }
    }
    pub fn get(&self, id: NodeId) -> Option<&Node> { self.nodes.get(&id) }

    /// Apply a delta, mutating the document. Returns the inverse delta.
    pub fn apply(&mut self, d: Delta) -> Delta {
        let mut inv = Vec::with_capacity(d.0.len());
        for op in d.0 {
            match op {
                NodeOp::Add { parent, node, index } => {
                    let id = node.id;
                    self.nodes.insert(id, node);
                    let p = self.nodes.get_mut(&parent).expect("parent exists");
                    let idx = index.min(p.children.len());
                    p.children.insert(idx, id);
                    inv.push(NodeOp::Remove { parent, id });
                }
                NodeOp::Remove { parent, id } => {
                    let node = self.nodes.remove(&id).expect("node exists");
                    let p = self.nodes.get_mut(&parent).expect("parent exists");
                    let index = p.children.iter().position(|c| *c == id).expect("child");
                    p.children.remove(index);
                    inv.push(NodeOp::Add { parent, node, index });
                }
                NodeOp::Update { id, before, after } => {
                    self.nodes.insert(id, after.clone());
                    inv.push(NodeOp::Update { id, before: after, after: before });
                }
            }
        }
        inv.reverse();          // inverse ops apply in reverse order
        Delta(inv)
    }
}
```

Add `IdGen: Clone` in `node.rs` (`#[derive(Default, Clone, PartialEq, Debug, Serialize, Deserialize)]` on `IdGen`, and derive the same on `Node`/`Style`/etc. already done). Add `pub mod delta;` + `pub use delta::*;` to `lib.rs`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p document delta::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/document/
git commit -m "Add invertible Delta model and Document.apply"
```

---

### Task 3: History (undo/redo) on top of Deltas

**Files:**
- Create: `crates/document/src/history.rs`
- Modify: `crates/document/src/lib.rs` (`pub mod history;`)

**Interfaces:**
- Consumes: `Document`, `Delta` (Task 2).
- Produces: `Editor` wrapping a `Document` with `commit(&mut self, Delta) -> Delta`, `undo(&mut self) -> Option<Delta>`, `redo(&mut self) -> Option<Delta>`. Each returns the **forward** delta the UI should render.

- [ ] **Step 1: Write the failing test**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::*;
    use crate::delta::*;

    #[test]
    fn undo_then_redo_round_trips() {
        let mut ed = Editor::new();
        let root = ed.doc.root;
        let id = ed.doc.ids.next();
        let add = Delta(vec![NodeOp::Add {
            parent: root, index: 0,
            node: Node::shape(id, ShapeKind::Rect { w: 1.0, h: 1.0 }) }]);
        ed.commit(add);
        assert!(ed.doc.get(id).is_some());

        let undo = ed.undo().unwrap();          // forward delta = removal
        assert!(ed.doc.get(id).is_none());
        assert_eq!(undo.0.len(), 1);

        ed.redo().unwrap();
        assert!(ed.doc.get(id).is_some());
    }

    #[test]
    fn commit_clears_redo_stack() {
        let mut ed = Editor::new();
        let id = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root, index: 0,
            node: Node::shape(id, ShapeKind::Rect { w: 1.0, h: 1.0 }) }]));
        ed.undo();
        assert!(ed.redo().is_some());
        // fresh commit after undo must drop redo
        let id2 = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root, index: 0,
            node: Node::shape(id2, ShapeKind::Ellipse { rx: 1.0, ry: 1.0 }) }]));
        assert!(ed.redo().is_none());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p document history::`
Expected: FAIL — `Editor` not found.

- [ ] **Step 3: Write minimal implementation**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use crate::delta::{Document, Delta};

pub struct Editor {
    pub doc: Document,
    undo_stack: Vec<Delta>,   // each entry is an inverse delta
    redo_stack: Vec<Delta>,   // each entry is an inverse delta
}
impl Editor {
    pub fn new() -> Editor { Editor { doc: Document::new(), undo_stack: vec![], redo_stack: vec![] } }

    /// Apply a forward delta as one undoable step. Returns the forward delta (for the UI).
    pub fn commit(&mut self, forward: Delta) -> Delta {
        let inverse = self.doc.apply(forward.clone());
        self.undo_stack.push(inverse);
        self.redo_stack.clear();
        forward
    }
    pub fn undo(&mut self) -> Option<Delta> {
        let inverse = self.undo_stack.pop()?;
        let forward = self.doc.apply(inverse);     // undo == applying the stored inverse
        self.redo_stack.push(forward.clone());
        Some(forward)
    }
    pub fn redo(&mut self) -> Option<Delta> {
        let forward = self.redo_stack.pop()?;
        let inverse = self.doc.apply(forward.clone());
        self.undo_stack.push(inverse);
        Some(forward)
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p document history::`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/document/
git commit -m "Add undo/redo history over the delta model"
```

---

### Task 4: Command builders — transform, primitives, delete, reorder

**Files:**
- Create: `crates/document/src/commands.rs`
- Modify: `crates/document/src/lib.rs` (`pub mod commands;`)

**Interfaces:**
- Consumes: `Editor`, `Document`, `Delta`, `NodeOp`, `Node`, `ShapeKind` (Tasks 1–3), `geometry::Affine`.
- Produces free functions that build forward `Delta`s (not applied — the caller `commit`s them): `transform_nodes`, `add_primitive`, `delete_nodes`, `reorder`. Each returns `Result<Delta, CmdError>`.

- [ ] **Step 1: Write the failing test**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{node::*, delta::*, history::Editor};
    use geometry::Affine;

    #[test]
    fn transform_nodes_multiplies_into_transform() {
        let mut ed = Editor::new();
        let id = ed.doc.ids.next();
        ed.commit(add_primitive(&mut ed.doc.ids, ed.doc.root,
            ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap());
        // NB: add_primitive needs the id it minted; see impl returning (Delta) that mints internally.
        let target = *ed.doc.get(ed.doc.root).unwrap().children.first().unwrap();
        let d = transform_nodes(&ed.doc, &[target], Affine::translate(5.0, 0.0)).unwrap();
        ed.commit(d);
        let t = ed.doc.get(target).unwrap().transform;
        assert_eq!(t.apply(0.0, 0.0), (5.0, 0.0));
    }

    #[test]
    fn delete_removes_node_and_is_undoable() {
        let mut ed = Editor::new();
        ed.commit(add_primitive(&mut ed.doc.ids, ed.doc.root,
            ShapeKind::Ellipse { rx: 3.0, ry: 3.0 }).unwrap());
        let id = *ed.doc.get(ed.doc.root).unwrap().children.first().unwrap();
        ed.commit(delete_nodes(&ed.doc, &[id]).unwrap());
        assert!(ed.doc.get(id).is_none());
        ed.undo();
        assert!(ed.doc.get(id).is_some());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p document commands::`
Expected: FAIL — command fns not found.

- [ ] **Step 3: Write minimal implementation**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use geometry::Affine;
use crate::{node::*, delta::*};

#[derive(Debug, PartialEq)]
pub enum CmdError { NotFound, EmptySelection }

/// Build a delta that appends a new primitive under `parent`. Mints the id from `ids`.
pub fn add_primitive(ids: &mut IdGen, parent: NodeId, kind: ShapeKind) -> Result<Delta, CmdError> {
    let node = Node::shape(ids.next(), kind);
    Ok(Delta(vec![NodeOp::Add { parent, node, index: usize::MAX }]))
}

/// Left-multiply `m` into each node's transform.
pub fn transform_nodes(doc: &Document, ids: &[NodeId], m: Affine) -> Result<Delta, CmdError> {
    if ids.is_empty() { return Err(CmdError::EmptySelection); }
    let mut ops = vec![];
    for &id in ids {
        let before = doc.get(id).ok_or(CmdError::NotFound)?.clone();
        let mut after = before.clone();
        after.transform = m.then(&before.transform);
        ops.push(NodeOp::Update { id, before, after });
    }
    Ok(Delta(ops))
}

pub fn delete_nodes(doc: &Document, ids: &[NodeId]) -> Result<Delta, CmdError> {
    if ids.is_empty() { return Err(CmdError::EmptySelection); }
    let mut ops = vec![];
    for &id in ids {
        let parent = parent_of(doc, id).ok_or(CmdError::NotFound)?;
        ops.push(NodeOp::Remove { parent, id });
    }
    Ok(Delta(ops))
}

pub fn reorder(doc: &Document, id: NodeId, new_index: usize) -> Result<Delta, CmdError> {
    let parent = parent_of(doc, id).ok_or(CmdError::NotFound)?;
    Ok(Delta(vec![
        NodeOp::Remove { parent, id },
        NodeOp::Add { parent, node: doc.get(id).unwrap().clone(), index: new_index },
    ]))
}

fn parent_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    doc.nodes.iter().find(|(_, n)| n.children.contains(&id)).map(|(pid, _)| *pid)
}
```

Fix `Document::apply` `Add` to treat `index == usize::MAX` as append (already clamped by `.min(len)`).

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p document commands::`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/document/
git commit -m "Add transform/primitive/delete/reorder command builders"
```

---

### Task 5: Boolean + text commands (geometry-backed)

**Files:**
- Modify: `crates/document/src/commands.rs`

**Interfaces:**
- Consumes: `geometry::{boolean, BoolOp, text_to_path, rect_path, ellipse_path, Path}`.
- Produces: `boolean_op(doc, ids, BoolOp) -> Result<Delta, CmdError>` (replaces the selection with one Path shape), `add_text(ids, parent, family, size_mm, text) -> Result<Delta, CmdError>`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn boolean_union_replaces_selection_with_single_path() {
    let mut ed = Editor::new();
    for _ in 0..2 {
        ed.commit(add_primitive(&mut ed.doc.ids, ed.doc.root,
            ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap());
    }
    let sel: Vec<NodeId> = ed.doc.get(ed.doc.root).unwrap().children.clone();
    ed.commit(boolean_op(&ed.doc, &sel, geometry::BoolOp::Union).unwrap());
    let kids = &ed.doc.get(ed.doc.root).unwrap().children;
    assert_eq!(kids.len(), 1);
    assert!(matches!(ed.doc.get(kids[0]).unwrap().kind,
        NodeKind::Shape(ShapeKind::Path { .. })));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p document commands::boolean`
Expected: FAIL — `boolean_op` not found.

- [ ] **Step 3: Write minimal implementation**

Append to `commands.rs`:
```rust
use geometry::{boolean, BoolOp, text_to_path, rect_path, ellipse_path, Path};

fn shape_to_path(node: &Node) -> Option<Path> {
    let p = match &node.kind {
        NodeKind::Shape(ShapeKind::Rect { w, h }) => rect_path(0.0, 0.0, *w, *h),
        NodeKind::Shape(ShapeKind::Ellipse { rx, ry }) => ellipse_path(*rx, *ry, *rx, *ry),
        NodeKind::Shape(ShapeKind::Path { d }) => Path::from_svg(d).ok()?,
        _ => return None,
    };
    Some(p.transformed(&node.transform))
}

pub fn boolean_op(doc: &Document, ids: &[NodeId], op: BoolOp) -> Result<Delta, CmdError> {
    if ids.len() < 2 { return Err(CmdError::EmptySelection); }
    let mut paths = vec![];
    for &id in ids { paths.push(shape_to_path(doc.get(id).ok_or(CmdError::NotFound)?)
        .ok_or(CmdError::NotFound)?); }
    let result = boolean(op, &paths).map_err(|_| CmdError::NotFound)?;
    let mut ops: Vec<NodeOp> = ids.iter()
        .map(|&id| NodeOp::Remove { parent: parent_of(doc, id).unwrap(), id })
        .collect();
    // append the merged path (id minted by caller via a fresh IdGen borrow is not available here;
    // callers pass &mut Editor, so expose an Editor method instead — see note).
    ops.push(NodeOp::Add {
        parent: doc.root,
        node: Node::shape(NodeId(u64::MAX), ShapeKind::Path { d: result.to_svg() }),
        index: usize::MAX,
    });
    Ok(Delta(ops))
}
```

> **Id note:** `boolean_op`/`add_text` mint a new node but only `Editor` owns `IdGen`. Add two thin `Editor` methods that inject the id before commit:
```rust
impl Editor {
    pub fn boolean(&mut self, ids: &[NodeId], op: geometry::BoolOp) -> Result<Delta, crate::commands::CmdError> {
        let mut d = crate::commands::boolean_op(&self.doc, ids, op)?;
        if let Some(NodeOp::Add { node, .. }) = d.0.last_mut() { node.id = self.doc.ids.next(); }
        Ok(self.commit(d))
    }
}
```
Update the test to call `ed.boolean(&sel, BoolOp::Union)`. Add `add_text` symmetrically (`text_to_path` → `ShapeKind::Path`). Requires `Path::from_svg`/`to_svg` in geometry (add to the SP2 contract).

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p document commands::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/document/
git commit -m "Add boolean and text commands backed by the geometry crate"
```

---

### Task 6: Machine profile + artboard

**Files:**
- Create: `crates/document/src/machine.rs`
- Modify: `crates/document/src/{lib.rs,delta.rs}` (add `artboard` field to `Document`)

**Interfaces:**
- Produces: `MachineProfile { id, name, width_mm, height_mm }`, `Document.artboard: Rect`, `Editor::set_machine(&mut self, MachineProfile)`, and `builtin_profiles() -> Vec<MachineProfile>` (Cameo 5, Puma IV stubs).

- [ ] **Step 1: Write the failing test**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[test]
fn set_machine_resizes_artboard() {
    let mut ed = Editor::new();
    let puma = builtin_profiles().into_iter().find(|p| p.id == "puma_iv").unwrap();
    ed.set_machine(puma);
    assert!(ed.doc.artboard.w > 300.0);   // Puma is wide-format
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p document machine::`
Expected: FAIL — `builtin_profiles` not found.

- [ ] **Step 3: Write minimal implementation**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Serialize, Deserialize};
use geometry::Rect;
use crate::delta::Document;
use crate::history::Editor;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct MachineProfile { pub id: String, pub name: String, pub width_mm: f64, pub height_mm: f64 }

pub fn builtin_profiles() -> Vec<MachineProfile> {
    vec![
        MachineProfile { id: "cameo5_alpha".into(), name: "Silhouette Cameo 5 Alpha".into(),
                         width_mm: 330.0, height_mm: 3000.0 },
        MachineProfile { id: "puma_iv".into(), name: "GCC Puma IV".into(),
                         width_mm: 600.0, height_mm: 5000.0 },
    ]
}
impl Editor {
    pub fn set_machine(&mut self, p: MachineProfile) {
        self.doc.artboard = Rect { x: 0.0, y: 0.0, w: p.width_mm, h: p.height_mm };
        self.doc.machine = Some(p);
    }
}
```

Add to `Document` (in `delta.rs`): `pub artboard: Rect`, `pub machine: Option<MachineProfile>`, defaulting `artboard` to the first builtin profile's size in `Document::new`. Keep them in the `PartialEq`/`Serialize` derive.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p document machine::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/document/
git commit -m "Add machine profiles and artboard sizing"
```

---

### Task 7: Snapshot serialization for IPC

**Files:**
- Create: `crates/document/src/snapshot.rs`
- Modify: `crates/document/src/lib.rs`

**Interfaces:**
- Produces: `Document::snapshot_json(&self) -> String` and `Delta` already `Serialize`. The UI consumes these verbatim.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn snapshot_round_trips_through_json() {
    let mut ed = Editor::new();
    ed.commit(crate::commands::add_primitive(&mut ed.doc.ids, ed.doc.root,
        ShapeKind::Rect { w: 2.0, h: 2.0 }).unwrap());
    let json = ed.doc.snapshot_json();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ed.doc);
}
```

- [ ] **Step 2–4:** implement `pub fn snapshot_json(&self) -> String { serde_json::to_string(self).unwrap() }`; run `cargo test -p document snapshot::` → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/document/ && git commit -m "Add JSON snapshot serialization for IPC"
```

---

### Task 8: `fileio` — SVG import into a Delta

**Files:**
- Create: `crates/fileio/src/import.rs` (extends SP2's `fileio`)
- Modify: `crates/fileio/src/lib.rs`

**Interfaces:**
- Consumes: `fileio::svg_to_paths` (SP2), `document::{Delta, NodeOp, Node, ShapeKind, NodeId, IdGen}`.
- Produces: `import_svg(bytes: &[u8], ids: &mut IdGen, parent: NodeId) -> Result<(Delta, Vec<String>), IoError>` — the delta plus the skipped-element report.

- [ ] **Step 1: Write the failing test**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[test]
fn import_svg_produces_one_add_per_path() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/></svg>"#;
    let mut ids = document::IdGen::default();
    let (delta, skipped) = import_svg(svg, &mut ids, document::NodeId(1)).unwrap();
    assert_eq!(delta.0.len(), 1);
    assert!(skipped.is_empty());
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p fileio import::` → FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use document::{Delta, NodeOp, Node, ShapeKind, NodeId, IdGen, Style};
use crate::{svg_to_paths, IoError};

pub fn import_svg(bytes: &[u8], ids: &mut IdGen, parent: NodeId)
    -> Result<(Delta, Vec<String>), IoError> {
    let imp = svg_to_paths(bytes)?;
    let ops = imp.paths.into_iter().map(|(path, hint)| {
        let mut node = Node::shape(ids.next(), ShapeKind::Path { d: path.to_svg() });
        node.style = Style { stroke: hint.stroke.or(Some(0x000000FF)), fill: hint.fill };
        NodeOp::Add { parent, node, index: usize::MAX }
    }).collect();
    Ok((Delta(ops), imp.skipped))
}
```

Add `document` as a `fileio` dev/normal dependency in `crates/fileio/Cargo.toml`.

- [ ] **Step 4: Run to verify it passes.** `cargo test -p fileio import::` → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/fileio/ && git commit -m "Add SVG import producing a document Delta"
```

---

### Task 9: `fileio` — project save/load (atomic zip container)

**Files:**
- Create: `crates/fileio/src/project.rs`
- Modify: `crates/fileio/{src/lib.rs,Cargo.toml}` (add `zip = "2"`, `tempfile = "3"`)

**Interfaces:**
- Produces: `save_project(path: &Path, doc: &Document) -> Result<(), IoError>` (writes `design.svg` + `manifest.json` atomically), `load_project(path: &Path) -> Result<Document, IoError>`.

- [ ] **Step 1: Write the failing test**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[test]
fn save_then_load_round_trips_document() {
    let mut doc = document::Document::new();
    let id = doc.ids.next();
    doc.apply(document::Delta(vec![document::NodeOp::Add {
        parent: doc.root, index: 0,
        node: document::Node::shape(id, document::ShapeKind::Rect { w: 5.0, h: 5.0 }) }]));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("proj.cut");
    save_project(&path, &doc).unwrap();
    let back = load_project(&path).unwrap();
    assert_eq!(back, doc);
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p fileio project::` → FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::Path;
use std::io::{Write, Read};
use document::Document;
use crate::IoError;

pub fn save_project(path: &Path, doc: &Document) -> Result<(), IoError> {
    let tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap())
        .map_err(|e| IoError::Io(e.to_string()))?;
    let mut zip = zip::ZipWriter::new(tmp.reopen().map_err(|e| IoError::Io(e.to_string()))?);
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", opts).map_err(|e| IoError::Io(e.to_string()))?;
    zip.write_all(doc.snapshot_json().as_bytes()).map_err(|e| IoError::Io(e.to_string()))?;
    zip.start_file("design.svg", opts).map_err(|e| IoError::Io(e.to_string()))?;
    zip.write_all(crate::doc_to_svg(doc).as_bytes()).map_err(|e| IoError::Io(e.to_string()))?;
    zip.finish().map_err(|e| IoError::Io(e.to_string()))?;
    tmp.persist(path).map_err(|e| IoError::Io(e.to_string()))?;   // atomic rename
    Ok(())
}

pub fn load_project(path: &Path) -> Result<Document, IoError> {
    let file = std::fs::File::open(path).map_err(|e| IoError::Io(e.to_string()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| IoError::Parse(e.to_string()))?;
    let mut s = String::new();
    zip.by_name("manifest.json").map_err(|e| IoError::Parse(e.to_string()))?
        .read_to_string(&mut s).map_err(|e| IoError::Io(e.to_string()))?;
    serde_json::from_str(&s).map_err(|e| IoError::Parse(e.to_string()))
}
```

`doc_to_svg` (a small serializer of the scene tree to SVG for the canonical `design.svg`) lives in SP2's `fileio`; for SP3 the manifest is the source of truth on load, and `design.svg` is the interchange copy. A minimal `doc_to_svg` that emits each Path is enough for V1.

- [ ] **Step 4: Run to verify it passes.** `cargo test -p fileio project::` → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/fileio/ && git commit -m "Add atomic project save/load zip container"
```

---

### Task 10: Tauri app + IPC commands

**Files:**
- Create: `apps/desktop/Cargo.toml`, `apps/desktop/tauri.conf.json`, `apps/desktop/src/main.rs`, `apps/desktop/src/ipc.rs`
- Create: `apps/desktop/src/state.rs`

**Interfaces:**
- Consumes: `document::Editor`, `fileio::{import_svg, save_project, load_project}`.
- Produces: Tauri `#[tauri::command]` handlers returning `Result<T, String>`: `new_doc`, `snapshot`, `commit_transform`, `add_primitive`, `boolean_op`, `add_text`, `delete`, `reorder`, `undo`, `redo`, `import_svg`, `save_project`, `load_project`, `set_machine`, `list_machines`. All mutate a `Mutex<Editor>` in Tauri state and return a `Delta` (or snapshot) as JSON-serializable.

- [ ] **Step 1: Write the failing test** (Rust integration test on the state layer, no Tauri runtime needed)

`apps/desktop/src/state.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_state_commit_transform_moves_node() {
        let mut app = AppState::new();
        let id = app.add_rect(10.0, 10.0);
        app.commit_transform(vec![id], geometry::Affine::translate(3.0, 0.0)).unwrap();
        assert_eq!(app.editor.doc.get(id).unwrap().transform.apply(0.0, 0.0), (3.0, 0.0));
    }
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p desktop state::` → FAIL.

- [ ] **Step 3: Write minimal implementation**

`apps/desktop/src/state.rs` wraps `Editor` with thin methods returning `Delta`; `add_rect` is a helper the test uses:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use document::{Editor, NodeId, ShapeKind, commands};
use geometry::{Affine, BoolOp};

pub struct AppState { pub editor: Editor }
impl AppState {
    pub fn new() -> Self { AppState { editor: Editor::new() } }
    pub fn add_rect(&mut self, w: f64, h: f64) -> NodeId {
        let d = commands::add_primitive(&mut self.editor.doc.ids, self.editor.doc.root,
            ShapeKind::Rect { w, h }).unwrap();
        // capture minted id from the Add op
        let id = if let document::NodeOp::Add { node, .. } = &d.0[0] { node.id } else { unreachable!() };
        self.editor.commit(d);
        id
    }
    pub fn commit_transform(&mut self, ids: Vec<NodeId>, m: Affine)
        -> Result<document::Delta, String> {
        let d = commands::transform_nodes(&self.editor.doc, &ids, m).map_err(|e| format!("{e:?}"))?;
        Ok(self.editor.commit(d))
    }
}
```

`ipc.rs` exposes `#[tauri::command]` wrappers over `Mutex<AppState>`; `main.rs` registers them via `tauri::generate_handler!` and serves the UI. (These are wiring; the logic is tested in `state.rs`.)

- [ ] **Step 4: Run to verify it passes.** `cargo test -p desktop state::` → PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ && git commit -m "Add Tauri app shell and IPC command layer"
```

---

### Task 11: UI scaffold + Workbench tokens + Renderer interface

**Files:**
- Create: `apps/desktop/ui/package.json`, `apps/desktop/ui/index.html`, `apps/desktop/ui/src/main.tsx`, `apps/desktop/ui/src/tokens.css`
- Create: `apps/desktop/ui/src/render/Renderer.ts`, `apps/desktop/ui/src/render/Canvas2DRenderer.ts`
- Create: `apps/desktop/ui/src/render/hittest.ts`, `apps/desktop/ui/src/render/hittest.test.ts`

**Interfaces:**
- Produces: `interface Renderer { setScene(s: Scene): void; markDirty(id: NodeId): void; draw(): void }`, `Canvas2DRenderer`, and pure `hitTest(scene, x, y): NodeId | null`.

- [ ] **Step 1: Write the failing test** (`hittest.test.ts`, Vitest)

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { hitTest } from "./hittest";

describe("hitTest", () => {
  it("returns the topmost node whose bounds contain the point", () => {
    const scene = { nodes: [
      { id: 1, bounds: { x: 0, y: 0, w: 10, h: 10 } },
      { id: 2, bounds: { x: 5, y: 5, w: 10, h: 10 } },
    ]};
    expect(hitTest(scene, 7, 7)).toBe(2);   // 2 is on top and contains the point
    expect(hitTest(scene, 1, 1)).toBe(1);
    expect(hitTest(scene, 99, 99)).toBe(null);
  });
});
```

- [ ] **Step 2: Run to verify it fails.** `cd apps/desktop/ui && npx vitest run hittest` → FAIL.

- [ ] **Step 3: Write minimal implementation** — `hittest.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
export type Bounds = { x: number; y: number; w: number; h: number };
export type SceneNode = { id: number; bounds: Bounds };
export type Scene = { nodes: SceneNode[] };

export function hitTest(scene: Scene, x: number, y: number): number | null {
  for (let i = scene.nodes.length - 1; i >= 0; i--) {   // topmost last
    const b = scene.nodes[i].bounds;
    if (x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h) return scene.nodes[i].id;
  }
  return null;
}
```

Add `tokens.css` with the Workbench variables; `Renderer.ts` (interface) and `Canvas2DRenderer.ts` (dirty-rect draw loop) as wiring.

- [ ] **Step 4: Run to verify it passes.** `npx vitest run hittest` → PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/ && git commit -m "Add UI scaffold, Workbench tokens, renderer + hit-test"
```

---

### Task 12: Optimistic transform + commit reconciliation (pure TS logic)

**Files:**
- Create: `apps/desktop/ui/src/interaction/transform.ts`, `apps/desktop/ui/src/interaction/transform.test.ts`
- Create: `apps/desktop/ui/src/ipc.ts` (thin `invoke` wrappers)

**Interfaces:**
- Produces: `dragMatrix(start, current): Matrix`, `applyOptimistic(scene, ids, m): Scene`, `reconcile(scene, delta): Scene` — all pure. The controller wires pointer events to these + `ipc.commitTransform`.

- [ ] **Step 1: Write the failing test**

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { dragMatrix, applyOptimistic, reconcile } from "./transform";

describe("optimistic transform", () => {
  it("dragMatrix builds a translation from start→current", () => {
    expect(dragMatrix({ x: 2, y: 3 }, { x: 5, y: 3 })).toEqual([1, 0, 0, 1, 3, 0]);
  });
  it("applyOptimistic offsets only selected node bounds", () => {
    const scene = { nodes: [
      { id: 1, bounds: { x: 0, y: 0, w: 4, h: 4 } },
      { id: 2, bounds: { x: 0, y: 0, w: 4, h: 4 } },
    ]};
    const out = applyOptimistic(scene, [2], [1, 0, 0, 1, 5, 0]);
    expect(out.nodes[1].bounds.x).toBe(5);
    expect(out.nodes[0].bounds.x).toBe(0);
  });
  it("reconcile applies an update op from the authoritative delta", () => {
    const scene = { nodes: [{ id: 1, bounds: { x: 0, y: 0, w: 4, h: 4 } }] };
    const out = reconcile(scene, [{ op: "update", nodeId: 1, patch: { bounds: { x: 9, y: 0, w: 4, h: 4 } } }]);
    expect(out.nodes[0].bounds.x).toBe(9);
  });
});
```

- [ ] **Step 2: Run to verify it fails.** `npx vitest run transform` → FAIL.

- [ ] **Step 3: Write minimal implementation** — `transform.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { Scene } from "../render/hittest";
export type Pt = { x: number; y: number };
export type Matrix = [number, number, number, number, number, number]; // a b c d e f

export function dragMatrix(start: Pt, cur: Pt): Matrix {
  return [1, 0, 0, 1, cur.x - start.x, cur.y - start.y];
}
export function applyOptimistic(scene: Scene, ids: number[], m: Matrix): Scene {
  return { nodes: scene.nodes.map(n =>
    ids.includes(n.id)
      ? { ...n, bounds: { ...n.bounds, x: n.bounds.x + m[4], y: n.bounds.y + m[5] } }
      : n) };
}
export type DeltaOp = { op: "add" | "update" | "remove"; nodeId: number; patch?: any };
export function reconcile(scene: Scene, delta: DeltaOp[]): Scene {
  let nodes = scene.nodes.slice();
  for (const d of delta) {
    if (d.op === "update") nodes = nodes.map(n => n.id === d.nodeId ? { ...n, ...d.patch } : n);
    else if (d.op === "remove") nodes = nodes.filter(n => n.id !== d.nodeId);
    else if (d.op === "add") nodes.push(d.patch);
  }
  return { nodes };
}
```

- [ ] **Step 4: Run to verify it passes.** `npx vitest run transform` → PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/ && git commit -m "Add optimistic transform + reconcile logic"
```

---

### Task 13: Panels + app assembly (wiring)

**Files:**
- Create: `apps/desktop/ui/src/panels/{TopBar,ToolRail,LayersPanel,PropertiesPanel,StatusBar}.tsx`
- Create: `apps/desktop/ui/src/panels/NumberField.tsx`, `apps/desktop/ui/src/panels/NumberField.test.ts`
- Create: `apps/desktop/ui/src/App.tsx`

**Interfaces:**
- Produces: the assembled editor. Only `NumberField` (drag-scrub numeric input) carries testable logic; panels are wiring over `ipc.ts`.

- [ ] **Step 1: Write the failing test** (`NumberField.test.ts` — the scrub math)

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { scrubValue } from "./NumberField";

describe("scrubValue", () => {
  it("changes value by dx * step", () => {
    expect(scrubValue(10, 4, 0.5)).toBe(12);       // 10 + 4*0.5
  });
  it("respects a min clamp", () => {
    expect(scrubValue(1, -100, 1, 0)).toBe(0);
  });
});
```

- [ ] **Step 2: Run to verify it fails.** `npx vitest run NumberField` → FAIL.

- [ ] **Step 3: Write minimal implementation** — `NumberField.tsx` exports the pure helper plus the component:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
export function scrubValue(v: number, dx: number, step: number, min = -Infinity): number {
  return Math.max(min, v + dx * step);
}
```

The component uses `scrubValue` on pointer-drag over the label; `TopBar` calls `ipc.listMachines`/`ipc.setMachine`; `LayersPanel` renders the tree from the snapshot; `PropertiesPanel` shows X/Y/W/H/∠ via `NumberField` and calls `ipc.commitTransform`; `App.tsx` lays them out per the wireframe using `tokens.css`. These are wiring over already-tested logic.

- [ ] **Step 4: Run to verify it passes.** `npx vitest run NumberField` → PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/ && git commit -m "Add editor panels, drag-scrub number field, app layout"
```

---

### Task 14: End-to-end smoke + manual checklist

**Files:**
- Create: `apps/desktop/ui/e2e/smoke.spec.ts` (Playwright against the built Tauri app or the Vite dev server with a mocked `invoke`)
- Create: `docs/protocol/../` → `apps/desktop/MANUAL-CHECKLIST.md`

**Interfaces:**
- Consumes: the whole app.
- Produces: one automated happy-path smoke + a human checklist for interaction/visual items automation can't judge.

- [ ] **Step 1: Write the smoke test**

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { test, expect } from "@playwright/test";
test("new doc → add rect → save → reload keeps the rect", async ({ page }) => {
  await page.goto("http://localhost:5173");
  await page.getByRole("button", { name: "Rectangle" }).click();
  await page.mouse.click(400, 300);
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
  await page.getByRole("button", { name: "Save" }).click();
  await page.getByRole("button", { name: "Reload" }).click();
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
});
```

- [ ] **Step 2: Run to verify it fails.** `npx playwright test smoke` → FAIL (buttons/testids not yet wired).

- [ ] **Step 3: Wire the missing `data-testid`/`aria` labels** in the panels from Task 13 until the smoke passes. Write `MANUAL-CHECKLIST.md`:

```markdown
# SP3 manual checklist (per release)
- [ ] Switch machine Cameo 5 ⇄ Puma IV — artboard resizes.
- [ ] Import a complex SVG — stays 60fps while panning/zooming.
- [ ] Drag/scale/rotate — handles are cyan; motion is smooth; one undo reverts the whole gesture.
- [ ] Boolean union/subtract/intersect/exclude on two overlapping shapes — correct result.
- [ ] Add text in a system font — converts to a cut path on save.
- [ ] Dark and light themes both legible; tabular numerals align in the properties panel.
```

- [ ] **Step 4: Run to verify it passes.** `npx playwright test smoke` → PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ && git commit -m "Add end-to-end smoke test and manual release checklist"
```

---

## Self-review

**Spec coverage:** hybrid model (Tasks 2–4, 12) · scene-graph renderer + WebGL-swap interface (Task 11) · SVG-only import (Task 8) · save/load open container (Task 9) · machine-area artboard + stub profiles (Task 6) · V1 tool set: select/transform (4,12), primitives (4), boolean (5), text (5), align/reorder/delete (4; align folds into `transform_nodes` at the UI layer) · undo = one gesture (3) · Workbench tokens (11,13) · error handling as typed Results (4,5,8,9,10) · testing across Rust + pure TS + one smoke (all tasks, 14). Deferred items (node editing, PDF/DXF, cut/trace/print&cut) have no tasks — correct.

**Placeholder scan:** no TBD/TODO; every code step has real code. The two "wiring" tasks (13 panels, 10 Tauri handlers) isolate their untested glue behind tested pure logic (`scrubValue`, `AppState`, `hitTest`, `transform`).

**Type consistency:** `Delta`/`NodeOp`/`Node`/`NodeId`/`ShapeKind`/`Editor`/`Document` names consistent across Tasks 1–10; `Scene`/`hitTest`/`reconcile`/`Matrix` consistent across 11–13. `Editor::boolean`/`add_text` inject the minted id (Task 5 note) to keep `IdGen` the sole id source.

**Known follow-ups folded into the SP2 contract:** `Affine` must derive `Serialize/Deserialize/PartialEq`; `Path` needs `from_svg`/`to_svg`; `fileio` needs `doc_to_svg`. All listed in "Assumed SP2 interfaces" — resolve there, not per-task.
