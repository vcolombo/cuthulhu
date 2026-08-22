// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashSet;
use geometry::{boolean, ellipse_path, rect_path, text_to_path, Affine, BoolOp, Path};
use crate::{node::*, delta::*};

#[derive(Debug, PartialEq)]
pub enum CmdError { NotFound, NoParent, EmptySelection, Geometry(String), EmptyPresetId }

/// What the operator reads when a command refuses: the desktop's `ipc` layer forwards this
/// string straight into the dialog. It used to forward `{e:?}` instead, so a boolean op on
/// two shapes that do not overlap arrived as `Geometry("Degenerate")` — a struct literal
/// wrapped around the sentence `GeomError` had already written (#93).
impl std::fmt::Display for CmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Two rules raise this, and only a wording that names both is true of both: a node
            // id the document does not hold, and `set_machine` naming a profile this build does
            // not ship (a project saved against a cutter it has never heard of).
            //
            // "node", not "shape": every command here takes ids straight from the selection, and
            // the Layers panel selects Groups and Layers too, so the id that went stale is as
            // often a container. Node is CONTEXT.md's word for all three.
            CmdError::NotFound => write!(f, "the node or machine this command names is not there"),
            // Split off `NotFound` rather than widening it: a node with no parent is present,
            // and saying it "is not there" was the third rewording that sentence needed. Reached
            // by the document root and by an orphan out of a manifest, whose topology is not
            // validated on load.
            CmdError::NoParent => write!(f, "this node has no parent, and the command needs one"),
            // Not only an empty selection: also a boolean op given one shape, and a delete whose
            // every id was skipped as the descendant of another. In each the selection exists and
            // still offers this command nothing to do.
            CmdError::EmptySelection => write!(f, "the selection has nothing this command can act on"),
            // No clause in front of the payload: every site that builds one writes a finished
            // sentence, so a prefix would make the refusal read twice (the reason #90 dropped
            // `"preflight: "`).
            CmdError::Geometry(m) => write!(f, "{m}"),
            CmdError::EmptyPresetId => write!(f, "a material assignment needs a preset id"),
        }
    }
}

impl std::error::Error for CmdError {}

/// Build a delta that appends a new primitive under `parent`. Mints the id from `ids`.
pub fn add_primitive(ids: &mut IdGen, parent: NodeId, kind: ShapeKind) -> Result<Delta, CmdError> {
    let node = Node::shape(ids.next(), kind);
    Ok(Delta(vec![NodeOp::Add { parent, node, index: usize::MAX }]))
}

/// Apply world-space transform `m` to each node by composing it with the node's existing local transform.
/// Converts the world-space matrix into the node's parent space so that new_world = old_world.then(m)
/// holds under transformed ancestors.
pub fn transform_nodes(doc: &Document, ids: &[NodeId], m: Affine) -> Result<Delta, CmdError> {
    if ids.is_empty() { return Err(CmdError::EmptySelection); }
    let selected: HashSet<NodeId> = ids.iter().copied().collect();
    let mut ops = vec![];
    let mut seen = HashSet::new();
    for &id in ids {
        let node = doc.get(id).ok_or(CmdError::NotFound)?;
        // A selected ancestor already carries this node along; updating it too
        // would move its world position by `m` twice.
        if !seen.insert(id) || has_selected_ancestor(doc, &selected, id) { continue; }
        let before = node.clone();
        // Convert the world-space matrix into this node's parent space so that
        // new_world = old_world.then(m) holds under transformed ancestors:
        // new_local = old_local.then(pw).then(m).then(pw⁻¹)
        let pw = match parent_of(doc, id) {
            Some(pid) => world_transform(doc, pid).ok_or(CmdError::NotFound)?,
            None => Affine::identity(),
        };
        // "something in the selection", not "this shape": `transform_nodes` acts on whole
        // selected subtrees, so the node under the singular ancestor is as often a Group.
        let pw_inv = pw.inverse().ok_or_else(|| CmdError::Geometry(
            "something in the selection sits under a transform that cannot be reversed".into()))?;
        let mut after = before.clone();
        after.transform = before.transform.then(&pw).then(&m).then(&pw_inv);
        ops.push(NodeOp::Update { id, before, after });
    }
    Ok(Delta(ops))
}

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
        // otherwise spin here. Unlike `plan_passes_with`, which errors on a revisit because a
        // preorder walk from the single root can only reach a node twice through a cycle, this
        // walk starts from an arbitrary selection where a revisit is the ordinary overlapping
        // case — a group and a shape inside it — so it must skip the node, not refuse the edit.
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
    // An empty id names no material. `PassKey` parses `preset:` so that its grammar is total in
    // both languages, which means such an assignment would reach the cut path and be refused
    // there; refusing it at the edit is where an operator can still act on it.
    if matches!(&value, PresetAssignment::Preset(id) if id.is_empty()) {
        return Err(CmdError::EmptyPresetId);
    }
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

