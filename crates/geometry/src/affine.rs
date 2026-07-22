// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Serialize, Deserialize};

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Point { pub x: f64, pub y: f64 }
pub type Polyline = Vec<Point>;

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }

/// Row-major 2x3 affine: [a b c d e f] → x' = a x + c y + e, y' = b x + d y + f.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub fn identity() -> Affine { Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]) }
    pub fn translate(dx: f64, dy: f64) -> Affine { Affine([1.0, 0.0, 0.0, 1.0, dx, dy]) }
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }
    /// self.then(other) = apply self, then other.
    pub fn then(&self, other: &Affine) -> Affine {
        let [a1, b1, c1, d1, e1, f1] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Affine([
            a2 * a1 + c2 * b1,        b2 * a1 + d2 * b1,
            a2 * c1 + c2 * d1,        b2 * c1 + d2 * d1,
            a2 * e1 + c2 * f1 + e2,   b2 * e1 + d2 * f1 + f2,
        ])
    }
    /// None for singular (non-invertible) transforms.
    pub fn inverse(&self) -> Option<Affine> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det == 0.0 { return None; }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Some(Affine([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translate_then_apply() {
        let m = Affine::translate(3.0, -2.0);
        assert_eq!(m.apply(1.0, 1.0), (4.0, -1.0));
    }
    #[test]
    fn then_composes_left_to_right_of_argument() {
        let t = Affine::translate(5.0, 0.0);
        let composed = t.then(&Affine::translate(0.0, 2.0)); // apply t, then +2y
        assert_eq!(composed.apply(0.0, 0.0), (5.0, 2.0));
    }
    #[test]
    fn inverse_undoes_translate() {
        let m = Affine::translate(7.0, 9.0);
        let back = m.inverse().unwrap();
        let (fx, fy) = m.apply(1.0, 1.0);
        assert_eq!(back.apply(fx, fy), (1.0, 1.0));
    }
    #[test]
    fn singular_inverse_is_none() {
        assert_eq!(Affine([0.0; 6]).inverse(), None);
    }
}
