// SPDX-License-Identifier: GPL-3.0-or-later
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

use crate::affine::Point;
use crate::path::{GeomError, Path, Seg};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoolOp { Union, Subtract, Intersect, Exclude }

/// Flatten tolerance (mm) used to feed curved paths into the polygon overlay engine.
const FLATTEN_TOL: f64 = 0.1;

type Contour = Vec<[f64; 2]>;
type Shape = Vec<Contour>; // one shape: outer contour(s) + holes, all fed through NonZero fill

fn path_to_shape(p: &Path) -> Shape {
    p.flatten(FLATTEN_TOL).into_iter().filter_map(|poly| {
        let mut pts: Vec<[f64; 2]> = poly.iter().map(|pt| [pt.x, pt.y]).collect();
        // flatten() closes polylines by repeating the start point; i_overlay auto-closes contours.
        if pts.len() > 1 && pts.first() == pts.last() { pts.pop(); }
        if pts.len() < 3 { None } else { Some(pts) }
    }).collect()
}

fn shape_to_path(shape: &Shape) -> Path {
    let mut segs = vec![];
    for contour in shape {
        if contour.is_empty() { continue; }
        segs.push(Seg::Move(Point { x: contour[0][0], y: contour[0][1] }));
        for pt in &contour[1..] { segs.push(Seg::Line(Point { x: pt[0], y: pt[1] })); }
        segs.push(Seg::Close);
    }
    Path { segs }
}

/// Polygon boolean over flattened input paths. Folds pairwise left-to-right: for `Subtract`
/// this yields paths[0] minus the union of the rest, which is equivalent to a one-shot
/// multi-clip difference; for `Union`/`Intersect`/`Exclude` the fold is associative.
pub fn boolean(op: BoolOp, paths: &[Path]) -> Result<Path, GeomError> {
    if paths.len() < 2 { return Err(GeomError::Degenerate); }
    let rule = match op {
        BoolOp::Union => OverlayRule::Union,
        BoolOp::Subtract => OverlayRule::Difference,
        BoolOp::Intersect => OverlayRule::Intersect,
        BoolOp::Exclude => OverlayRule::Xor,
    };
    let mut acc = path_to_shape(&paths[0]);
    for p in &paths[1..] {
        let clip = path_to_shape(p);
        let shapes = acc.overlay(&clip, rule, FillRule::NonZero);
        acc = shapes.into_iter().flatten().collect();
    }
    if acc.is_empty() { return Err(GeomError::Degenerate); }
    Ok(shape_to_path(&acc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Path as GPath;

    #[test]
    fn union_of_overlapping_rects_is_larger_than_either() {
        let a = GPath::from_svg("M0,0 L10,0 L10,10 L0,10 Z").unwrap();
        let b = GPath::from_svg("M5,5 L15,5 L15,15 L5,15 Z").unwrap();
        let u = boolean(BoolOp::Union, &[a, b]).unwrap();
        let bounds = u.bounds();
        assert_eq!((bounds.x, bounds.y, bounds.w, bounds.h), (0.0, 0.0, 15.0, 15.0));
    }

    #[test]
    fn subtract_removes_overlap_leaving_notched_bounds() {
        let a = GPath::from_svg("M0,0 L10,0 L10,10 L0,10 Z").unwrap();
        let b = GPath::from_svg("M5,5 L15,5 L15,15 L5,15 Z").unwrap();
        let d = boolean(BoolOp::Subtract, &[a, b]).unwrap();
        // a minus b: bounds still 0..10 (the notch is interior), but area shrinks.
        let bounds = d.bounds();
        assert_eq!((bounds.x, bounds.y, bounds.w, bounds.h), (0.0, 0.0, 10.0, 10.0));
        assert!(!d.segs.is_empty());
    }

    #[test]
    fn intersect_of_overlapping_rects_is_the_overlap_region() {
        let a = GPath::from_svg("M0,0 L10,0 L10,10 L0,10 Z").unwrap();
        let b = GPath::from_svg("M5,5 L15,5 L15,15 L5,15 Z").unwrap();
        let i = boolean(BoolOp::Intersect, &[a, b]).unwrap();
        let bounds = i.bounds();
        assert_eq!((bounds.x, bounds.y, bounds.w, bounds.h), (5.0, 5.0, 5.0, 5.0));
    }

    #[test]
    fn exclude_of_disjoint_rects_keeps_both_as_separate_contours() {
        let a = GPath::from_svg("M0,0 L10,0 L10,10 L0,10 Z").unwrap();
        let b = GPath::from_svg("M20,0 L30,0 L30,10 L20,10 Z").unwrap();
        let x = boolean(BoolOp::Exclude, &[a, b]).unwrap();
        let bounds = x.bounds();
        assert_eq!((bounds.x, bounds.y, bounds.w, bounds.h), (0.0, 0.0, 30.0, 10.0));
        // two disjoint rects xor'd stay disjoint -> two Move commands.
        let moves = x.segs.iter().filter(|s| matches!(s, Seg::Move(_))).count();
        assert_eq!(moves, 2);
    }

    #[test]
    fn single_input_path_is_degenerate() {
        let a = GPath::from_svg("M0,0 L10,0 L10,10 L0,10 Z").unwrap();
        assert_eq!(boolean(BoolOp::Union, &[a]), Err(GeomError::Degenerate));
    }
}