pub fn delete_nodes(doc: &Document, ids: &[NodeId]) -> Result<Delta, CmdError> {
    if ids.is_empty() { return Err(CmdError::EmptySelection); }
    let selected: HashSet<NodeId> = ids.iter().copied().collect();
    // Emit a subtree's Removes children-first so each Remove still has its parent
    // in the map, and the inverse delta (reversed Adds) restores parents first.
    fn push_subtree(doc: &Document, id: NodeId, parent: NodeId, ops: &mut Vec<NodeOp>)
        -> Result<(), CmdError> {
        let node = doc.get(id).ok_or(CmdError::NotFound)?;
        for &child in &node.children {
            push_subtree(doc, child, id, ops)?;
        }
        ops.push(NodeOp::Remove { parent, id });
        Ok(())
    }
    let mut ops = vec![];
    let mut seen = HashSet::new();
    for &id in ids {
        if !seen.insert(id) || has_selected_ancestor(doc, &selected, id) { continue; }
        // Existence first: an id that names nothing has no parent either, so asking `parent_of`
        // about it would answer `NoParent` for what is really a stale selection.
        doc.get(id).ok_or(CmdError::NotFound)?;
        let parent = parent_of(doc, id).ok_or(CmdError::NoParent)?;
        push_subtree(doc, id, parent, &mut ops)?;
    }
    if ops.is_empty() { return Err(CmdError::EmptySelection); }
    Ok(Delta(ops))
}

pub fn reorder(doc: &Document, id: NodeId, new_index: usize) -> Result<Delta, CmdError> {
    // Existence first, for the reason `delete_nodes` gives above.
    let node = doc.get(id).ok_or(CmdError::NotFound)?.clone();
    let parent = parent_of(doc, id).ok_or(CmdError::NoParent)?;
    Ok(Delta(vec![
        NodeOp::Remove { parent, id },
        NodeOp::Add { parent, node, index: new_index },
    ]))
}

fn parent_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    doc.nodes.iter().find(|(_, n)| n.children.contains(&id)).map(|(pid, _)| *pid)
}

/// True when any ancestor of `id` is also in `selected`. Commands that act on a
/// selection per-subtree (transform, delete) skip such nodes so that exactly one
/// operation applies per selected subtree — the ancestor carries them along.
fn has_selected_ancestor(doc: &Document, selected: &HashSet<NodeId>, id: NodeId) -> bool {
    let mut cur = id;
    while let Some(pid) = parent_of(doc, cur) {
        if selected.contains(&pid) { return true; }
        cur = pid;
    }
    false
}

/// World transform of `id`: its local transform composed through every ancestor
/// (node world = local.then(parent world)). None if `id` is not in the document.
pub fn world_transform(doc: &Document, id: NodeId) -> Option<Affine> {
    let mut m = doc.get(id)?.transform.clone();
    let mut cur = id;
    while let Some(pid) = parent_of(doc, cur) {
        m = m.then(&doc.get(pid)?.transform);
        cur = pid;
    }
    Some(m)
}

/// Shape's outline in its own local space (node's own transform NOT applied), in mm,
/// matching `Rect { x:0, y:0, w, h }` bounds convention (an ellipse of radii rx,ry
/// centered at (rx,ry) has the same 0,0-origin bounds). `None` for containers (Group/Layer).
/// Text converts via `geometry::text_to_path`; font/parse failures come back as `Err`.
pub fn shape_outline(node: &Node) -> Result<Option<Path>, String> {
    match &node.kind {
        NodeKind::Shape(ShapeKind::Rect { w, h }) => Ok(Some(rect_path(0.0, 0.0, *w, *h))),
        NodeKind::Shape(ShapeKind::Ellipse { rx, ry }) => Ok(Some(ellipse_path(*rx, *ry, *rx, *ry))),
        // `to_string`, not `{e:?}`: this string is carried verbatim into `PlanError::BadShape`
        // and `CmdError::Geometry`, and both of those reach an operator (#91).
        NodeKind::Shape(ShapeKind::Path { d }) => Path::from_svg(d).map(Some).map_err(|e| e.to_string()),
        NodeKind::Shape(ShapeKind::Text { family, size_mm, text }) =>
            text_to_path(family, *size_mm, text).map(Some).map_err(|e| e.to_string()),
        _ => Ok(None),
    }
}

