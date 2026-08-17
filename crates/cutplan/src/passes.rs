// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use document::{shape_outline, CutLineType, Document, NodeId, NodeKind, PresetAssignment, Style};
use geometry::{Affine, Point, Polyline};
use serde::{Deserialize, Serialize};

use crate::pass_key::PassKey;

/// A single shape's flattened, world-transformed outline, ready to cut.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct PlannedShape { pub node_id: NodeId, pub polylines: Vec<Polyline> }

/// All shapes cut together as one pass, and the key that says which pass it is. What the key
/// means is the `Grouping`'s business: a colour, a material preset, or `All` for the single
/// pass a `Grouping::Single` plan holds.
///
/// Named for the Document rather than for a colour because a colour is now one of three
/// things a pass can be keyed on — the type was `ColorPass` while it was the only one.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DocumentPass { pub key: PassKey, pub shapes: Vec<PlannedShape> }

/// Every `DocumentPass` a document contains, in first-seen order — an inventory of
/// what *could* be cut. Nothing here is selected, configured or checked; that is
/// `plan_cut`'s job, and what it returns is a `CutPlan`.
///
/// This is everything a cut needs to know about the document, which is why
/// `plan_cut` takes one of these rather than a `Document`: the geometry that
/// gets validated is the geometry that gets cut, and nobody plans twice.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DocumentPasses {
    pub passes: Vec<DocumentPass>,
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

/// The colour a shape's pass is keyed on under a colour-ish `Grouping`. Alpha-0 counts as
/// absent, exactly as a 0-alpha stroke did when the stroke decided cuttability.
///
/// `Color` falls back from stroke to fill because a shape with no stroke can be cut since
/// #144 — traced and fill-only art is the common case — and a pass with no colour at all is
/// something an operator cannot recognise in a pass list. `Stroke` and `Fill` are strict by
/// request: an operator who asked to split by one paint does not want the other silently
/// standing in for it.
fn color_key(style: &Style, grouping: Grouping) -> Option<u32> {
    let visible = |c: Option<u32>| c.filter(|c| c & 0xFF != 0);
    match grouping {
        Grouping::Color => visible(style.stroke).or(visible(style.fill)),
        Grouping::Stroke => visible(style.stroke),
        Grouping::Fill => visible(style.fill),
        // Not reachable: the caller asks for a colour only under a colour mode.
        Grouping::Single | Grouping::Preset => None,
    }
}

/// How `plan_passes` splits cut shapes into passes.
///
/// `Color` is today's rule — stroke where visible, else fill — and stays the default, so a
/// caller that names no mode plans exactly what it planned before #148. `Single` is one pass
/// in document order, which is what `cuthulhu cut` without `--group-by` has always meant.
///
/// There is no line-type mode: `CutLineType` is `{Cut, NoCut}` and a `NoCut` shape never
/// reaches a pass, so such a mode would be `Single` under another name while carrying
/// different skip/order semantics. #56 adds it with `CutEdge`, the member that makes it split
/// anything.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Grouping { Single, Color, Stroke, Fill, Preset }

/// Walk the document in preorder from `doc.root`, group the shapes whose `CutLineType` is
/// `Cut` by the key `grouping` asks for, and flatten each shape's outline under its
/// accumulated world transform. A `NoCut` shape is counted, not cut. Iterative (explicit
/// stack) so depth is not bounded by the Rust call stack; a `visited` set catches cycles in
/// malformed docs.
pub fn plan_passes(doc: &Document) -> Result<DocumentPasses, PlanError> {
    plan_passes_with(doc, Grouping::Color)
}

