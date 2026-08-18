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

/// Which `MaterialPreset` a Node's geometry is cut with, or where to look for one.
///
/// A sibling of `cut_line_type`, and not on `Style`, for the reason #68 settled: production
/// intent is not paint.
///
/// Three states rather than `Option<String>`, because the two-state spelling cannot say
/// "deliberately no material, do not inherit". With absence meaning inherit, a shape inside an
/// HTV Layer could never reach the no-preset pass — a pass that exists and resolves to the
/// operator's own settings. `cutplan::plan_passes_with` resolves the chain; nothing stores a
/// resolved value, so reparenting cannot leave a stale one.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "state", content = "id", rename_all = "kebab-case")]
pub enum PresetAssignment {
    /// Take the nearest ancestor's assignment; no ancestor means no material.
    #[default]
    Inherit,
    /// No material, whatever any ancestor says.
    Unassigned,
    /// This `MaterialPreset::id`. Never validated here: presets are machine-scoped and a
    /// user entry can be deleted, so an id that resolves to nothing is a real state.
    Preset(String),
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(from = "NodeWire")]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub transform: Affine,   // relative to parent
    pub style: Style,
    pub cut_line_type: CutLineType,
    pub material_preset: PresetAssignment,
    pub children: Vec<NodeId>,
}
impl Node {
    pub fn shape(id: NodeId, kind: ShapeKind) -> Node {
        Node { id, kind: NodeKind::Shape(kind), transform: Affine::identity(),
               style: Style::default(), cut_line_type: CutLineType::Cut,
               material_preset: PresetAssignment::Inherit, children: vec![] }
    }
    pub fn container(id: NodeId, kind: NodeKind) -> Node {
        Node { id, kind, transform: Affine::identity(),
               style: Style::default(), cut_line_type: CutLineType::Cut,
               material_preset: PresetAssignment::Inherit, children: vec![] }
    }
}

