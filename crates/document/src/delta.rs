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

#[cfg(test)]
mod tests {
    use super::*;

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