/// `plan_passes` with the grouping named explicitly. See `Grouping`.
pub fn plan_passes_with(doc: &Document, grouping: Grouping) -> Result<DocumentPasses, PlanError> {
    let mut visited: HashSet<NodeId> = HashSet::new();
    // The nearest assigned ancestor's material rides down the walk beside the world
    // transform. Storing a resolved value on each shape instead would go stale the moment a
    // node is reparented — silently, and only visible as the wrong settings on real material.
    let mut stack: Vec<(NodeId, Affine, Option<&str>)> = vec![(doc.root, Affine::identity(), None)];
    let mut passes: Vec<DocumentPass> = vec![];
    let mut skipped_not_cut = 0usize;

    while let Some((id, parent_world, inherited)) = stack.pop() {
        if !visited.insert(id) {
            return Err(PlanError::CycleDetected);
        }
        let node = doc.get(id).ok_or(PlanError::MissingNode(id))?;
        let world = node.transform.then(&parent_world);

        // Resolved for this node and everything under it. `Unassigned` is what stops the
        // chain — the state an `Option<String>` could not express.
        let material: Option<&str> = match &node.material_preset {
            PresetAssignment::Inherit => inherited,
            PresetAssignment::Unassigned => None,
            PresetAssignment::Preset(id) => Some(id.as_str()),
        };

        // Descend on the node's own kind, not on `shape_outline` returning `None`. The two
        // agree, but reading it from `NodeKind` is what lets the outline stay unresolved
        // until the shape is known to be cut — resolving first meant a font or path-data
        // failure on a shape nobody would cut refused the whole plan (#139).
        match &node.kind {
            NodeKind::Group | NodeKind::Layer => {
                // Push in reverse so preorder visits children left-to-right.
                for &child in node.children.iter().rev() {
                    stack.push((child, world, material));
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
                        match grouping {
                            // Matched on the borrowed id, and owned only when a pass is
                            // actually created: keying every shape would allocate a `String`
                            // per cut shape, including each one that joins a pass already
                            // there — per-shape heap churn on exactly the documents where
                            // preset grouping is worth having.
                            Grouping::Preset => {
                                match passes.iter_mut()
                                    .find(|p| matches!(&p.key, PassKey::Preset(id) if id.as_deref() == material))
                                {
                                    Some(pass) => pass.shapes.push(shape),
                                    None => passes.push(DocumentPass {
                                        // Not checked against the preset file: a deleted user
                                        // preset is a real state, and refusing a cut over a
                                        // settings lookup is not `plan_cut`'s job.
                                        key: PassKey::Preset(material.map(String::from)),
                                        shapes: vec![shape],
                                    }),
                                }
                            }
                            // Every other key is `Copy`-cheap to build, so build then match.
                            _ => {
                                let key = match grouping {
                                    // One bucket, and a key that says so: `Color(None)` is the
                                    // pass of unpainted shapes, which is a different fact.
                                    Grouping::Single => PassKey::All,
                                    _ => PassKey::Color(color_key(&node.style, grouping)),
                                };
                                match passes.iter_mut().find(|p| p.key == key) {
                                    Some(pass) => pass.shapes.push(shape),
                                    None => passes.push(DocumentPass { key, shapes: vec![shape] }),
                                }
                            }
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
pub fn travel_moves(configured: &[&DocumentPass]) -> Vec<(Point, Point)> {
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
        assert_eq!(red.key, PassKey::Color(Some(RED)));
        assert_eq!(red.shapes.len(), 2);
        // the grouped child's polyline reflects the group's translate (world transform applied)
        assert!(red.shapes.iter().any(|s| s.polylines[0][0].x >= 10.0));
        assert_eq!(planned.passes[1].key, PassKey::Color(Some(BLUE)));
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
        let pass_a = DocumentPass {
            key: PassKey::Color(Some(1)),
            shapes: vec![
                shape(1, vec![vec![pt(0.0, 0.0), pt(1.0, 0.0)]]),
                shape(2, vec![vec![pt(2.0, 0.0), pt(3.0, 0.0)]]),
            ],
        };
        let pass_b = DocumentPass {
            key: PassKey::Color(Some(2)),
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
        assert_eq!(planned.passes[0].key, PassKey::Color(Some(0x00FF00FF)));
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

    /// Neither paint, and cut anyway. A pass has always been able to carry no colour;
    /// #144 made that reachable, so every consumer that renders a swatch or prints a header
    /// has a case for it — which is now `PassKey::Color(None)`, written `no-color`.
    #[test]
    fn a_cut_shape_with_no_paint_lands_in_the_colorless_pass() {
        let mut doc = Document::new();
        let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 10.0, h: 10.0 });
        node.style = Style { stroke: None, fill: None };
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));

        let planned = plan_passes(&doc).unwrap();
        assert_eq!(planned.passes.len(), 1);
        assert_eq!(planned.passes[0].key, PassKey::Color(None));
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
        assert_eq!(planned.passes[0].key, PassKey::Color(None), "both paints are invisible, so neither keys the pass");
    }

    /// The fill is a FALLBACK, not a co-equal key: a shape carrying both visible paints is
    /// keyed on its stroke. Every other pass-key test gives a shape one visible paint or
    /// none, so reversing `pass_key`'s two arms would leave all of them green while
    /// silently regrouping every stroked-and-filled document into different passes — a
    /// different set of colours for the operator to swap tools between.
    #[test]
    fn a_shape_with_both_paints_is_keyed_on_its_stroke() {
        let mut doc = Document::new();
        let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 10.0, h: 10.0 });
        node.style = Style { stroke: Some(0xFF0000FF), fill: Some(0x00FF00FF) };
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));

        let planned = plan_passes(&doc).unwrap();
        assert_eq!(planned.passes.len(), 1);
        assert_eq!(planned.passes[0].key, PassKey::Color(Some(0xFF0000FF)), "the stroke wins over the fill");
    }

    /// `Single` exists so the plain CLI cut can stop overwriting the document's colours to
    /// get one pass. Document order is the substance of it: merging colour-grouped passes
    /// afterwards would have concatenated colour by colour and quietly reordered the cut
    /// (see the spec's rejected alternative), so the order is asserted, not just the count.
    #[test]
    fn single_grouping_yields_one_pass_in_document_order() {
        let mut doc = Document::new();
        let mut ids = vec![];
        for fill in [0xFF0000FF, 0x00FF00FF, 0xFF0000FF] {
            let mut node = document::Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
            node.style = Style { stroke: None, fill: Some(fill) };
            ids.push(node.id);
            doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));
        }

        let by_color = plan_passes_with(&doc, Grouping::Color).unwrap();
        assert_eq!(by_color.passes.len(), 2, "premise: two fills, so colour grouping splits");

        let single = plan_passes_with(&doc, Grouping::Single).unwrap();
        assert_eq!(single.passes.len(), 1);
        assert_eq!(single.passes[0].key, PassKey::All, "one pass by request, not the colourless pass");
        let planned: Vec<_> = single.passes[0].shapes.iter().map(|s| s.node_id).collect();
        assert_eq!(planned, ids, "document order, not colour-grouped order");
    }

    /// One document, five modes, and the key set each produces. The point of the table is
    /// that the modes differ only in what they key on: the same shapes are cut, in the same
    /// document order, and only the split changes.
    #[test]
    fn every_grouping_keys_the_same_shapes_differently() {
        const GREEN: u32 = 0x00FF00FF;
        const RED: u32 = 0xFF0000FF;
        const BLUE: u32 = 0x0000FFFF;
        let mut doc = Document::new();
        for style in [
            Style { stroke: Some(RED), fill: Some(GREEN) },
            Style { stroke: Some(GREEN), fill: Some(GREEN) },
            Style { stroke: None, fill: Some(BLUE) },
            Style { stroke: None, fill: None },
        ] {
            let mut node = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
            node.style = style;
            doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node, index: usize::MAX }]));
        }

        let keys = |g: Grouping| -> Vec<String> {
            plan_passes_with(&doc, g).unwrap().passes.iter().map(|p| p.key.to_string()).collect()
        };

        assert_eq!(keys(Grouping::Single), vec!["all"]);
        // Stroke where visible, else fill: the rule #144 shipped, unchanged.
        assert_eq!(keys(Grouping::Color),
            vec!["color:ff0000ff", "color:00ff00ff", "color:0000ffff", "no-color"]);
        // Strict: a shape with no visible stroke keys on no colour at all, which is the same
        // bucket a shape with no paint whatsoever lands in.
        assert_eq!(keys(Grouping::Stroke), vec!["color:ff0000ff", "color:00ff00ff", "no-color"]);
        assert_eq!(keys(Grouping::Fill), vec!["color:00ff00ff", "color:0000ffff", "no-color"]);
        assert_eq!(keys(Grouping::Preset), vec!["no-preset"]);

        for g in [Grouping::Single, Grouping::Color, Grouping::Stroke, Grouping::Fill, Grouping::Preset] {
            let planned = plan_passes_with(&doc, g).unwrap();
            let shapes: usize = planned.passes.iter().map(|p| p.shapes.len()).sum();
            assert_eq!(shapes, 4, "{g:?} dropped a shape");
            assert_eq!(planned.skipped_not_cut, 0);
        }
    }

    /// `plan_passes` is what every caller that does not name a mode gets, and #148 must not
    /// move it: `Color` is verbatim the stroke-else-fill rule those callers already had.
    #[test]
    fn the_default_grouping_is_unchanged_colour_grouping() {
        let mut doc = Document::new();
        let mut shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        shape.style = Style { stroke: None, fill: Some(0xFF0000FF) };
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        assert_eq!(plan_passes(&doc).unwrap().passes,
            plan_passes_with(&doc, Grouping::Color).unwrap().passes);
        assert_eq!(plan_passes(&doc).unwrap().passes[0].key, PassKey::Color(Some(0xFF0000FF)));
    }

    /// The three assignment states, resolved down the tree. `Unassigned` is the one that
    /// earns the enum: without it the second shape could not leave its Layer's pass.
    #[test]
    fn a_material_resolves_from_the_nearest_assigned_ancestor() {
        let mut doc = Document::new();
        let mut layer = Node::container(doc.ids.next(), NodeKind::Layer);
        layer.material_preset = PresetAssignment::Preset("cameo5-htv".into());
        let layer_id = layer.id;
        let inherits = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let mut refuses = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        refuses.material_preset = PresetAssignment::Unassigned;
        let mut overrides = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        overrides.material_preset = PresetAssignment::Preset("cameo5-vinyl-adhesive".into());
        doc.apply(Delta(vec![
            NodeOp::Add { parent: doc.root, node: layer, index: usize::MAX },
            NodeOp::Add { parent: layer_id, node: inherits, index: usize::MAX },
            NodeOp::Add { parent: layer_id, node: refuses, index: usize::MAX },
            NodeOp::Add { parent: layer_id, node: overrides, index: usize::MAX },
        ]));

        let keys: Vec<String> = plan_passes_with(&doc, Grouping::Preset).unwrap()
            .passes.iter().map(|p| p.key.to_string()).collect();
        assert_eq!(keys, vec!["preset:cameo5-htv", "no-preset", "preset:cameo5-vinyl-adhesive"]);
    }

    /// Resolution lives in the walk, so a shape moved into an assigned Layer picks that
    /// Layer's material up with no edit of its own. A stored resolved value would have gone
    /// stale here, silently, and only shown up as the wrong settings on real material.
    #[test]
    fn a_reparented_shape_inherits_without_being_edited() {
        let mut doc = Document::new();
        let mut layer = Node::container(doc.ids.next(), NodeKind::Layer);
        layer.material_preset = PresetAssignment::Preset("cameo5-htv".into());
        let layer_id = layer.id;
        // A shape already inside the Layer, so its material's pass exists before the move —
        // an empty container contributes no pass at all.
        let resident = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        let shape_id = shape.id;
        doc.apply(Delta(vec![
            NodeOp::Add { parent: doc.root, node: layer, index: usize::MAX },
            NodeOp::Add { parent: layer_id, node: resident, index: usize::MAX },
            NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX },
        ]));
        let before: Vec<String> = plan_passes_with(&doc, Grouping::Preset).unwrap()
            .passes.iter().map(|p| p.key.to_string()).collect();
        assert_eq!(before, vec!["preset:cameo5-htv", "no-preset"],
            "premise: outside the Layer it resolves to no material");

        // Remove carries only the id; the node itself comes back through the Add, which is
        // exactly how a reparent is expressed (`document::NodeOp`).
        let moved = doc.get(shape_id).unwrap().clone();
        doc.apply(Delta(vec![
            NodeOp::Remove { parent: doc.root, id: shape_id },
            NodeOp::Add { parent: layer_id, node: moved, index: usize::MAX },
        ]));

        let keys: Vec<String> = plan_passes_with(&doc, Grouping::Preset).unwrap()
            .passes.iter().map(|p| p.key.to_string()).collect();
        assert_eq!(keys, vec!["preset:cameo5-htv"]);
    }

    /// An id no preset file resolves still keys a pass. Refusing here would put a
    /// settings-file concern behind `plan_cut`, and a user preset can be deleted while a
    /// document still names it.
    #[test]
    fn an_unknown_preset_id_still_keys_a_pass() {
        let mut doc = Document::new();
        let mut shape = Node::shape(doc.ids.next(), ShapeKind::Rect { w: 1.0, h: 1.0 });
        shape.material_preset = PresetAssignment::Preset("deleted-by-the-operator".into());
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        assert_eq!(plan_passes_with(&doc, Grouping::Preset).unwrap().passes[0].key,
            PassKey::Preset(Some("deleted-by-the-operator".into())));
    }

    /// The predicate is still `cut_line_type`, still checked before the outline is resolved
    /// (#139) — a grouping mode changes the key, never that order.
    #[test]
    fn a_no_cut_shape_is_counted_under_every_grouping() {
        let mut doc = Document::new();
        let mut shape = Node::shape(doc.ids.next(), ShapeKind::Text {
            family: "no such family".into(), size_mm: 10.0, text: "x".into() });
        shape.cut_line_type = CutLineType::NoCut;
        doc.apply(Delta(vec![NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX }]));

        for g in [Grouping::Single, Grouping::Color, Grouping::Stroke, Grouping::Fill, Grouping::Preset] {
            let planned = plan_passes_with(&doc, g).unwrap();
            assert_eq!(planned.skipped_not_cut, 1, "{g:?}");
            assert!(planned.passes.is_empty(), "{g:?}");
        }
    }
}
