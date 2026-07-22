// SPDX-License-Identifier: GPL-3.0-or-later
use geometry::{Path, Seg, Point};

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
fn mm(p: usvg::tiny_skia_path::Point) -> Point { Point { x: p.x as f64 * PX_TO_MM, y: p.y as f64 * PX_TO_MM } }
fn lerp23(from: Point, to: Point) -> Point {
    Point { x: from.x + 2.0 / 3.0 * (to.x - from.x), y: from.y + 2.0 / 3.0 * (to.y - from.y) }
}
fn paint_rgba(_paint: &usvg::Paint) -> u32 { 0x000000FF } // ponytail: solid-black default; real color mapping later

#[cfg(test)]
mod tests {
    use super::*;
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
}
