// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use document::{shape_outline, Document, NodeId};
use geometry::{Affine, Point, Polyline};
use serde::{Deserialize, Serialize};

/// A single shape's flattened, world-transformed outline, ready to cut.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct PlannedShape { pub node_id: NodeId, pub polylines: Vec<Polyline> }

/// All shapes sharing one stroke color (0xRRGGBBAA), cut together as one pass.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ColorPass { pub color: Option<u32>, pub shapes: Vec<PlannedShape> }

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct PlannedCut { pub passes: Vec<ColorPass>, pub skipped_no_stroke: usize, pub doc_revision: u64 }

#[derive(Debug, PartialEq)]
pub enum PlanError { BadShape(NodeId, String), MissingNode(NodeId), CycleDetected }

/// Hash of the document's JSON snapshot — cheap staleness check for a previously
/// computed `PlannedCut` (recompute if this no longer matches `doc_revision(doc)`).
pub fn doc_revision(doc: &Document) -> u64 {
    let mut hasher = DefaultHasher::new();
    doc.snapshot_json().hash(&mut hasher);
    hasher.finish()
}

/// Walk the document in preorder from `doc.root`, group shapes by full stroke RGBA
/// (`None` or alpha-0 strokes are skipped, not cut), and flatten each shape's outline
/// under its accumulated world transform. Iterative (explicit stack) so depth is not
/// bounded by the Rust call stack; a `visited` set catches cycles in malformed docs.
pub fn plan_passes(doc: &Document) -> Result<PlannedCut, PlanError> {
    let mut visited: HashSet<NodeId> = HashSet::new();
    let mut stack: Vec<(NodeId, Affine)> = vec![(doc.root, Affine::identity())];
    let mut passes: Vec<ColorPass> = vec![];
    let mut skipped_no_stroke = 0usize;

    while let Some((id, parent_world)) = stack.pop() {
        if !visited.insert(id) {
            return Err(PlanError::CycleDetected);
        }
        let node = doc.get(id).ok_or(PlanError::MissingNode(id))?;
        let world = node.transform.then(&parent_world);

        match shape_outline(node).map_err(|e| PlanError::BadShape(id, e))? {
            None => {
                // Container: descend, pushing in reverse so preorder visits children left-to-right.
                for &child in node.children.iter().rev() {
                    stack.push((child, world));
                }
            }
            Some(path) => {
                // 0-alpha counts as "no stroke" — nothing to cut, same as None.
                match node.style.stroke.filter(|c| c & 0xFF != 0) {
                    None => skipped_no_stroke += 1,
                    Some(color) => {
                        let polylines = path.transformed(&world).flatten(0.1);
                        let shape = PlannedShape { node_id: id, polylines };
                        match passes.iter_mut().find(|p| p.color == Some(color)) {
                            Some(pass) => pass.shapes.push(shape),
                            None => passes.push(ColorPass { color: Some(color), shapes: vec![shape] }),
                        }
                    }
                }
            }
        }
    }

    Ok(PlannedCut { passes, skipped_no_stroke, doc_revision: doc_revision(doc) })
}

/// Travel (non-cutting) moves needed to visit every shape across `configured` passes,
/// in the given order: end of one shape's last polyline -> start of the next shape's
/// first polyline. `configured` lets the caller reorder/subset passes (e.g. by machine
/// color-change cost) independently of `plan_passes`' first-seen grouping order.
pub fn travel_moves(configured: &[&ColorPass]) -> Vec<(Point, Point)> {
    let mut moves = vec![];
    let mut prev_end: Option<Point> = None;
    for pass in configured {
        for shape in &pass.shapes {
            let start = shape.polylines.first().and_then(|p| p.first()).copied();
            let end = shape.polylines.last().and_then(|p| p.last()).copied();
            if let (Some(prev), Some(start)) = (prev_end, start) {
                moves.push((prev, start));
            }
            if let Some(end) = end {
                prev_end = Some(end);
            }
        }
    }
    moves
}

#[cfg(test)]
mod tests {
    use super::*;
    use document::history::Editor;
    use document::{Delta, Node, NodeKind, NodeOp, ShapeKind, Style};

    fn with_stroke(mut node: Node, stroke: Option<u32>) -> Node {
        node.style = Style { stroke, fill: None };
        node
    }

