// SPDX-License-Identifier: GPL-3.0-or-later
//! Weed borders and weed lines: the cuts that free a pass's sheet from the roll and split the
//! waste around the design into strips an operator can lift one at a time.
//!
//! Generated per pass from that pass's shapes only, because a pass is one sheet of material:
//! the red sheet's border belongs around the red shapes. The output is plain polylines that go
//! into the pass's `Job` after its shapes, so preflight checks them like anything else cut.

use geometry::{Point, Polyline};
use serde::{Deserialize, Serialize};

use crate::passes::PlannedShape;

/// Which weed lines to cut inside the border.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum WeedLines { None, Horizontal, Vertical, Both }

/// What to weed, in millimetres. One value per cut, applied to every pass in it.
/// (`ponytail:` no per-pass options. Ceiling: one cut cannot border the red sheet and leave the
/// blue one bare. Upgrade: a weed field on the dialog's pass row.)
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct WeedOptions {
    /// How far the border stands off the pass's bounds.
    pub margin_mm: f64,
    pub lines: WeedLines,
    /// Distance between lines, measured from the border's top (or left) edge.
    pub spacing_mm: f64,
    /// The closest a line may come to any of the pass's outlines.
    pub clearance_mm: f64,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
pub struct WeedRange { pub min: f64, pub max: f64 }

impl WeedRange {
    /// False for NaN as well, so a value that is not a number is refused rather than let through.
    pub fn admits(&self, v: f64) -> bool { v >= self.min && v <= self.max }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
pub struct WeedRanges { pub margin_mm: WeedRange, pub spacing_mm: WeedRange, pub clearance_mm: WeedRange }

/// The one copy of these bounds: the cut dialog reads them over `settings_ranges`, as it does
/// the speed and force ranges, rather than restating them. Starting values, not machine facts;
/// the hardware checks in `MANUAL-CHECKLIST.md` are where they get tuned.
pub const WEED_RANGES: WeedRanges = WeedRanges {
    margin_mm: WeedRange { min: 0.5, max: 50.0 },
    spacing_mm: WeedRange { min: 5.0, max: 500.0 },
    clearance_mm: WeedRange { min: 0.2, max: 20.0 },
};

/// A line piece shorter than this is dropped: it weeds nothing and costs a blade lift.
pub const MIN_WEED_PIECE_MM: f64 = 2.0;

/// Weed options refused, as a sentence naming the field. Never clamped: a margin quietly cut to
/// 50 mm is a border the operator did not ask for.
#[derive(Clone, Debug, PartialEq)]
pub struct WeedError(pub String);

impl std::fmt::Display for WeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}
impl std::error::Error for WeedError {}

impl WeedOptions {
    pub fn validate(&self) -> Result<(), WeedError> {
        let check = |name: &str, v: f64, r: WeedRange| {
            if r.admits(v) { Ok(()) } else {
                Err(WeedError(format!("the weed {name} must be {}–{} mm", r.min, r.max)))
            }
        };
        check("margin", self.margin_mm, WEED_RANGES.margin_mm)?;
        // Spacing and clearance only shape lines, so a border-only cut is not refused over
        // fields that would not be used.
        if self.lines == WeedLines::None {
            return Ok(());
        }
        check("line spacing", self.spacing_mm, WEED_RANGES.spacing_mm)?;
        check("line clearance", self.clearance_mm, WEED_RANGES.clearance_mm)?;
        // A line runs to the border, so a clearance as wide as the margin would cut it off
        // against the border itself.
        if self.clearance_mm >= self.margin_mm {
            return Err(WeedError("the weed line clearance must be less than the margin".into()));
        }
        Ok(())
    }
}

/// The weed geometry for one pass: lines first, then the border, which is cut last so the sheet
/// stays held while the blade still has detail to cut. Nothing for a pass with no points, or
/// with bounds that are not finite: preflight then refuses the shape by its id.
/// `opts` is assumed valid (`WeedOptions::validate`).
pub fn weed_geometry(shapes: &[PlannedShape], opts: &WeedOptions) -> Vec<Polyline> {
    let Some((x0, y0, x1, y1)) = bounds(shapes) else { return vec![] };
    let m = opts.margin_mm;
    let (left, top, right, bottom) = (x0 - m, y0 - m, x1 + m, y1 + m);
    let mut out = vec![];
    if matches!(opts.lines, WeedLines::Horizontal | WeedLines::Both) {
        let outlines: Vec<Vec<&Polyline>> = shapes.iter().map(|s| s.polylines.iter().collect()).collect();
        for y in steps(top, bottom, opts.spacing_mm) {
            for (a, b) in pieces(&outlines, y, left, right, opts.clearance_mm) {
                out.push(vec![Point { x: a, y }, Point { x: b, y }]);
            }
        }
    }
    if matches!(opts.lines, WeedLines::Vertical | WeedLines::Both) {
        // The same scan with the axes swapped, so one clipping routine serves both directions.
        let swapped: Vec<Vec<Polyline>> = shapes.iter()
            .map(|s| s.polylines.iter().map(|p| p.iter().map(|q| Point { x: q.y, y: q.x }).collect()).collect())
            .collect();
        let outlines: Vec<Vec<&Polyline>> = swapped.iter().map(|s| s.iter().collect()).collect();
        for x in steps(left, right, opts.spacing_mm) {
            for (a, b) in pieces(&outlines, x, top, bottom, opts.clearance_mm) {
                out.push(vec![Point { x, y: a }, Point { x, y: b }]);
            }
        }
    }
    out.push(vec![
        Point { x: left, y: top },
        Point { x: right, y: top },
        Point { x: right, y: bottom },
        Point { x: left, y: bottom },
        Point { x: left, y: top },
    ]);
    out
}

fn bounds(shapes: &[PlannedShape]) -> Option<(f64, f64, f64, f64)> {
    let mut b: Option<(f64, f64, f64, f64)> = None;
    for p in shapes.iter().flat_map(|s| s.polylines.iter()).flatten() {
        if !p.x.is_finite() || !p.y.is_finite() {
            return None;
        }
        b = Some(match b {
            None => (p.x, p.y, p.x, p.y),
            Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
        });
    }
    b
}

/// Line positions strictly between `from` and `to`, `spacing` apart starting from `from`. A line
/// on the border's own edge would cut along it twice.
fn steps(from: f64, to: f64, spacing: f64) -> impl Iterator<Item = f64> {
    (1..).map(move |k| from + k as f64 * spacing).take_while(move |&v| v < to - 1e-9)
}

/// The parts of the horizontal line `y` between `lo` and `hi` that keep `clearance` from every
/// outline and lie outside every shape's filled area.
fn pieces(shapes: &[Vec<&Polyline>], y: f64, lo: f64, hi: f64, clearance: f64) -> Vec<(f64, f64)> {
    let mut blocked: Vec<(f64, f64)> = vec![];
    for seg in shapes.iter().flatten().flat_map(|p| p.windows(2)) {
        if let Some(r) = band_span(seg[0], seg[1], y, clearance) {
            blocked.push(r);
        }
    }
    blocked.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut free = vec![];
    let mut at = lo;
    for (a, b) in blocked {
        if a > at {
            free.push((at, a.min(hi)));
        }
        at = at.max(b);
        if at >= hi {
            break;
        }
    }
    if at < hi {
        free.push((at, hi));
    }
    // A free piece crosses no outline (crossing one is within clearance of it), so it is wholly
    // inside or wholly outside each shape, and its midpoint says which.
    free.into_iter()
        .filter(|&(a, b)| b - a >= MIN_WEED_PIECE_MM)
        .filter(|&(a, b)| {
            let mid = Point { x: (a + b) / 2.0, y };
            !shapes.iter().any(|outline| inside_even_odd(outline, mid))
        })
        .collect()
}

/// The x-range on line `y` that segment `a`–`b` blocks: its part inside the band
/// `[y - c, y + c]`, widened by `c` on each side. Every line point within `c` of the segment
/// lies in that range, so it over-blocks slightly and never under-blocks.
fn band_span(a: Point, b: Point, y: f64, c: f64) -> Option<(f64, f64)> {
    let (lo, hi) = (y - c, y + c);
    if a.y.max(b.y) < lo || a.y.min(b.y) > hi {
        return None;
    }
    let dy = b.y - a.y;
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    if dy.abs() > 1e-12 {
        let (ta, tb) = ((lo - a.y) / dy, (hi - a.y) / dy);
        t0 = t0.max(ta.min(tb));
        t1 = t1.min(ta.max(tb));
    }
    let xa = a.x + (b.x - a.x) * t0;
    let xb = a.x + (b.x - a.x) * t1;
    Some((xa.min(xb) - c, xa.max(xb) + c))
}

/// Whether `p` lies inside a shape's filled area by the even-odd rule, over its closed polylines
/// only: an open path has no inside. Even-odd, so the counter of an O is outside and the waste
/// there is weeded too.
fn inside_even_odd(outline: &[&Polyline], p: Point) -> bool {
    let mut inside = false;
    for poly in outline.iter().filter(|q| q.len() > 2 && q.first() == q.last()) {
        for seg in poly.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            if (a.y > p.y) != (b.y > p.y) {
                let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
                if x > p.x {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use document::NodeId;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Polyline {
        vec![
            Point { x, y }, Point { x: x + w, y }, Point { x: x + w, y: y + h },
            Point { x, y: y + h }, Point { x, y },
        ]
    }

    fn shape(polylines: Vec<Polyline>) -> PlannedShape { PlannedShape { node_id: NodeId(1), polylines } }

    fn opts(lines: WeedLines) -> WeedOptions {
        WeedOptions { margin_mm: 3.0, lines, spacing_mm: 10.0, clearance_mm: 1.5 }
    }

    fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len2 = dx * dx + dy * dy;
        let t = if len2 == 0.0 { 0.0 } else { (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0) };
        ((p.x - a.x - t * dx).powi(2) + (p.y - a.y - t * dy).powi(2)).sqrt()
    }

    /// Brute force, independent of the band arithmetic: sample each line piece densely and take
    /// the true distance to every segment of every outline.
    fn closest_approach(lines: &[Polyline], shapes: &[PlannedShape]) -> f64 {
        let mut min = f64::INFINITY;
        for l in lines {
            let (a, b) = (l[0], l[1]);
            for i in 0..=400 {
                let t = i as f64 / 400.0;
                let p = Point { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
                for seg in shapes.iter().flat_map(|s| s.polylines.iter()).flat_map(|q| q.windows(2)) {
                    min = min.min(dist_to_segment(p, seg[0], seg[1]));
                }
            }
        }
        min
    }

    #[test]
    fn the_border_is_the_bounds_grown_by_the_margin_and_comes_last() {
        let shapes = [shape(vec![rect(10.0, 20.0, 30.0, 5.0)]), shape(vec![rect(50.0, 22.0, 4.0, 10.0)])];
        let out = weed_geometry(&shapes, &opts(WeedLines::None));
        assert_eq!(out, vec![rect(7.0, 17.0, 50.0, 18.0)]);
    }

    #[test]
    fn no_geometry_and_non_finite_bounds_give_nothing() {
        assert!(weed_geometry(&[], &opts(WeedLines::Both)).is_empty());
        let bad = shape(vec![vec![Point { x: 0.0, y: 0.0 }, Point { x: f64::NAN, y: 1.0 }]]);
        assert!(weed_geometry(&[bad], &opts(WeedLines::Both)).is_empty());
    }

    #[test]
    fn lines_are_spaced_from_the_border_edge_and_stop_short_of_the_far_edge() {
        // Two small rects far apart: the border spans 0..100 x 0..40 with margin 3.
        let shapes = [shape(vec![rect(3.0, 3.0, 2.0, 2.0)]), shape(vec![rect(95.0, 35.0, 2.0, 2.0)])];
        let out = weed_geometry(&shapes, &opts(WeedLines::Horizontal));
        let ys: Vec<f64> = out[..out.len() - 1].iter().map(|l| l[0].y).collect();
        let mut distinct = ys.clone();
        distinct.dedup();
        assert_eq!(distinct, vec![10.0, 20.0, 30.0]);
        for l in &out[..out.len() - 1] {
            assert!(l[0].x >= 0.0 && l[1].x <= 100.0 && l[0].x < l[1].x, "{l:?}");
        }
    }

    #[test]
    fn a_line_through_a_row_of_shapes_is_cut_into_the_gaps_between_them() {
        // Three 10 mm squares 10 mm apart on y 0..10; the line at y = 7 runs through all three.
        let shapes: Vec<PlannedShape> = (0..3).map(|i| shape(vec![rect(20.0 * i as f64, 0.0, 10.0, 10.0)])).collect();
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 10.0, clearance_mm: 1.5 };
        let out = weed_geometry(&shapes, &o);
        let lines = &out[..out.len() - 1];
        // Only the gaps survive: 10..20 and 30..40, each narrowed by the clearance.
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!((lines[0][0].x, lines[0][1].x), (11.5, 18.5));
        assert_eq!((lines[1][0].x, lines[1][1].x), (31.5, 38.5));
        assert!(closest_approach(lines, &shapes) >= 1.5 - 1e-9);
    }

    #[test]
    fn no_line_piece_comes_within_the_clearance_of_any_outline() {
        // A diagonal and a triangle, so the band arithmetic meets slanted segments.
        let tri = vec![
            Point { x: 10.0, y: 10.0 }, Point { x: 60.0, y: 15.0 }, Point { x: 30.0, y: 70.0 },
            Point { x: 10.0, y: 10.0 },
        ];
        let diag = vec![Point { x: 70.0, y: 5.0 }, Point { x: 120.0, y: 80.0 }];
        let shapes = [shape(vec![tri]), shape(vec![diag])];
        let o = WeedOptions { margin_mm: 4.0, lines: WeedLines::Both, spacing_mm: 5.0, clearance_mm: 2.0 };
        let out = weed_geometry(&shapes, &o);
        let lines = &out[..out.len() - 1];
        assert!(lines.len() > 10, "expected lines around the shapes, got {}", lines.len());
        assert!(closest_approach(lines, &shapes) >= 2.0 - 1e-9);
    }

    #[test]
    fn no_piece_inside_a_filled_shape_but_one_inside_a_hole_stays() {
        // A 60 mm square ring with a 40 mm hole: the line at y = 30 crosses ring, hole, ring.
        let ring = shape(vec![rect(0.0, 0.0, 60.0, 60.0), rect(10.0, 10.0, 40.0, 40.0)]);
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 33.0, clearance_mm: 1.5 };
        let out = weed_geometry(&[ring], &o);
        let lines = &out[..out.len() - 1];
        // y = 30: the 1.5 mm strips between border and ring are under 2 mm, the ring's body is
        // inside, the hole is outside by even-odd. Only the hole survives.
        assert_eq!(lines, &[vec![Point { x: 11.5, y: 30.0 }, Point { x: 48.5, y: 30.0 }]]);
    }

    #[test]
    fn an_open_path_blocks_by_clearance_only() {
        // An open U: the line through its arms keeps the piece between them, since nothing is filled.
        let u = vec![
            Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 40.0 }, Point { x: 40.0, y: 40.0 }, Point { x: 40.0, y: 0.0 },
        ];
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 23.0, clearance_mm: 1.5 };
        let out = weed_geometry(&[shape(vec![u])], &o);
        let lines = &out[..out.len() - 1];
        // The strips outside the arms are 1.5 mm, under the minimum.
        assert_eq!(lines, &[vec![Point { x: 1.5, y: 20.0 }, Point { x: 38.5, y: 20.0 }]]);
    }

    #[test]
    fn pieces_shorter_than_the_minimum_are_dropped() {
        // Squares 4 mm apart with clearance 1.5 leave a 1 mm gap: too short to keep.
        let shapes = [shape(vec![rect(0.0, 0.0, 10.0, 10.0)]), shape(vec![rect(14.0, 0.0, 10.0, 10.0)])];
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 8.0, clearance_mm: 1.5 };
        let out = weed_geometry(&shapes, &o);
        assert_eq!(out.len(), 1, "only the border: {out:?}");
    }

    #[test]
    fn vertical_lines_mirror_horizontal_ones() {
        let shapes: Vec<PlannedShape> = (0..3).map(|i| shape(vec![rect(0.0, 20.0 * i as f64, 10.0, 10.0)])).collect();
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Vertical, spacing_mm: 10.0, clearance_mm: 1.5 };
        let out = weed_geometry(&shapes, &o);
        let lines = &out[..out.len() - 1];
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!((lines[0][0], lines[0][1]), (Point { x: 7.0, y: 11.5 }, Point { x: 7.0, y: 18.5 }));
        assert!(closest_approach(lines, &shapes) >= 1.5 - 1e-9);
    }

    #[test]
    fn options_are_refused_at_each_edge_with_a_sentence() {
        let ok = WeedOptions { margin_mm: 3.0, lines: WeedLines::Both, spacing_mm: 25.0, clearance_mm: 1.5 };
        assert_eq!(ok.validate(), Ok(()));
        let cases: [(WeedOptions, &str); 8] = [
            (WeedOptions { margin_mm: 0.4, ..ok }, "the weed margin must be 0.5–50 mm"),
            (WeedOptions { margin_mm: 50.1, ..ok }, "the weed margin must be 0.5–50 mm"),
            (WeedOptions { margin_mm: f64::NAN, ..ok }, "the weed margin must be 0.5–50 mm"),
            (WeedOptions { spacing_mm: 4.9, ..ok }, "the weed line spacing must be 5–500 mm"),
            (WeedOptions { spacing_mm: 500.1, ..ok }, "the weed line spacing must be 5–500 mm"),
            (WeedOptions { clearance_mm: 0.1, ..ok }, "the weed line clearance must be 0.2–20 mm"),
            (WeedOptions { margin_mm: 20.0, clearance_mm: 20.1, ..ok }, "the weed line clearance must be 0.2–20 mm"),
            (WeedOptions { clearance_mm: 3.0, ..ok }, "the weed line clearance must be less than the margin"),
        ];
        for (o, want) in cases {
            assert_eq!(o.validate().unwrap_err().to_string(), want, "{o:?}");
        }
        // The edges themselves are admitted.
        assert_eq!(WeedOptions { margin_mm: 0.5, lines: WeedLines::None, ..ok }.validate(), Ok(()));
        assert_eq!(WeedOptions { margin_mm: 50.0, spacing_mm: 500.0, clearance_mm: 20.0, ..ok }.validate(), Ok(()));
        assert_eq!(WeedOptions { spacing_mm: 5.0, clearance_mm: 0.2, ..ok }.validate(), Ok(()));
    }

    #[test]
    fn a_border_only_cut_ignores_the_line_fields() {
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::None, spacing_mm: 0.0, clearance_mm: 99.0 };
        assert_eq!(o.validate(), Ok(()));
    }
}
