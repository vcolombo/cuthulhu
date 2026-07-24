// SPDX-License-Identifier: GPL-3.0-or-later
use crate::affine::Point;
use crate::path::{Path, Seg};

/// Circle-arc cubic-Bezier approximation constant (4/3 * (sqrt(2)-1)).
const KAPPA: f64 = 0.5522847498;

/// Axis-aligned rect path, top-left at (x, y), CCW winding.
pub fn rect_path(x: f64, y: f64, w: f64, h: f64) -> Path {
    Path { segs: vec![
        Seg::Move(Point { x, y }),
        Seg::Line(Point { x: x + w, y }),
        Seg::Line(Point { x: x + w, y: y + h }),
        Seg::Line(Point { x, y: y + h }),
        Seg::Close,
    ] }
}

/// Ellipse path centered at (cx, cy), four cubic-Bezier quarter arcs.
pub fn ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64) -> Path {
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    Path { segs: vec![
        Seg::Move(Point { x: cx + rx, y: cy }),
        Seg::Cubic(Point { x: cx + rx, y: cy + ky }, Point { x: cx + kx, y: cy + ry }, Point { x: cx, y: cy + ry }),
        Seg::Cubic(Point { x: cx - kx, y: cy + ry }, Point { x: cx - rx, y: cy + ky }, Point { x: cx - rx, y: cy }),
        Seg::Cubic(Point { x: cx - rx, y: cy - ky }, Point { x: cx - kx, y: cy - ry }, Point { x: cx, y: cy - ry }),
        Seg::Cubic(Point { x: cx + kx, y: cy - ry }, Point { x: cx + rx, y: cy - ky }, Point { x: cx + rx, y: cy }),
        Seg::Close,
    ] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_path_bounds_match_inputs_exactly() {
        let p = rect_path(1.0, 2.0, 10.0, 4.0);
        let b = p.bounds();
        assert_eq!((b.x, b.y, b.w, b.h), (1.0, 2.0, 10.0, 4.0));
    }

    #[test]
    fn ellipse_path_bounds_approximate_the_radii() {
        let p = ellipse_path(0.0, 0.0, 10.0, 5.0);
        let b = p.bounds();
        // kappa cubic approximation can deviate slightly either side of the true ellipse,
        // so bounds are close but not exact.
        assert!((b.x - -10.0).abs() < 0.05, "x={}", b.x);
        assert!((b.y - -5.0).abs() < 0.05, "y={}", b.y);
        assert!((b.w - 20.0).abs() < 0.05, "w={}", b.w);
        assert!((b.h - 10.0).abs() < 0.05, "h={}", b.h);
    }
}
