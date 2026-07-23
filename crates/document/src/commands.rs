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
/// The new transform applies the node's existing transform first, then the world-space transform.
pub fn transform_nodes(doc: &Document, ids: &[NodeId], m: Affine) -> Result<Delta, CmdError> {
    if ids.is_empty() { return Err(CmdError::EmptySelection); }
    let mut ops = vec![];
    for &id in ids {
        let before = doc.get(id).ok_or(CmdError::NotFound)?.clone();
        let mut after = before.clone();
        after.transform = before.transform.then(&m);
        ops.push(NodeOp::Update { id, before, after });
    }
    Ok(Delta(ops))
}

pub fn delete_nodes(doc: &Document, ids: &[NodeId]) -> Result<Delta, CmdError> {
    if ids.is_empty() { return Err(CmdError::EmptySelection); }
    let mut ops = vec![];
    let mut seen = HashSet::new();
    for &id in ids {
        if !seen.insert(id) { continue; } // skip duplicates
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

/// Shape's outline in its own local space, in mm, matching `Rect { x:0, y:0, w, h }` bounds
/// convention (an ellipse of radii rx,ry centered at (rx,ry) has the same 0,0-origin bounds).
fn shape_to_path(node: &Node) -> Option<Path> {
    let p = match &node.kind {
        NodeKind::Shape(ShapeKind::Rect { w, h }) => rect_path(0.0, 0.0, *w, *h),
        NodeKind::Shape(ShapeKind::Ellipse { rx, ry }) => ellipse_path(*rx, *ry, *rx, *ry),
        NodeKind::Shape(ShapeKind::Path { d }) => Path::from_svg(d).ok()?,
        _ => return None,
    };
    Some(p.transformed(&node.transform))
}

/// Replace `ids` (>= 2 shape nodes) with a single Path node holding the boolean-op result.
/// The result is appended under the parent of `ids[0]`. Mints `NodeId(u64::MAX)` as a
/// placeholder for the new node's id — `Editor::boolean` overwrites it before commit.
pub fn boolean_op(doc: &Document, ids: &[NodeId], op: BoolOp) -> Result<Delta, CmdError> {
    if ids.len() < 2 { return Err(CmdError::EmptySelection); }
    let mut paths = vec![];
    for &id in ids {
        paths.push(shape_to_path(doc.get(id).ok_or(CmdError::NotFound)?).ok_or(CmdError::NotFound)?);
    }
    let result = boolean(op, &paths).map_err(|e| CmdError::Geometry(format!("{e:?}")))?;
    let parent = parent_of(doc, ids[0]).ok_or(CmdError::NotFound)?;
    let mut ops: Vec<NodeOp> = ids.iter()
        .map(|&id| NodeOp::Remove { parent: parent_of(doc, id).unwrap(), id })
        .collect();
    ops.push(NodeOp::Add {
        parent,
        node: Node::shape(NodeId(u64::MAX), ShapeKind::Path { d: result.to_svg() }),
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
}
