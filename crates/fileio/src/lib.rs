// SPDX-License-Identifier: GPL-3.0-or-later
use geometry::{Path, Seg, Point, Affine};
use document::{Document, NodeId, NodeKind, ShapeKind};

pub mod import;
pub use import::import_svg;
pub mod project;
pub use project::{save_project, load_project};

/// Minimal scene-tree → SVG serializer for the interchange `design.svg`.
/// The manifest (`Document::snapshot_json`) is the source of truth on load;
/// this is a best-effort visual copy, so unsupported node kinds are skipped
/// with a comment rather than causing an error.
pub fn doc_to_svg(doc: &Document) -> String {
    let mut body = String::new();
    walk_svg(doc, doc.root, &Affine::identity(), &mut body);
    let ab = doc.artboard;
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" viewBox="{x} {y} {w} {h}">{body}</svg>"#,
        x = ab.x, y = ab.y, w = ab.w, h = ab.h, body = body,
    )
}

fn walk_svg(doc: &Document, id: NodeId, parent_xf: &Affine, out: &mut String) {
    let Some(node) = doc.get(id) else { return };
    let xf = node.transform.then(parent_xf);
    match &node.kind {
        NodeKind::Shape(shape) => match shape_path(shape) {
            Some(p) => out.push_str(&format!("<path d=\"{}\"/>", p.transformed(&xf).to_svg())),
            None => out.push_str(&format!("<!-- skipped {} -->", shape_kind_name(shape))),
        },
        NodeKind::Group | NodeKind::Layer => {
            for child in &node.children { walk_svg(doc, *child, &xf, out); }
        }
    }
}

fn shape_path(kind: &ShapeKind) -> Option<Path> {
    match kind {
        ShapeKind::Path { d } => Path::from_svg(d).ok(),
        ShapeKind::Rect { w, h } => Some(geometry::rect_path(0.0, 0.0, *w, *h)),
        ShapeKind::Ellipse { rx, ry } => Some(geometry::ellipse_path(0.0, 0.0, *rx, *ry)),
        ShapeKind::Text { .. } => None,
    }
}

fn shape_kind_name(kind: &ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Path { .. } => "path (unparseable)",
        ShapeKind::Rect { .. } => "rect",
        ShapeKind::Ellipse { .. } => "ellipse",
        ShapeKind::Text { .. } => "text",
    }
}

const PX_TO_MM: f64 = 25.4 / 96.0;

#[derive(Debug)]
pub enum IoError { Parse(String), Io(String) }
#[derive(Clone, Debug)]
pub struct StyleHint { pub stroke: Option<u32>, pub fill: Option<u32> }
pub struct SvgImport { pub paths: Vec<(Path, StyleHint)>, pub skipped: Vec<String> }

pub fn svg_to_paths(bytes: &[u8]) -> Result<SvgImport, IoError> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default())
        .map_err(|e| IoError::Parse(e.to_string()))?;
    let mut paths = vec![];
    let mut skipped = vec![];
    walk(tree.root(), &mut paths, &mut skipped);
    Ok(SvgImport { paths, skipped })
}