    /// Picks whatever font family is actually installed, instead of hardcoding one
    /// (macOS-only). Returns None on a headless CI box with zero system faces.
    fn any_available_family() -> Option<String> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let name = db.faces().next().and_then(|f| f.families.first().map(|(name, _)| name.clone()));
        name
    }

    #[test]
    fn plans_group_by_stroke_rgba_with_single_traversal_transforms() {
        let mut ed = Editor::new();
        let root = ed.doc.root;
        const RED: u32 = 0xFF0000FF;
        const BLUE: u32 = 0x0000FFFF;

        // group translate(10,0) containing a red rect
        let gid = ed.doc.ids.next();
        let mut group = Node::container(gid, NodeKind::Group);
        group.transform = Affine::translate(10.0, 0.0);
        ed.commit(Delta(vec![NodeOp::Add { parent: root, node: group, index: usize::MAX }]));
        let grouped_child = ed.doc.ids.next();
        let node = with_stroke(Node::shape(grouped_child, ShapeKind::Rect { w: 5.0, h: 5.0 }), Some(RED));
        ed.commit(Delta(vec![NodeOp::Add { parent: gid, node, index: usize::MAX }]));

        // root-level red rect at origin
        let root_rect = ed.doc.ids.next();
        let node = with_stroke(Node::shape(root_rect, ShapeKind::Rect { w: 5.0, h: 5.0 }), Some(RED));
        ed.commit(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));

        // blue ellipse
        let ellipse = ed.doc.ids.next();
        let node = with_stroke(Node::shape(ellipse, ShapeKind::Ellipse { rx: 3.0, ry: 3.0 }), Some(BLUE));
        ed.commit(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));

        // stroke-None rect: excluded from every pass
        let no_stroke = ed.doc.ids.next();
        let node = with_stroke(Node::shape(no_stroke, ShapeKind::Rect { w: 5.0, h: 5.0 }), None);
        ed.commit(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));

        let planned = plan_passes(&ed.doc).unwrap();
        assert_eq!(planned.passes.len(), 2, "red + blue; None excluded");
        assert_eq!(planned.skipped_no_stroke, 1);
        let red = &planned.passes[0]; // first-seen order
        assert_eq!(red.color, Some(RED));
        assert_eq!(red.shapes.len(), 2);
        // the grouped child's polyline reflects the group's translate (world transform applied)
        assert!(red.shapes.iter().any(|s| s.polylines[0][0].x >= 10.0));
        assert_eq!(planned.passes[1].color, Some(BLUE));
    }

    #[test]
    fn text_plans_as_glyph_outlines_or_typed_error() {
        let mut ed = Editor::new();
        let root = ed.doc.root;

        if let Some(family) = any_available_family() {
            let id = ed.doc.ids.next();
            let node = Node::shape(id, ShapeKind::Text { family, size_mm: 10.0, text: "Hi".into() });
            ed.commit(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));
            let planned = plan_passes(&ed.doc).unwrap();
            assert_eq!(planned.passes.len(), 1);
            assert!(!planned.passes[0].shapes[0].polylines.is_empty());
        }

        // bogus family is a typed error regardless of what's installed
        let bad_id = ed.doc.ids.next();
        let node = Node::shape(bad_id, ShapeKind::Text {
            family: "Definitely Not A Real Font Family 12345".into(), size_mm: 10.0, text: "Hi".into(),
        });
        let mut bad_doc = ed.doc.clone();
        bad_doc.apply(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));
        assert_eq!(plan_passes(&bad_doc),
            Err(PlanError::BadShape(bad_id, format!("{:?}", geometry::GeomError::NoFont))));
    }

    #[test]
    fn stale_revision_detectable() {
        let mut ed = Editor::new();
        let planned = plan_passes(&ed.doc).unwrap();
        let id = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add {
            parent: ed.doc.root, index: usize::MAX,
            node: Node::shape(id, ShapeKind::Rect { w: 1.0, h: 1.0 }),
        }]));
        assert_ne!(planned.doc_revision, doc_revision(&ed.doc));
    }

    fn shape(id: u64, polylines: Vec<Polyline>) -> PlannedShape {
        PlannedShape { node_id: NodeId(id), polylines }
    }
    fn pt(x: f64, y: f64) -> Point { Point { x, y } }

    #[test]
    fn travel_moves_follow_configured_order() {
        let pass_a = ColorPass {
            color: Some(1),
            shapes: vec![
                shape(1, vec![vec![pt(0.0, 0.0), pt(1.0, 0.0)]]),
                shape(2, vec![vec![pt(2.0, 0.0), pt(3.0, 0.0)]]),
            ],
        };
        let pass_b = ColorPass {
            color: Some(2),
            shapes: vec![shape(3, vec![vec![pt(10.0, 0.0), pt(11.0, 0.0)]])],
        };
        // reversed order: pass_b before pass_a
        let moves = travel_moves(&[&pass_b, &pass_a]);
        assert_eq!(moves, vec![
            (pt(11.0, 0.0), pt(0.0, 0.0)), // end of pass_b's only shape -> start of pass_a's first shape
            (pt(1.0, 0.0), pt(2.0, 0.0)),  // end of pass_a's first shape -> start of pass_a's second shape
        ]);
    }
}
