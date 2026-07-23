// SPDX-License-Identifier: GPL-3.0-or-later
use crate::delta::{Document, Delta};

pub struct Editor {
    pub doc: Document,
    undo_stack: Vec<Delta>,   // each entry is an inverse delta
    redo_stack: Vec<Delta>,   // each entry is a forward delta
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
        let redo = self.doc.apply(inverse.clone());
        self.redo_stack.push(redo);
        Some(inverse)
    }
    pub fn redo(&mut self) -> Option<Delta> {
        let forward = self.redo_stack.pop()?;
        let inverse = self.doc.apply(forward.clone());
        self.undo_stack.push(inverse);
        Some(forward)
    }
}

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
        assert!(matches!(undo.0[0], NodeOp::Remove { .. }));

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
