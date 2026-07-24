// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashSet;
use geometry::{boolean, ellipse_path, rect_path, text_to_path, Affine, BoolOp, Path};
use crate::{node::*, delta::*};

#[derive(Debug, PartialEq)]
pub enum CmdError { NotFound, EmptySelection, Geometry(String) }

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
    let mut ops = vec![];
    for &id in ids {
        let before = doc.get(id).ok_or(CmdError::NotFound)?.clone();
        // Convert the world-space matrix into this node's parent space so that
        // new_world = old_world.then(m) holds under transformed ancestors:
        // new_local = old_local.then(pw).then(m).then(pw⁻¹)
        let pw = match parent_of(doc, id) {
            Some(pid) => world_transform(doc, pid).ok_or(CmdError::NotFound)?,
            None => Affine::identity(),
        };
        let pw_inv = pw.inverse()
            .ok_or_else(|| CmdError::Geometry("degenerate ancestor transform".into()))?;
        let mut after = before.clone();
        after.transform = before.transform.then(&pw).then(&m).then(&pw_inv);
        ops.push(NodeOp::Update { id, before, after });
    }
    Ok(Delta(ops))
}

pub fn delete_nodes(doc: &Document, ids: &[NodeId]) -> Result<Delta, CmdError> {
    if ids.is_empty() { return Err(CmdError::EmptySelection); }
    let selected: HashSet<NodeId> = ids.iter().copied().collect();
    let has_selected_ancestor = |id: NodeId| {
        let mut cur = id;
        while let Some(pid) = parent_of(doc, cur) {
            if selected.contains(&pid) { return true; }
            cur = pid;
        }
        false
    };
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
        if !seen.insert(id) || has_selected_ancestor(id) { continue; }
        let parent = parent_of(doc, id).ok_or(CmdError::NotFound)?;
        push_subtree(doc, id, parent, &mut ops)?;
    }
    if ops.is_empty() { return Err(CmdError::EmptySelection); }
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
/// centered at (rx,ry) has the same 0,0-origin bounds).
fn local_shape_path(node: &Node) -> Option<Path> {
    match &node.kind {
        NodeKind::Shape(ShapeKind::Rect { w, h }) => Some(rect_path(0.0, 0.0, *w, *h)),
        NodeKind::Shape(ShapeKind::Ellipse { rx, ry }) => Some(ellipse_path(*rx, *ry, *rx, *ry)),
        NodeKind::Shape(ShapeKind::Path { d }) => Path::from_svg(d).ok(),
        _ => None,
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
    let mut paths = vec![];
    for &id in &ids {
        let node = doc.get(id).ok_or(CmdError::NotFound)?;
        let local = local_shape_path(node).ok_or(CmdError::NotFound)?;
        let world = world_transform(doc, id).ok_or(CmdError::NotFound)?;
        paths.push(local.transformed(&world));
    }
    let result = boolean(op, &paths).map_err(|e| CmdError::Geometry(format!("{e:?}")))?;
    let dest_parent = parent_of(doc, ids[0]).ok_or(CmdError::NotFound)?;
    let dest_world = world_transform(doc, dest_parent).ok_or(CmdError::NotFound)?;
    let dest_inv = dest_world.inverse()
        .ok_or_else(|| CmdError::Geometry("degenerate destination transform".into()))?;
    let result_local = result.transformed(&dest_inv);
    let mut ops: Vec<NodeOp> = ids.iter()
        .map(|&id| NodeOp::Remove { parent: parent_of(doc, id).unwrap(), id })
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
    let path = text_to_path(family, size_mm, text).map_err(|e| CmdError::Geometry(format!("{e:?}")))?;
    let node = Node::shape(NodeId(u64::MAX), ShapeKind::Path { d: path.to_svg() });
    Ok(Delta(vec![NodeOp::Add { parent, node, index: usize::MAX }]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Editor;
    use geometry::Affine;

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
                Err(CmdError::Geometry(format!("{:?}", geometry::GeomError::NoFont)))),
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
}
