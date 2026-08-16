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
}
