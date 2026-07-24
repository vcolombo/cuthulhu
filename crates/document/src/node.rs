// SPDX-License-Identifier: GPL-3.0-or-later
use geometry::Affine;
use serde::{Serialize, Deserialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct NodeId(pub u64);

#[derive(Default, Clone, PartialEq, Debug, Serialize, Deserialize)]
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