/// Replace `ids` (>= 2 shape nodes) with a single Path node holding the boolean-op result.
/// Inputs are flattened via each node's world transform (so nodes at different nesting
/// depths combine correctly), and the result is mapped back into the destination parent's
/// space before being appended under the parent of `ids[0]`. Mints `NodeId(u64::MAX)` as a
/// placeholder for the new node's id — `Editor::boolean` overwrites it before commit.
pub fn boolean_op(doc: &Document, ids: &[NodeId], op: BoolOp) -> Result<Delta, CmdError> {
    let mut seen = HashSet::new();
    let ids: Vec<NodeId> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();
    if ids.len() < 2 { return Err(CmdError::EmptySelection); }
    // Settle the tree before the geometry: whether a selected node exists and where its result
    // can land does not depend on any outline, so a stale id or an orphan should say so rather
    // than lose the race to a no-outline or empty-result refusal. Walking `parent_of` once per
    // id also drops a second scan of `doc.nodes` per node.
    let parents: Vec<NodeId> = ids.iter()
        .map(|&id| {
            doc.get(id).ok_or(CmdError::NotFound)?;
            parent_of(doc, id).ok_or(CmdError::NoParent)
        })
        .collect::<Result<_, CmdError>>()?;
    let dest_parent = parents[0];

    let mut paths = vec![];
    for &id in &ids {
        let node = doc.get(id).expect("settled above");
        // `Ok(None)` is a Group or Layer: present, and simply without an outline of its own —
        // not the absent node `NotFound` names. Reachable, because the Layers panel selects
        // containers and the toolbar offers a boolean op on any two selected nodes.
        let local = shape_outline(node).map_err(CmdError::Geometry)?.ok_or_else(|| {
            CmdError::Geometry("a group or layer has no outline of its own to combine".into())
        })?;
        let world = world_transform(doc, id).ok_or(CmdError::NotFound)?;
        paths.push(local.transformed(&world));
    }
    // `to_string`, not `{e:?}`: the same operator-facing contract as `shape_outline` above.
    // Two shapes that do not overlap refuse here, and `Degenerate` alone said nothing (#93).
    let result = boolean(op, &paths).map_err(|e| CmdError::Geometry(e.to_string()))?;
    let dest_world = world_transform(doc, dest_parent).ok_or(CmdError::NotFound)?;
    let dest_inv = dest_world.inverse()
        .ok_or_else(|| CmdError::Geometry(
            "the result's parent sits under a transform that cannot be reversed".into()))?;
    let result_local = result.transformed(&dest_inv);
    let mut ops: Vec<NodeOp> = ids.iter().zip(parents)
        .map(|(&id, parent)| NodeOp::Remove { parent, id })
        .collect();
    ops.push(NodeOp::Add {
        parent: dest_parent,
        node: Node::shape(NodeId(u64::MAX), ShapeKind::Path { d: result_local.to_svg() }),
        index: usize::MAX,
    });
    Ok(Delta(ops))
}

