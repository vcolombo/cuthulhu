// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use document::{shape_outline, CutLineType, Document, NodeId, NodeKind, Style};
use geometry::{Affine, Point, Polyline};
use serde::{Deserialize, Serialize};

/// A single shape's flattened, world-transformed outline, ready to cut.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct PlannedShape { pub node_id: NodeId, pub polylines: Vec<Polyline> }

/// All shapes cut together as one pass, keyed on the colour they share (0xRRGGBBAA) —
/// their stroke where they have a visible one, else their fill. `None` is a pass of shapes
/// with no visible paint at all.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ColorPass { pub color: Option<u32>, pub shapes: Vec<PlannedShape> }

/// Every `ColorPass` a document contains, in first-seen order — an inventory of
/// what *could* be cut. Nothing here is selected, configured or checked; that is
/// `plan_cut`'s job, and what it returns is a `CutPlan`.
///
/// This is everything a cut needs to know about the document, which is why
/// `plan_cut` takes one of these rather than a `Document`: the geometry that
/// gets validated is the geometry that gets cut, and nobody plans twice.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DocumentPasses {
    pub passes: Vec<ColorPass>,
    pub skipped_not_cut: usize,
    pub doc_revision: u64,
    /// The machine the document targets, if it names one. Carried here because
    /// preflight checks it and `plan_cut` no longer sees the `Document`.
    pub machine_id: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum PlanError { BadShape(NodeId, String), MissingNode(NodeId), CycleDetected }
impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // `shape #3`, not a name: `plan_passes` walks ids, and a lookup from id to
            // whatever the UI calls the shape is not something it has (same limit as
            // `PreflightError`). The payload is `shape_outline`'s sentence about the
            // shape itself, so it reads on from the id.
            PlanError::BadShape(node, message) => write!(f, "shape #{}: {message}", node.0),
            PlanError::MissingNode(node) =>
                write!(f, "shape #{} is referenced by the document but missing from it", node.0),
            PlanError::CycleDetected =>
                write!(f, "the document's shapes contain each other in a loop"),
        }
    }
}
impl std::error::Error for PlanError {}

/// Hash of the document's JSON snapshot — cheap staleness check for a previously
/// computed `DocumentPasses` (recompute if this no longer matches `doc_revision(doc)`).
pub fn doc_revision(doc: &Document) -> u64 {
    let mut hasher = DefaultHasher::new();
    doc.snapshot_json().hash(&mut hasher);
    hasher.finish()
}

/// The colour a cut shape's pass is keyed on: its stroke if it has a visible one, else its
/// fill. Alpha-0 counts as absent in both, exactly as a 0-alpha stroke did when the stroke
/// decided cuttability.
///
/// A fallback is needed because a shape with no stroke can be cut since #144 — traced and
/// fill-only art is the common case — and a pass with no colour at all is something an
/// operator cannot recognise in a pass list. `None` remains possible for a shape with no
/// visible paint whatsoever.
fn pass_key(style: &Style) -> Option<u32> {
    style.stroke.filter(|c| c & 0xFF != 0).or(style.fill.filter(|c| c & 0xFF != 0))
}