fn walk(group: &usvg::Group, out: &mut Vec<(Path, StyleHint)>, skipped: &mut Vec<String>) {
    for node in group.children() {
        match node {
            usvg::Node::Path(p) => {
                let mut segs = vec![];
                let mut here = Point { x: 0.0, y: 0.0 };
                let t = p.abs_transform();
                let mm = |pt: usvg::tiny_skia_path::Point| mm_transformed(pt, &t);
                for seg in p.data().segments() {
                    use usvg::tiny_skia_path::PathSegment as S;
                    match seg {
                        S::MoveTo(pt) => { here = mm(pt); segs.push(Seg::Move(here)); }
                        S::LineTo(pt) => { here = mm(pt); segs.push(Seg::Line(here)); }
                        S::CubicTo(a, b, c) => { segs.push(Seg::Cubic(mm(a), mm(b), mm(c))); here = mm(c); }
                        S::QuadTo(q, e) => {
                            // Exact degree elevation: c1 = p0 + 2/3(q-p0), c2 = e + 2/3(q-e).
                            let (q, e) = (mm(q), mm(e));
                            let c1 = lerp23(here, q);
                            let c2 = lerp23(e, q);
                            segs.push(Seg::Cubic(c1, c2, e));
                            here = e;
                        }
                        S::Close => segs.push(Seg::Close),
                    }
                }
                let hint = StyleHint {
                    stroke: p.stroke().map(|s| paint_rgba(s.paint())),
                    fill: p.fill().map(|f| paint_rgba(f.paint())),
                };
                out.push((Path { segs }, hint));
            }
            usvg::Node::Group(g) => walk(g, out, skipped),
            usvg::Node::Image(_) => skipped.push("image".into()),
            usvg::Node::Text(_) => skipped.push("text".into()),
        }
    }
}
fn mm_transformed(p: usvg::tiny_skia_path::Point, t: &usvg::Transform) -> Point {
    let (x, y) = (p.x as f64, p.y as f64);
    let (tx, ty) = (
        t.sx as f64 * x + t.kx as f64 * y + t.tx as f64,
        t.ky as f64 * x + t.sy as f64 * y + t.ty as f64,
    );
    Point { x: tx * PX_TO_MM, y: ty * PX_TO_MM }
}
fn lerp23(from: Point, to: Point) -> Point {
    Point { x: from.x + 2.0 / 3.0 * (to.x - from.x), y: from.y + 2.0 / 3.0 * (to.y - from.y) }
}
fn paint_rgba(_paint: &usvg::Paint) -> u32 { 0x000000FF } // ponytail: solid-black default; real color mapping later

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn group_transforms_are_applied() {
        // 250px rect scaled 0.32 by an ancestor <g> → 80px → 80*25.4/96 mm.
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="378" height="378"
                        viewBox="0 0 378 378"><g transform="matrix(0.32,0,0,0.32,0,0)">
                        <rect x="100" y="100" width="250" height="250"/></g></svg>"#;
        let imp = svg_to_paths(svg).unwrap();
        let b = imp.paths[0].0.bounds();
        assert!((b.w - 80.0 * PX_TO_MM).abs() < 0.01, "w={} mm", b.w);
        assert!((b.x - 32.0 * PX_TO_MM).abs() < 0.01, "x={} mm", b.x);
    }
    #[test]
    fn quadratic_converts_to_exact_cubic() {
        // M0,0 Q10,10 20,0 — true quad midpoint (t=0.5) is (10, 5) px.
        // Wrong (q,q,e) conversion puts the cubic midpoint at (10, 7.5) px.
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"
                        viewBox="0 0 20 20"><path d="M0,0 Q10,10 20,0" fill="none" stroke="black"/></svg>"#;
        let imp = svg_to_paths(svg).unwrap();
        assert_eq!(imp.paths.len(), 1);
        let polys = imp.paths[0].0.flatten(0.01 * PX_TO_MM);
        let want = (10.0 * PX_TO_MM, 5.0 * PX_TO_MM);
        let hit = polys[0].iter().any(|p|
            (p.x - want.0).abs() < 0.05 && (p.y - want.1).abs() < 0.05);
        assert!(hit, "no flattened point near true quad midpoint {want:?}: {polys:?}");
    }
    #[test]
    fn parses_a_rect_into_one_path_in_mm() {
        // 20x20 user units at 96dpi → but usvg keeps user units; we map px→mm at 96dpi.
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"
                        viewBox="0 0 20 20"><rect width="20" height="20"/></svg>"#;
        let imp = svg_to_paths(svg).unwrap();
        assert_eq!(imp.paths.len(), 1);
        let b = imp.paths[0].0.bounds();
        // 20 px → 20 * 25.4/96 mm ≈ 5.29 mm
        assert!((b.w - 20.0 * 25.4 / 96.0).abs() < 0.01);
        assert!(imp.skipped.is_empty());
    }
    #[test]
    fn doc_to_svg_emits_a_path_for_a_rect_shape() {
        let mut doc = Document::new();
        let id = doc.ids.next();
        doc.apply(document::Delta(vec![document::NodeOp::Add {
            parent: doc.root, index: 0,
            node: document::Node::shape(id, ShapeKind::Rect { w: 5.0, h: 5.0 }) }]));
        let svg = doc_to_svg(&doc);
        assert!(svg.contains("<path"), "svg missing <path>: {svg}");
    }
    #[test]
    fn doc_to_svg_composes_transforms_child_first_then_ancestors() {
        // group: translate(10,0); child point (1,0) scaled 2x → (2,0), then group
        // translate → (12,0). If the composition order were swapped (ancestor
        // applied before child), the result would be (11,0)*2 = (22,0) instead.
        let mut doc = Document::new();
        let group_id = doc.ids.next();
        let mut group = document::Node::container(group_id, NodeKind::Group);
        group.transform = Affine::translate(10.0, 0.0);
        doc.apply(document::Delta(vec![document::NodeOp::Add {
            parent: doc.root, index: 0, node: group }]));

        let child_id = doc.ids.next();
        let mut child = document::Node::shape(child_id, ShapeKind::Path { d: "M1,0".into() });
        child.transform = Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        doc.apply(document::Delta(vec![document::NodeOp::Add {
            parent: group_id, index: 0, node: child }]));

        let svg = doc_to_svg(&doc);
        assert!(svg.contains("M12,0"), "expected composed point M12,0: {svg}");
        assert!(!svg.contains("M22,0"), "order looks swapped (parent applied before child): {svg}");
    }
}