/// `Node` as it may appear on disk. `manifest.json` does carry a schema version now (`fileio`
/// wraps the document in a versioned envelope), but a `Node` is also deserialized from
/// `Document::snapshot_json()` across IPC, which carries none — so on those paths a missing
/// field is still the only migration signal there is, and `#[serde(default)]` cannot serve as
/// one twice over: it cannot tell an absent field from an explicit `Cut`, and it cannot see the
/// node's stroke, which is the only thing that says what an old document used to cut.
///
/// An explicit `"cut_line_type": null` is treated as absence, which serde gives for free and
/// which is deliberate rather than incidental. Nothing this workspace writes can produce it —
/// `Serialize` is derived and always writes a concrete value — so it only ever arrives from a
/// damaged or third-party file, and for such a file deriving from the stroke reproduces
/// exactly the pre-#144 behaviour, which is the same answer absence gets. Refusing it instead
/// would fail closed: `load_project` would refuse the whole document over one null field,
/// turning a recoverable file into one the operator cannot open at all.
///
/// This sits on `Node` rather than in `fileio`'s versioned migration table, where the
/// legacy-machine-id step lives, because a `Node` is also deserialized through
/// `Document::snapshot_json` and across IPC; confining it to project load would leave those
/// paths to guess.
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
    /// A plain `#[serde(default)]`, unlike `cut_line_type` above: a document written before
    /// this field existed had no way to assign a material, so absence *is* `Inherit`. There
    /// is nothing to derive and no old behaviour to preserve.
    #[serde(default)]
    material_preset: Option<PresetAssignment>,
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
               cut_line_type, material_preset: w.material_preset.unwrap_or_default(),
               children: w.children }
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

    /// A new Node inherits. There is no import default to argue about: a material is the
    /// operator's choice per shape or per Layer, and inheriting is the state that lets a
    /// Layer's choice reach the shapes under it.
    #[test]
    fn a_new_node_inherits_its_material() {
        let mut ids = IdGen::default();
        assert_eq!(Node::shape(ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 }).material_preset,
            PresetAssignment::Inherit);
        assert_eq!(Node::container(ids.next(), NodeKind::Group).material_preset,
            PresetAssignment::Inherit);
    }

    /// Three states, and the wire form each takes. `Unassigned` is the one the two-state
    /// spelling could not express: a shape deliberately carrying no material *inside* an
    /// assigned Layer, which is a pass an operator can otherwise never reach.
    #[test]
    fn a_material_assignment_round_trips_in_all_three_states() {
        for (value, json) in [
            (PresetAssignment::Inherit, r#"{"state":"inherit"}"#),
            (PresetAssignment::Unassigned, r#"{"state":"unassigned"}"#),
            (PresetAssignment::Preset("cameo5-htv".into()), r#"{"state":"preset","id":"cameo5-htv"}"#),
        ] {
            assert_eq!(serde_json::to_string(&value).unwrap(), json);
            assert_eq!(serde_json::from_str::<PresetAssignment>(json).unwrap(), value);
        }
    }

    /// A document written before the field existed had no way to assign a material, so
    /// absence means inherit — and an explicit `null` means the same, which serde gives for
    /// free and which is deliberate: nothing this workspace writes can produce one, so it
    /// only ever arrives from a hand-edited file.
    #[test]
    fn a_node_saved_without_a_material_assignment_inherits() {
        let json = r#"{"id":7,"kind":{"Shape":{"Rect":{"w":1.0,"h":1.0}}},
                       "transform":[1.0,0.0,0.0,1.0,0.0,0.0],
                       "style":{"stroke":255,"fill":null},
                       "cut_line_type":"Cut","children":[]}"#;
        let node: Node = serde_json::from_str(json).unwrap();
        assert_eq!(node.material_preset, PresetAssignment::Inherit);
        assert_eq!(node.cut_line_type, CutLineType::Cut, "premise: the other attribute still decodes");

        let nulled = json.replace(r#""cut_line_type":"Cut""#, r#""cut_line_type":"Cut","material_preset":null"#);
        assert_eq!(serde_json::from_str::<Node>(&nulled).unwrap().material_preset,
            PresetAssignment::Inherit);
    }

    /// Written on every save, so the field stops being absent the first time a document is
    /// written by this version and is never ambiguous again.
    #[test]
    fn a_material_assignment_is_always_written() {
        let mut node = Node::shape(NodeId(1), ShapeKind::Rect { w: 1.0, h: 1.0 });
        node.material_preset = PresetAssignment::Preset("cameo5-htv".into());
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains(r#""material_preset":{"state":"preset","id":"cameo5-htv"}"#), "{json}");
        assert_eq!(serde_json::from_str::<Node>(&json).unwrap(), node);
    }

    /// A document written before the attribute existed must cut exactly what it cut then.
    /// The cases are the whole of the old rule (`plan_passes`' stroke filter): a stroke, no
    /// stroke, a stroke nobody can see, and a stroke barely anybody can see — the last
    /// because the rule turns on alpha being non-zero, not on the stroke being opaque, and
    /// without it a migration tightened to "fully opaque" would mark every partially
    /// transparent legacy stroke `NoCut` with these tests still green.
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
        // 0xFF000001 — red at alpha 1. Invisible in practice, cut all the same, because the
        // old predicate asked only whether the alpha byte was non-zero.
        assert_eq!(node("4278190081").cut_line_type, CutLineType::Cut);
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

    /// An explicit `null` is treated as absence, and that is a decision rather than an
    /// accident of serde. Nothing here writes one — `Serialize` always emits a concrete
    /// value — so it only arrives from a damaged or third-party file, where deriving from
    /// the stroke reproduces the pre-#144 behaviour, which is the same answer absence gets.
    /// Refusing it would fail closed and make `load_project` reject a whole document over
    /// one null field. Pinned because nothing else states which way that goes.
    #[test]
    fn an_explicit_null_migrates_the_same_way_an_absent_field_does() {
        let node = |field: &str| -> Node {
            let json = format!(
                r#"{{"id":7,"kind":{{"Shape":{{"Rect":{{"w":1.0,"h":1.0}}}}}},
                     "transform":[1.0,0.0,0.0,1.0,0.0,0.0],
                     "style":{{"stroke":255,"fill":null}},{field}"children":[]}}"#
            );
            serde_json::from_str(&json).unwrap()
        };
        assert_eq!(node(r#""cut_line_type":null,"#).cut_line_type, node("").cut_line_type);
        assert_eq!(node(r#""cut_line_type":null,"#).cut_line_type, CutLineType::Cut,
            "derived from the stroke, exactly as the absent field is");
    }
}