/// Walk the document in preorder from `doc.root`, group the shapes whose `CutLineType` is
/// `Cut` by the colour `pass_key` gives them, and flatten each shape's outline under its
/// accumulated world transform. A `NoCut` shape is counted, not cut. Iterative (explicit
/// stack) so depth is not bounded by the Rust call stack; a `visited` set catches cycles in
/// malformed docs.
pub fn plan_passes(doc: &Document) -> Result<DocumentPasses, PlanError> {
    let mut visited: HashSet<NodeId> = HashSet::new();
    let mut stack: Vec<(NodeId, Affine)> = vec![(doc.root, Affine::identity())];
    let mut passes: Vec<ColorPass> = vec![];
    let mut skipped_not_cut = 0usize;

    while let Some((id, parent_world)) = stack.pop() {
        if !visited.insert(id) {
            return Err(PlanError::CycleDetected);
        }
        let node = doc.get(id).ok_or(PlanError::MissingNode(id))?;
        let world = node.transform.then(&parent_world);

        // Descend on the node's own kind, not on `shape_outline` returning `None`. The two
        // agree, but reading it from `NodeKind` is what lets the outline stay unresolved
        // until the shape is known to be cut — resolving first meant a font or path-data
        // failure on a shape nobody would cut refused the whole plan (#139).
        match &node.kind {
            NodeKind::Group | NodeKind::Layer => {
                // Push in reverse so preorder visits children left-to-right.
                for &child in node.children.iter().rev() {
                    stack.push((child, world));
                }
            }
            NodeKind::Shape(_) => {
                // The predicate #144 moved here off the stroke. The *ordering* is #139's and
                // must not move with it: the outline stays unresolved until the shape is
                // known to be cut, so a font or path-data failure on a shape nobody cuts
                // cannot refuse the whole plan.
                match node.cut_line_type {
                    CutLineType::NoCut => skipped_not_cut += 1,
                    CutLineType::Cut => {
                        // `None` here is `shape_outline`'s container signal, which `NodeKind`
                        // has already ruled out, so no `ShapeKind` reaches this today. A new
                        // one added without its own arm there would fall into its catch-all
                        // and land here — refuse rather than skip, because this branch is
                        // past the cut filter: the shape *is* being cut, and quietly dropping
                        // it would send a partial plan to the blade. Same reason a shape whose
                        // outline fails to parse refuses instead of being skipped.
                        let Some(path) = shape_outline(node).map_err(|e| PlanError::BadShape(id, e))?
                        else {
                            return Err(PlanError::BadShape(
                                id, "this kind of shape cannot be resolved to an outline".into()));
                        };
                        let polylines = path.transformed(&world).flatten(0.1);
                        let shape = PlannedShape { node_id: id, polylines };
                        let color = pass_key(&node.style);
                        match passes.iter_mut().find(|p| p.color == color) {
                            Some(pass) => pass.shapes.push(shape),
                            None => passes.push(ColorPass { color, shapes: vec![shape] }),
                        }
                    }
                }
            }
        }
    }

    Ok(DocumentPasses {
        passes,
        skipped_not_cut,
        doc_revision: doc_revision(doc),
        machine_id: doc.machine.as_ref().map(|m| m.id.clone()),
    })
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
    use document::{CutLineType, Delta, Node, NodeKind, NodeOp, ShapeKind, Style};

    /// The whole table at once: a new variant fails to compile the match in `Display`,
    /// and a reworded one fails here. These strings are what an operator reads — all
    /// three used to arrive as `plan: MissingNode(NodeId(3))`, which is why this type
    /// gained `Display` at all (#91).
    #[test]
    fn every_plan_refusal_has_a_sentence() {
        let cases: Vec<(PlanError, &str)> = vec![
            (
                PlanError::BadShape(NodeId(3), geometry::GeomError::NoFont.to_string()),
                "shape #3: no fonts are installed on this system",
            ),
            (
                PlanError::BadShape(NodeId(5), geometry::GeomError::BadFont.to_string()),
                "shape #5: a font was found, but its file could not be read",
            ),
            (
                PlanError::BadShape(NodeId(6), geometry::GeomError::NoGlyphs.to_string()),
                "shape #6: the chosen font cannot draw any of this text",
            ),
            (
                PlanError::MissingNode(NodeId(4)),
                "shape #4 is referenced by the document but missing from it",
            ),
            (
                PlanError::CycleDetected,
                "the document's shapes contain each other in a loop",
            ),
        ];
        for (error, sentence) in cases {
            assert_eq!(error.to_string(), sentence, "{error:?}");
        }
    }

    fn with_stroke(mut node: Node, stroke: Option<u32>) -> Node {
        node.style = Style { stroke, fill: None };
        node
    }

    /// Mark a node not-cut. Since #144 a strokeless shape is cut by default, so a test that
    /// wants a skipped shape has to say so rather than leaving the stroke off.
    fn with_no_cut(mut node: document::Node) -> document::Node {
        node.cut_line_type = CutLineType::NoCut;
        node
    }

    /// Unicode noncharacters. Permanently unassigned, so a face drawing them is the
    /// exception rather than the rule — see `family_that_cannot_draw` for the exception.
    const UNDRAWABLE: &str = "\u{FDD0}\u{FDD1}";

    /// A family installed here that cannot draw `text`, or `None` if every face can.
    ///
    /// Searched rather than picked, because picking makes the caller's premise depend on
    /// font enumeration order: this box carries 2966 faces of which exactly one, macOS's
    /// `.LastResort`, maps essentially every codepoint. Taking the first face happens to
    /// avoid it here and would not elsewhere, failing the test for a reason that has
    /// nothing to do with planning. The search short-circuits on the first face that
    /// cannot draw, so it costs one resolution in practice, not 2966.
    fn family_that_cannot_draw(text: &str) -> Option<String> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        // Zero faces: every name resolves to NoFont, so any name gives an unresolvable
        // text. Returning None here instead would skip the regression on exactly the
        // machines where fonts are least predictable.
        if db.faces().next().is_none() {
            return Some("Any Family".into());
        }
        // Bound before returning: as a tail expression the iterator's borrow of `db`
        // outlives `db` itself.
        let found = db.faces()
            .filter_map(|f| f.families.first().map(|(name, _)| name.clone()))
            .find(|name| geometry::text_to_path(name, 10.0, text).is_err());
        found
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

        // marked not-cut: excluded from every pass
        let not_cut = ed.doc.ids.next();
        let node = with_no_cut(Node::shape(not_cut, ShapeKind::Rect { w: 5.0, h: 5.0 }));
        ed.commit(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));

        let planned = plan_passes(&ed.doc).unwrap();
        assert_eq!(planned.passes.len(), 2, "red + blue; the not-cut rect excluded");
        assert_eq!(planned.skipped_not_cut, 1);
        let red = &planned.passes[0]; // first-seen order
        assert_eq!(red.color, Some(RED));
        assert_eq!(red.shapes.len(), 2);
        // the grouped child's polyline reflects the group's translate (world transform applied)
        assert!(red.shapes.iter().any(|s| s.polylines[0][0].x >= 10.0));
        assert_eq!(planned.passes[1].color, Some(BLUE));
    }

    #[test]
    fn text_with_unknown_family_falls_back_or_reports_no_fonts() {
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

        // A bogus family substitutes an installed font (a project from another machine
        // still plans); only a system with zero faces refuses, and says so.
        let bad_id = ed.doc.ids.next();
        let node = Node::shape(bad_id, ShapeKind::Text {
            family: "Definitely Not A Real Font Family 12345".into(), size_mm: 10.0, text: "Hi".into(),
        });
        let mut bad_doc = ed.doc.clone();
        bad_doc.apply(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));
        match any_available_family() {
            Some(_) => {
                let planned = plan_passes(&bad_doc).expect("fallback should have substituted a font");
                let pass = planned.passes.last().unwrap();
                assert!(!pass.shapes.last().unwrap().polylines.is_empty());
            }
            None => assert_eq!(plan_passes(&bad_doc),
                Err(PlanError::BadShape(bad_id, geometry::GeomError::NoFont.to_string()))),
        }
    }

    /// A node the plan excludes must not be able to refuse the plan. `shape_outline`
    /// used to run before the stroke filter, so a font failure on a shape that would
    /// never be cut took unrelated valid geometry down with it (#139).
    #[test]
    fn a_skipped_text_that_cannot_resolve_does_not_refuse_the_plan() {
        let Some(family) = family_that_cannot_draw(UNDRAWABLE) else { return };
        let unresolvable = ShapeKind::Text {
            family, size_mm: 10.0, text: UNDRAWABLE.into(),
        };

        let mut ed = Editor::new();
        let root = ed.doc.root;
        let rect = ed.doc.ids.next();
        let node = with_stroke(Node::shape(rect, ShapeKind::Rect { w: 5.0, h: 5.0 }), Some(0xFF0000FF));
        ed.commit(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));

        // Establish the premise rather than assuming it: stroked, this text must refuse the
        // plan. If some face did draw those noncharacters the strokeless half below would
        // pass against the unfixed traversal too, pinning nothing — so assert it here, where
        // a face that resolves fails the test loudly instead of hollowing it out.
        let stroked = ed.doc.ids.next();
        let mut premise = ed.doc.clone();
        premise.apply(Delta(vec![NodeOp::Add {
            parent: root, index: usize::MAX,
            node: with_stroke(Node::shape(stroked, unresolvable.clone()), Some(0x0000FFFF)),
        }]));
        assert!(
            matches!(plan_passes(&premise), Err(PlanError::BadShape(id, _)) if id == stroked),
            "premise void: this text resolves on the picked face, so the case below pins nothing",
        );

        // The contract: the same text, marked not-cut, is skipped instead of fatal.
        let text = ed.doc.ids.next();
        ed.commit(Delta(vec![NodeOp::Add {
            parent: root, index: usize::MAX,
            node: with_no_cut(Node::shape(text, unresolvable)),
        }]));

        let planned = plan_passes(&ed.doc).expect("a skipped shape must not refuse the plan");
        assert_eq!(planned.passes.len(), 1, "the rect still plans");
        assert_eq!(planned.passes[0].shapes.len(), 1);
        assert_eq!(planned.skipped_not_cut, 1, "the text is skipped, not fatal");
    }

    /// Same ordering bug, reached through a different `shape_outline` branch — the defect
    /// is in when the outline is resolved, not in text.
    #[test]
    fn a_skipped_path_with_unreadable_data_does_not_refuse_the_plan() {
        let mut ed = Editor::new();
        let root = ed.doc.root;

        let rect = ed.doc.ids.next();
        let node = with_stroke(Node::shape(rect, ShapeKind::Rect { w: 5.0, h: 5.0 }), Some(0xFF0000FF));
        ed.commit(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));

        let bad = ed.doc.ids.next();
        let node = with_no_cut(
            Node::shape(bad, ShapeKind::Path { d: "totally not path data".into() }));
        ed.commit(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));

        let planned = plan_passes(&ed.doc).expect("a skipped shape must not refuse the plan");
        assert_eq!(planned.passes.len(), 1);
        assert_eq!(planned.skipped_not_cut, 1);
    }

    /// The other half of the contract: deferring resolution must not swallow a failure on
    /// a shape the plan *does* include. That one still refuses, with the same sentence.
    #[test]
    fn a_cut_shape_with_unreadable_data_still_refuses_the_plan() {
        let mut ed = Editor::new();
        let root = ed.doc.root;
        let bad = ed.doc.ids.next();
        let node = with_stroke(
            Node::shape(bad, ShapeKind::Path { d: "totally not path data".into() }),
            Some(0xFF0000FF),
        );
        ed.commit(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));

        match plan_passes(&ed.doc) {
            Err(PlanError::BadShape(id, message)) => {
                assert_eq!(id, bad);
                assert!(message.contains("path data"), "unexpected message: {message}");
            }
            other => panic!("expected BadShape for a shape that would be cut, got {other:?}"),
        }
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

    /// The point of the whole change: geometry with no stroke is cut when it says it is,
    /// and its pass is keyed on the fill so an operator can still tell passes apart.
    #[test]
    fn a_fill_only_shape_that_is_cut_plans_into_a_pass_keyed_on_its_fill() {
        let mut doc = Document::new();
        let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 10.0, h: 10.0 });
        node.style = Style { stroke: None, fill: Some(0x00FF00FF) };
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));

        let planned = plan_passes(&doc).unwrap();
        assert_eq!(planned.skipped_not_cut, 0);
        assert_eq!(planned.passes.len(), 1);
        assert_eq!(planned.passes[0].color, Some(0x00FF00FF));
    }

    /// The other direction, which the old rule could not express at all: a shape with a
    /// perfectly good stroke that the operator has marked not to cut.
    #[test]
    fn a_stroked_shape_marked_no_cut_plans_into_nothing() {
        let mut doc = Document::new();
        let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 10.0, h: 10.0 });
        node.style = Style { stroke: Some(0xFF0000FF), fill: None };
        node.cut_line_type = CutLineType::NoCut;
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));

        let planned = plan_passes(&doc).unwrap();
        assert!(planned.passes.is_empty());
        assert_eq!(planned.skipped_not_cut, 1);
    }

    /// Neither paint, and cut anyway. `ColorPass::color` has always been `Option<u32>`;
    /// this is the first thing that can make it `None`, so every consumer that renders a
    /// swatch or prints a header now has a case that reaches it.
    #[test]
    fn a_cut_shape_with_no_paint_lands_in_the_colorless_pass() {
        let mut doc = Document::new();
        let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 10.0, h: 10.0 });
        node.style = Style { stroke: None, fill: None };
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));

        let planned = plan_passes(&doc).unwrap();
        assert_eq!(planned.passes.len(), 1);
        assert_eq!(planned.passes[0].color, None);
    }

    /// Alpha-0 paint is not a colour to group by, in either channel — a fully transparent
    /// stroke used to mean "not cut", and the fallback must not resurrect it as a key.
    #[test]
    fn transparent_paint_is_not_a_pass_key() {
        let mut doc = Document::new();
        let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 10.0, h: 10.0 });
        node.style = Style { stroke: Some(0xFF000000), fill: Some(0x00FF0000) };
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));

        let planned = plan_passes(&doc).unwrap();
        assert_eq!(planned.passes.len(), 1);
        assert_eq!(planned.passes[0].color, None, "both paints are invisible, so neither keys the pass");
    }
}
