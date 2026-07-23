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
}
