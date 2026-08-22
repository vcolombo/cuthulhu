// SPDX-License-Identifier: GPL-3.0-or-later
use geometry::{Path, Seg, Point, Affine};
use document::{Document, NodeId, NodeKind, ShapeKind};

pub mod import;
pub use import::import_svg;
pub mod project;
mod manifest;
pub use project::{save_project, load_project};

/// Minimal scene-tree → SVG serializer for the interchange `design.svg`.
/// `manifest.json`'s versioned envelope (see `manifest`) is the source of truth on load;
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
            Some(p) => out.push_str(&format!(
                "<path d=\"{}\"{}{}/>",
                p.transformed(&xf).to_svg(),
                paint_attrs("stroke", node.style.stroke),
                paint_attrs("fill", node.style.fill),
            )),
            None => out.push_str(&format!("<!-- skipped {} -->", shape_kind_name(shape))),
        },
        NodeKind::Group | NodeKind::Layer => {
            for child in &node.children { walk_svg(doc, *child, &xf, out); }
        }
    }
}

/// One paint as SVG attributes: explicit `stroke="none"`/`fill="none"` for absent paint —
/// SVG's own defaults (black fill, no stroke) are the exact inverse of the editor's, which
/// is how bare paths rendered as filled black blobs externally. Alpha rides in a separate
/// `-opacity` attribute because `#RRGGBBAA` hex is SVG 2 and usvg's own import folds
/// paint-opacity back into the RGBA hint, closing the round-trip.
fn paint_attrs(name: &str, paint: Option<u32>) -> String {
    match paint {
        None => format!(" {name}=\"none\""),
        Some(rgba) => {
            let a = rgba & 0xFF;
            let rgb = format!(" {name}=\"#{:06x}\"", rgba >> 8);
            if a == 0xFF {
                rgb
            } else {
                // 4 decimals: worst-case round-trip error is 5e-5 * 255 ≈ 0.013, well
                // under the 0.5 that would round back to a different alpha byte.
                format!("{rgb} {name}-opacity=\"{:.4}\"", a as f64 / 255.0)
            }
        }
    }
}

fn shape_path(kind: &ShapeKind) -> Option<Path> {
    match kind {
        ShapeKind::Path { d } => Path::from_svg(d).ok(),
        ShapeKind::Rect { w, h } => Some(geometry::rect_path(0.0, 0.0, *w, *h)),
        ShapeKind::Ellipse { rx, ry } => Some(geometry::ellipse_path(*rx, *ry, *rx, *ry)),
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
pub enum IoError {
    Parse(String),
    Io(String),
    /// A project written by a newer build. Named before any of its document is deserialized, and
    /// named again when a save is aimed at it, because the two are the same fact: this build
    /// cannot read the file, so it must not be the one to replace it.
    UnsupportedProjectVersion { found: u32, supported: u32 },
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoError::Parse(m) => write!(f, "the file could not be understood ({m})"),
            IoError::Io(m) => write!(f, "the file could not be read or written ({m})"),
            IoError::UnsupportedProjectVersion { found, supported } => write!(
                f,
                "this project was saved by a newer Cuthulhu \
                 (manifest version {found}; this build reads {supported})"
            ),
        }
    }
}