/// Append a text node's glyph outlines (as a single Path) under `parent`. Mints
/// `NodeId(u64::MAX)` as a placeholder — `Editor::add_text` overwrites it before commit.
pub fn add_text(doc: &Document, parent: NodeId, family: &str, size_mm: f64, text: &str) -> Result<Delta, CmdError> {
    doc.get(parent).ok_or(CmdError::NotFound)?;
    // `to_string`, not `{e:?}`: same operator-facing contract as `shape_outline` (#91).
    let path = text_to_path(family, size_mm, text).map_err(|e| CmdError::Geometry(e.to_string()))?;
    let node = Node::shape(NodeId(u64::MAX), ShapeKind::Path { d: path.to_svg() });
    Ok(Delta(vec![NodeOp::Add { parent, node, index: usize::MAX }]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Editor;
    use geometry::Affine;

    /// The whole table at once: a new variant fails to compile the match in `Display`, and a
    /// reworded one fails here. These strings are what an operator reads — every one of them
    /// used to arrive as `{e:?}` (`EmptySelection`, `Geometry("Degenerate")`), which is why
    /// this type gained `Display` at all (#93).
    #[test]
    fn every_command_refusal_has_a_sentence() {
        let cases: Vec<(CmdError, &str)> = vec![
            (CmdError::NotFound, "the node or machine this command names is not there"),
            (CmdError::NoParent, "this node has no parent, and the command needs one"),
            (CmdError::EmptySelection, "the selection has nothing this command can act on"),
            // Forwarded verbatim, so what the operator reads for a boolean op on shapes that do
            // not overlap is the sentence `GeomError` writes, with nothing wrapped around it.
            (
                CmdError::Geometry(geometry::GeomError::Degenerate.to_string()),
                "the operation left no geometry behind",
            ),
            (CmdError::EmptyPresetId, "a material assignment needs a preset id"),
        ];
        for (error, sentence) in cases {
            assert_eq!(error.to_string(), sentence, "{error:?}");
        }
    }

    /// The end-to-end shape of #93, one layer below the IPC call: the refusal an operator gets
    /// for a boolean op on two shapes that do not overlap is a sentence, not `Degenerate`.
    #[test]
    fn a_boolean_op_on_disjoint_shapes_refuses_in_words() {
        let mut ed = Editor::new();
        let a = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root,
            node: Node::shape(a, ShapeKind::Rect { w: 10.0, h: 10.0 }), index: 0 }]));
        let b = ed.doc.ids.next();
        let mut far = Node::shape(b, ShapeKind::Rect { w: 10.0, h: 10.0 });
        far.transform = Affine::translate(100.0, 100.0);
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root, node: far, index: 1 }]));

        let refusal = boolean_op(&ed.doc, &[a, b], geometry::BoolOp::Intersect).unwrap_err();
        assert_eq!(refusal.to_string(), "the operation left no geometry behind");
    }

    /// A collapsed transform (scale 0) is the only way to reach the two hand-written
    /// `Geometry` payloads, so build one: without a construction path they are strings no test
    /// reads, free to drift back into the noun phrases they used to be.
    fn singular() -> Affine {
        Affine([0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
    }

    /// A *container* under the collapsed ancestor, deliberately: `transform_nodes` acts on whole
    /// selected subtrees, so the refusal must not call what it names a shape.
    #[test]
    fn a_transform_under_a_collapsed_ancestor_refuses_in_words() {
        let mut ed = Editor::new();
        let outer = ed.doc.ids.next();
        let mut collapsed = Node::container(outer, NodeKind::Group);
        collapsed.transform = singular();
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root, node: collapsed, index: 0 }]));
        let inner = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: outer,
            node: Node::container(inner, NodeKind::Group), index: 0 }]));

        let refusal = transform_nodes(&ed.doc, &[inner], Affine::translate(5.0, 0.0)).unwrap_err();
        assert_eq!(refusal.to_string(),
            "something in the selection sits under a transform that cannot be reversed");
    }

    /// The result lands under the parent of `ids[0]`, so a collapsed parent there refuses after
    /// the op itself succeeded — a different sentence from the ancestor case above.
    #[test]
    fn a_boolean_op_into_a_collapsed_parent_refuses_in_words() {
        let mut ed = Editor::new();
        let outer = ed.doc.ids.next();
        let mut collapsed = Node::container(outer, NodeKind::Group);
        collapsed.transform = singular();
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root, node: collapsed, index: 0 }]));
        let a = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: outer,
            node: Node::shape(a, ShapeKind::Rect { w: 10.0, h: 10.0 }), index: 0 }]));
        let b = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root,
            node: Node::shape(b, ShapeKind::Rect { w: 10.0, h: 10.0 }), index: 1 }]));

        let refusal = boolean_op(&ed.doc, &[a, b], geometry::BoolOp::Union).unwrap_err();
        assert_eq!(refusal.to_string(),
            "the result's parent sits under a transform that cannot be reversed");
    }

    /// A Group is present and has no outline of its own, which is not what `NotFound` says.
    /// Reachable: the Layers panel selects containers and the toolbar offers a boolean op on
    /// any two selected nodes, so Union over two Layers lands here.
    #[test]
    fn a_boolean_op_on_containers_says_they_have_no_outline() {
        let mut ed = Editor::new();
        let ids: Vec<NodeId> = (0..2).map(|i| {
            let id = ed.doc.ids.next();
            ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root,
                node: Node::container(id, NodeKind::Layer), index: i }]));
            id
        }).collect();

        let refusal = boolean_op(&ed.doc, &ids, geometry::BoolOp::Union).unwrap_err();
        assert_eq!(refusal.to_string(), "a group or layer has no outline of its own to combine");
    }

    /// The document root is present and parentless, so `delete` and `reorder` used to call it
    /// missing. Codex found it in the review of #93; the Layers panel gives the root no row, so
    /// nothing wired reaches this, but the sentence has to be true wherever the id comes from.
    ///
    /// `boolean_op` is here in both positions because it is the one command that walks more than
    /// one id: its parent lookup was a `.unwrap()` justified only for `ids[0]`, so the id *after*
    /// a good one is where a revert to `NotFound` — or to a panic — would show. Left uncovered
    /// when #277 hit the review-push cap.
    #[test]
    fn a_command_on_the_parentless_root_says_it_has_no_parent() {
        let mut ed = Editor::new();
        let root = ed.doc.root;
        let d = add_primitive(&mut ed.doc.ids, root, ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap();
        ed.commit(d);
        let real = *ed.doc.get(root).unwrap().children.first().unwrap();
        assert!(ed.doc.get(root).is_some(), "the root is present, which is the whole point");

        for refusal in [
            delete_nodes(&ed.doc, &[root]).unwrap_err(),
            reorder(&ed.doc, root, 0).unwrap_err(),
            boolean_op(&ed.doc, &[root, real], geometry::BoolOp::Union).unwrap_err(),
            boolean_op(&ed.doc, &[real, root], geometry::BoolOp::Union).unwrap_err(),
        ] {
            assert_eq!(refusal.to_string(), "this node has no parent, and the command needs one");
        }
    }

    /// The other side of `NoParent`: an id naming nothing has no parent either, so a command
    /// that asked `parent_of` first would blame the parent for a stale selection. Copilot found
    /// that regression in the review of #93.
    #[test]
    fn a_stale_id_is_still_not_found_rather_than_parentless() {
        let mut ed = Editor::new();
        let d = add_primitive(&mut ed.doc.ids, ed.doc.root,
            ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap();
        ed.commit(d);
        let real = *ed.doc.get(ed.doc.root).unwrap().children.first().unwrap();
        let stale = NodeId(9999);

        for refusal in [
            delete_nodes(&ed.doc, &[stale]).unwrap_err(),
            reorder(&ed.doc, stale, 0).unwrap_err(),
            boolean_op(&ed.doc, &[stale, real], geometry::BoolOp::Union).unwrap_err(),
        ] {
            assert_eq!(refusal, CmdError::NotFound);
        }
    }

    #[test]
    fn transform_nodes_multiplies_into_transform() {
        let mut ed = Editor::new();
        let d = add_primitive(&mut ed.doc.ids, ed.doc.root,
            ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap();
        ed.commit(d);
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
        let d = add_primitive(&mut ed.doc.ids, ed.doc.root,
            ShapeKind::Ellipse { rx: 3.0, ry: 3.0 }).unwrap();
        ed.commit(d);
        let id = *ed.doc.get(ed.doc.root).unwrap().children.first().unwrap();
        ed.commit(delete_nodes(&ed.doc, &[id]).unwrap());
        assert!(ed.doc.get(id).is_none());
        ed.undo();
        assert!(ed.doc.get(id).is_some());
    }

    #[test]
    fn transform_composes_in_world_space_over_existing_transform() {
        let mut ed = Editor::new();
        let d = add_primitive(&mut ed.doc.ids, ed.doc.root,
            ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap();
        ed.commit(d);
        let target = *ed.doc.get(ed.doc.root).unwrap().children.first().unwrap();
        // give the node a 2x scale
        ed.commit(transform_nodes(&ed.doc, &[target], Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0])).unwrap());
        // now translate by (5,0) in world space
        ed.commit(transform_nodes(&ed.doc, &[target], Affine::translate(5.0, 0.0)).unwrap());
        let t = ed.doc.get(target).unwrap().transform;
        // point (1,0): scale first → (2,0), then translate → (7,0). Flipped order would give (12,0).
        assert_eq!(t.apply(1.0, 0.0), (7.0, 0.0));
    }

    #[test]
    fn delete_nodes_dedupes_ids() {
        let mut ed = Editor::new();
        let d = add_primitive(&mut ed.doc.ids, ed.doc.root,
            ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap();
        ed.commit(d);
        let id = *ed.doc.get(ed.doc.root).unwrap().children.first().unwrap();
        // pass the same id twice; should only emit one Remove
        ed.commit(delete_nodes(&ed.doc, &[id, id]).unwrap());
        assert!(ed.doc.get(id).is_none());
    }

    #[test]
    fn boolean_union_replaces_selection_with_single_path() {
        let mut ed = Editor::new();
        for _ in 0..2 {
            let d = add_primitive(&mut ed.doc.ids, ed.doc.root,
                ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap();
            ed.commit(d);
        }
        let sel: Vec<NodeId> = ed.doc.get(ed.doc.root).unwrap().children.clone();
        ed.boolean(&sel, geometry::BoolOp::Union).unwrap();
        let kids = &ed.doc.get(ed.doc.root).unwrap().children;
        assert_eq!(kids.len(), 1);
        assert!(matches!(ed.doc.get(kids[0]).unwrap().kind,
            NodeKind::Shape(ShapeKind::Path { .. })));
    }

    #[test]
    fn boolean_inputs_use_world_space_and_result_lands_in_parent_space() {
        let mut ed = Editor::new();
        // group translated (100,0); two 10x10 rects inside at local x=0 and x=5 (overlapping)
        let gid = ed.doc.ids.next();
        let mut group = Node::container(gid, NodeKind::Group);
        group.transform = Affine::translate(100.0, 0.0);
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root, node: group, index: 0 }]));
        let a = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: gid,
            node: Node::shape(a, ShapeKind::Rect { w: 10.0, h: 10.0 }), index: 0 }]));
        let b = ed.doc.ids.next();
        let mut nb = Node::shape(b, ShapeKind::Rect { w: 10.0, h: 10.0 });
        nb.transform = Affine::translate(5.0, 0.0);
        ed.commit(Delta(vec![NodeOp::Add { parent: gid, node: nb, index: 1 }]));

        ed.boolean(&[a, b], geometry::BoolOp::Union).unwrap();
        let kids = ed.doc.get(gid).unwrap().children.clone();
        assert_eq!(kids.len(), 1, "result should replace both inputs under the group");
        let result = ed.doc.get(kids[0]).unwrap();
        let d = match &result.kind {
            NodeKind::Shape(ShapeKind::Path { d }) => d.clone(),
            other => panic!("expected Path, got {other:?}"),
        };
        // Path data is in the group's LOCAL space: union of x 0..15 — not 100..115.
        let bounds = geometry::Path::from_svg(&d).unwrap().bounds();
        assert!((bounds.x - 0.0).abs() < 0.5, "x={} (world coords leaked in)", bounds.x);
        assert!((bounds.w - 15.0).abs() < 0.5, "w={}", bounds.w);
    }

    #[test]
    fn boolean_cross_depth_inputs_discriminate_world_vs_node_transform() {
        // root -> groupA(translate 50,0) -> groupB(translate 0,20) -> a (Rect 10x10, identity)
        //      -> groupA -> c (Rect 10x10, translate(-5,20))  [sibling of groupB, not of a]
        let mut ed = Editor::new();
        let ga = ed.doc.ids.next();
        let mut group_a = Node::container(ga, NodeKind::Group);
        group_a.transform = Affine::translate(50.0, 0.0);
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root, node: group_a, index: 0 }]));
        let gb = ed.doc.ids.next();
        let mut group_b = Node::container(gb, NodeKind::Group);
        group_b.transform = Affine::translate(0.0, 20.0);
        ed.commit(Delta(vec![NodeOp::Add { parent: ga, node: group_b, index: 0 }]));
        let a = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: gb,
            node: Node::shape(a, ShapeKind::Rect { w: 10.0, h: 10.0 }), index: 0 }]));
        let c = ed.doc.ids.next();
        let mut nc = Node::shape(c, ShapeKind::Rect { w: 10.0, h: 10.0 });
        nc.transform = Affine::translate(-5.0, 20.0);
        ed.commit(Delta(vec![NodeOp::Add { parent: ga, node: nc, index: 1 }]));

        // world(a) = translate(50,20); world(c) = translate(45,20)
        // union world bounds: x 45..60, y 20..30 (w=15, h=10)
        // dest_parent = groupB, dest_world = translate(50,20)
        // result in groupB-local: x -5..10 (w=15), y 0..10 (h=10)
        ed.boolean(&[a, c], geometry::BoolOp::Union).unwrap();
        let kids = ed.doc.get(gb).unwrap().children.clone();
        assert_eq!(kids.len(), 1, "result should land under groupB (parent of a)");
        let result = ed.doc.get(kids[0]).unwrap();
        let d = match &result.kind {
            NodeKind::Shape(ShapeKind::Path { d }) => d.clone(),
            other => panic!("expected Path, got {other:?}"),
        };
        let bounds = geometry::Path::from_svg(&d).unwrap().bounds();
        assert!((bounds.x - -5.0).abs() < 0.5, "x={} (expected -5)", bounds.x);
        assert!((bounds.w - 15.0).abs() < 0.5, "w={} (expected 15)", bounds.w);
        assert!((bounds.y - 0.0).abs() < 0.5, "y={} (expected 0)", bounds.y);
        assert!((bounds.h - 10.0).abs() < 0.5, "h={} (expected 10)", bounds.h);
    }

    #[test]
    fn boolean_op_dedupes_ids() {
        let mut ed = Editor::new();
        for _ in 0..2 {
            let d = add_primitive(&mut ed.doc.ids, ed.doc.root,
                ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap();
            ed.commit(d);
        }
        let sel: Vec<NodeId> = ed.doc.get(ed.doc.root).unwrap().children.clone();
        // duplicate the first id in the selection; must not double-Remove-panic in apply.
        ed.boolean(&[sel[0], sel[0], sel[1]], geometry::BoolOp::Union).unwrap();
        let kids = &ed.doc.get(ed.doc.root).unwrap().children;
        assert_eq!(kids.len(), 1);
    }

    #[test]
    fn boolean_union_is_undoable() {
        let mut ed = Editor::new();
        for _ in 0..2 {
            let d = add_primitive(&mut ed.doc.ids, ed.doc.root,
                ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap();
            ed.commit(d);
        }
        let sel: Vec<NodeId> = ed.doc.get(ed.doc.root).unwrap().children.clone();
        ed.boolean(&sel, geometry::BoolOp::Union).unwrap();
        ed.undo();
        let kids = &ed.doc.get(ed.doc.root).unwrap().children;
        assert_eq!(kids.len(), 2);
    }

    #[test]
    fn boolean_op_requires_at_least_two_ids() {
        let mut ed = Editor::new();
        let d = add_primitive(&mut ed.doc.ids, ed.doc.root,
            ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap();
        ed.commit(d);
        let id = *ed.doc.get(ed.doc.root).unwrap().children.first().unwrap();
        assert_eq!(boolean_op(&ed.doc, &[id], geometry::BoolOp::Union), Err(CmdError::EmptySelection));
    }

    #[test]
    fn boolean_op_unknown_id_is_not_found() {
        let mut ed = Editor::new();
        let d = add_primitive(&mut ed.doc.ids, ed.doc.root,
            ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap();
        ed.commit(d);
        let real = *ed.doc.get(ed.doc.root).unwrap().children.first().unwrap();
        let bogus = NodeId(9999);
        assert_eq!(boolean_op(&ed.doc, &[real, bogus], geometry::BoolOp::Union),
            Err(CmdError::NotFound));
    }

    /// Picks whatever font family is actually installed, instead of hardcoding "Helvetica"
    /// (macOS-only, absent on Linux CI). Returns None on a headless box with zero system faces.
    fn any_available_family() -> Option<String> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let name = db.faces().next().and_then(|f| f.families.first().map(|(name, _)| name.clone()));
        name
    }

    #[test]
    fn add_text_appends_a_path_shape_under_parent() {
        let mut ed = Editor::new();
        let parent = ed.doc.root;
        match any_available_family() {
            Some(family) => match ed.add_text(parent, &family, 10.0, "Hi") {
                Ok(_) => {
                    let kids = &ed.doc.get(parent).unwrap().children;
                    assert_eq!(kids.len(), 1);
                    assert!(matches!(ed.doc.get(kids[0]).unwrap().kind,
                        NodeKind::Shape(ShapeKind::Path { .. })));
                }
                Err(e) => panic!("unexpected error for family {family:?}: {e:?}"),
            },
            // headless CI with zero system fonts: assert the real Geometry(NoFont) path instead.
            None => assert_eq!(ed.add_text(parent, "Whatever", 10.0, "Hi"),
                Err(CmdError::Geometry(geometry::GeomError::NoFont.to_string()))),
        }
    }

    #[test]
    fn add_text_with_unknown_family_falls_back() {
        let mut ed = Editor::new();
        let parent = ed.doc.root;
        let result = ed.add_text(parent, "Definitely Not A Real Font Family 12345", 10.0, "Hi");
        match any_available_family() {
            Some(_) => {
                result.expect("fallback should have substituted a font");
                let kids = &ed.doc.get(parent).unwrap().children;
                assert_eq!(kids.len(), 1);
            }
            None => assert_eq!(result,
                Err(CmdError::Geometry(geometry::GeomError::NoFont.to_string()))),
        }
    }

    #[test]
    fn world_transform_composes_ancestors() {
        let mut ed = Editor::new();
        // group scaled 2x, child rect translated (5,0) locally
        let gid = ed.doc.ids.next();
        let mut group = Node::container(gid, NodeKind::Group);
        group.transform = Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root, node: group, index: 0 }]));
        let cid = ed.doc.ids.next();
        let mut child = Node::shape(cid, ShapeKind::Rect { w: 10.0, h: 10.0 });
        child.transform = Affine::translate(5.0, 0.0);
        ed.commit(Delta(vec![NodeOp::Add { parent: gid, node: child, index: 0 }]));
        // world = local.then(group): (0,0) -local-> (5,0) -group 2x-> (10,0)
        let w = world_transform(&ed.doc, cid).unwrap();
        assert_eq!(w.apply(0.0, 0.0), (10.0, 0.0));
        assert!(world_transform(&ed.doc, NodeId(999)).is_none());
    }

    #[test]
    fn transform_under_scaled_group_moves_exact_world_distance() {
        let mut ed = Editor::new();
        let gid = ed.doc.ids.next();
        let mut group = Node::container(gid, NodeKind::Group);
        group.transform = Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root, node: group, index: 0 }]));
        let cid = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: gid,
            node: Node::shape(cid, ShapeKind::Rect { w: 10.0, h: 10.0 }), index: 0 }]));

        let before_world = world_transform(&ed.doc, cid).unwrap().apply(0.0, 0.0);
        let d = transform_nodes(&ed.doc, &[cid], Affine::translate(10.0, 0.0)).unwrap();
        ed.commit(d);
        let after_world = world_transform(&ed.doc, cid).unwrap().apply(0.0, 0.0);
        // world moved exactly 10mm — NOT 20mm (the double-application bug this fixes)
        assert_eq!((after_world.0 - before_world.0, after_world.1 - before_world.1), (10.0, 0.0));
    }

    #[test]
    fn transform_applies_once_per_selected_subtree() {
        let mut ed = Editor::new();
        let gid = ed.doc.ids.next();
        let mut group = Node::container(gid, NodeKind::Group);
        group.transform = Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root, node: group, index: 0 }]));
        let cid = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: gid,
            node: Node::shape(cid, ShapeKind::Rect { w: 10.0, h: 10.0 }), index: 0 }]));

        let before_world = world_transform(&ed.doc, cid).unwrap().apply(0.0, 0.0);
        // group AND child selected — the child must ride along, not transform again
        ed.commit(transform_nodes(&ed.doc, &[gid, cid], Affine::translate(10.0, 0.0)).unwrap());
        let after_world = world_transform(&ed.doc, cid).unwrap().apply(0.0, 0.0);
        assert_eq!((after_world.0 - before_world.0, after_world.1 - before_world.1), (10.0, 0.0),
            "child world position must move by the gesture exactly once");
    }

    #[test]
    fn transform_skips_transitively_selected_ancestors() {
        let mut ed = Editor::new();
        // root → outer(selected) → mid(NOT selected) → leaf(selected)
        let outer = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root,
            node: Node::container(outer, NodeKind::Group), index: 0 }]));
        let mid = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: outer,
            node: Node::container(mid, NodeKind::Group), index: 0 }]));
        let leaf = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: mid,
            node: Node::shape(leaf, ShapeKind::Rect { w: 1.0, h: 1.0 }), index: 0 }]));

        let before_world = world_transform(&ed.doc, leaf).unwrap().apply(0.0, 0.0);
        ed.commit(transform_nodes(&ed.doc, &[outer, leaf], Affine::translate(10.0, 0.0)).unwrap());
        let after_world = world_transform(&ed.doc, leaf).unwrap().apply(0.0, 0.0);
        assert_eq!((after_world.0 - before_world.0, after_world.1 - before_world.1), (10.0, 0.0),
            "selected ancestor anywhere up the chain must suppress the leaf's own update");
    }

    #[test]
    fn transform_nodes_dedupes_ids() {
        let mut ed = Editor::new();
        let d = add_primitive(&mut ed.doc.ids, ed.doc.root,
            ShapeKind::Rect { w: 10.0, h: 10.0 }).unwrap();
        ed.commit(d);
        let id = *ed.doc.get(ed.doc.root).unwrap().children.first().unwrap();
        let d = transform_nodes(&ed.doc, &[id, id], Affine::translate(5.0, 0.0)).unwrap();
        assert_eq!(d.0.len(), 1, "duplicate ids must collapse to one Update");
        ed.commit(d);
        assert_eq!(ed.doc.get(id).unwrap().transform.apply(0.0, 0.0), (5.0, 0.0));
    }

    #[test]
    fn deleting_group_removes_descendants_and_undo_restores_structure() {
        let mut ed = Editor::new();
        let gid = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root,
            node: Node::container(gid, NodeKind::Group), index: 0 }]));
        let cid = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: gid,
            node: Node::shape(cid, ShapeKind::Rect { w: 1.0, h: 1.0 }), index: 0 }]));

        ed.commit(delete_nodes(&ed.doc, &[gid]).unwrap());
        assert!(ed.doc.get(gid).is_none());
        assert!(ed.doc.get(cid).is_none(), "descendants must not be orphaned");

        ed.undo();
        assert!(ed.doc.get(gid).is_some());
        assert!(ed.doc.get(cid).is_some());
        assert_eq!(ed.doc.get(gid).unwrap().children, vec![cid]);
    }

    #[test]
    fn selecting_group_and_child_together_does_not_panic() {
        let mut ed = Editor::new();
        let gid = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: ed.doc.root,
            node: Node::container(gid, NodeKind::Group), index: 0 }]));
        let cid = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add { parent: gid,
            node: Node::shape(cid, ShapeKind::Rect { w: 1.0, h: 1.0 }), index: 0 }]));
        // group first, child second — the ordering that panicked before
        ed.commit(delete_nodes(&ed.doc, &[gid, cid]).unwrap());
        assert!(ed.doc.get(gid).is_none());
        assert!(ed.doc.get(cid).is_none());
    }

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

        let d = set_material_preset(&doc, &[shape_id], PresetAssignment::Unassigned).unwrap();
        doc.apply(d);
        assert_eq!(doc.get(shape_id).unwrap().material_preset, PresetAssignment::Unassigned);
        let d = set_material_preset(&doc, &[shape_id], PresetAssignment::Inherit).unwrap();
        doc.apply(d);
        assert_eq!(doc.get(shape_id).unwrap().material_preset, PresetAssignment::Inherit);
    }

    /// An empty id names no material, so it is refused at the edit rather than at the cut.
    /// Codex found the path this closes: `PassKey` parses `preset:` (its grammar has to be total
    /// in both languages), so an empty assignment would key a pass, the row would carry an empty
    /// preset id, and `prepare_cut` would refuse it — correct, but only after the operator had
    /// already committed the edit and pressed Cut.
    #[test]
    fn set_material_preset_refuses_an_empty_id() {
        let mut doc = Document::new();
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        assert_eq!(
            set_material_preset(&doc, &[shape_id], PresetAssignment::Preset(String::new())),
            Err(CmdError::EmptyPresetId)
        );
        // ...and a real id still lands.
        assert!(set_material_preset(&doc, &[shape_id],
            PresetAssignment::Preset("cameo5-htv".into())).is_ok());
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
        assert_eq!(set_material_preset(&doc, &[NodeId(9999)], PresetAssignment::Inherit),
            Err(CmdError::NotFound));
    }
}