impl std::error::Error for IoError {}

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
                    stroke: p.stroke().map(|s| paint_rgba(s.paint(), s.opacity())),
                    fill: p.fill().map(|f| paint_rgba(f.paint(), f.opacity())),
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
fn paint_rgba(paint: &usvg::Paint, opacity: usvg::Opacity) -> u32 {
    match paint {
        usvg::Paint::Color(c) => {
            let a = (opacity.get() * 255.0).round() as u32;
            ((c.red as u32) << 24) | ((c.green as u32) << 16) | ((c.blue as u32) << 8) | a
        }
        // Gradients/patterns can't drive a blade; fall back to opaque black so the shape still cuts visibly.
        _ => 0x000000FF,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole table at once: a new variant fails to compile the match in `Display`, and a
    /// reworded one fails here. These strings are what an operator reads when a project will
    /// not open or save, so their wording is pinned rather than just their variant (#93).
    #[test]
    fn every_io_refusal_has_a_sentence() {
        let cases: Vec<(IoError, &str)> = vec![
            (
                IoError::Parse("unexpected end of file".into()),
                "the file could not be understood (unexpected end of file)",
            ),
            (
                IoError::Io("Permission denied (os error 13)".into()),
                "the file could not be read or written (Permission denied (os error 13))",
            ),
            (
                IoError::UnsupportedProjectVersion { found: 99, supported: 2 },
                "this project was saved by a newer Cuthulhu \
                 (manifest version 99; this build reads 2)",
            ),
        ];
        for (error, sentence) in cases {
            assert_eq!(error.to_string(), sentence, "{error:?}");
        }
    }

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
    fn doc_to_svg_ellipse_uses_center_at_rx_ry_local_space() {
        // Canonical convention (see commands.rs::shape_to_path): an Ellipse's local
        // space is centered at (rx, ry), i.e. bounds 0..2rx / 0..2ry, not 0..0 centered.
        let mut doc = Document::new();
        let id = doc.ids.next();
        doc.apply(document::Delta(vec![document::NodeOp::Add {
            parent: doc.root, index: 0,
            node: document::Node::shape(id, ShapeKind::Ellipse { rx: 3.0, ry: 2.0 }) }]));
        let svg = doc_to_svg(&doc);
        assert!(svg.contains("M6,2"), "expected ellipse path to start at (2rx,ry)=(6,2): {svg}");
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
    fn shape_with_style(doc: &mut Document, kind: ShapeKind, stroke: Option<u32>, fill: Option<u32>) {
        let id = doc.ids.next();
        let mut node = document::Node::shape(id, kind);
        node.style = document::Style { stroke, fill };
        doc.apply(document::Delta(vec![document::NodeOp::Add {
            parent: doc.root, index: usize::MAX, node }]));
    }

    #[test]
    fn doc_to_svg_default_style_is_black_stroke_no_fill() {
        // SVG's own defaults are the inverse (black fill, no stroke), so both
        // attributes must be explicit or external renderers invert the design.
        let mut doc = Document::new();
        let id = doc.ids.next();
        doc.apply(document::Delta(vec![document::NodeOp::Add {
            parent: doc.root, index: 0,
            node: document::Node::shape(id, ShapeKind::Rect { w: 5.0, h: 5.0 }) }]));
        let svg = doc_to_svg(&doc);
        assert!(svg.contains(r##"stroke="#000000""##), "missing black stroke: {svg}");
        assert!(svg.contains(r#"fill="none""#), "missing explicit no-fill: {svg}");
        assert!(!svg.contains("stroke-opacity"), "opaque paint needs no opacity attr: {svg}");
    }

    #[test]
    fn doc_to_svg_translucent_paint_emits_opacity() {
        let mut doc = Document::new();
        shape_with_style(&mut doc, ShapeKind::Rect { w: 5.0, h: 5.0 }, Some(0x0000FF80), Some(0x00FF0040));
        let svg = doc_to_svg(&doc);
        assert!(svg.contains(r##"stroke="#0000ff" stroke-opacity="0.5020""##), "stroke alpha: {svg}");
        assert!(svg.contains(r##"fill="#00ff00" fill-opacity="0.2510""##), "fill alpha: {svg}");
    }

    #[test]
    fn write_then_import_round_trips_stroke_and_fill() {
        let mut doc = Document::new();
        shape_with_style(&mut doc, ShapeKind::Rect { w: 5.0, h: 5.0 }, Some(0xFF0000FF), None);
        shape_with_style(&mut doc, ShapeKind::Rect { w: 5.0, h: 5.0 }, None, Some(0x00FF00FF));
        shape_with_style(&mut doc, ShapeKind::Rect { w: 5.0, h: 5.0 }, Some(0x0000FF80), None);
        let svg = doc_to_svg(&doc);
        let imp = svg_to_paths(svg.as_bytes()).unwrap();
        let hints: Vec<(Option<u32>, Option<u32>)> =
            imp.paths.iter().map(|(_, h)| (h.stroke, h.fill)).collect();
        assert!(hints.contains(&(Some(0xFF0000FF), None)), "opaque stroke: {hints:?}");
        assert!(hints.contains(&(None, Some(0x00FF00FF))), "fill-only: {hints:?}");
        assert!(hints.contains(&(Some(0x0000FF80), None)), "alpha survives the opacity attr: {hints:?}");
    }

    #[test]
    fn doc_to_svg_still_skips_text_with_a_comment() {
        let mut doc = Document::new();
        shape_with_style(&mut doc,
            ShapeKind::Text { family: "X".into(), size_mm: 10.0, text: "hi".into() },
            Some(0xFF0000FF), None);
        let svg = doc_to_svg(&doc);
        assert!(svg.contains("<!-- skipped text -->"), "comment emission changed: {svg}");
        assert!(!svg.contains("<path"), "text must not emit a styled path: {svg}");
    }

    #[test]
    fn import_preserves_stroke_colors_and_none() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="10">
        <rect width="10" height="10" stroke="#ff0000" fill="none"/>
        <rect x="10" width="10" height="10" stroke="#0000ff" fill="none"/>
        <rect x="20" width="10" height="10" fill="#00ff00"/></svg>"##;
        let imp = svg_to_paths(svg).unwrap();
        let strokes: Vec<Option<u32>> = imp.paths.iter().map(|(_, h)| h.stroke).collect();
        assert!(strokes.contains(&Some(0xFF0000FF)), "red stroke preserved: {strokes:?}");
        assert!(strokes.contains(&Some(0x0000FFFF)), "blue stroke preserved");
        assert!(strokes.contains(&None), "no-stroke shape stays None");
    }
}
