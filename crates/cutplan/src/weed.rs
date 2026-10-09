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
// ponytail: no per-pass options. Ceiling: one cut cannot border the red sheet and leave the blue
// one bare. Upgrade: a weed field on the dialog's pass row.
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

/// Where the dialog and the CLI start: a border only. Starting values like the ranges, tuned by the
/// hardware checks, and kept here so neither caller restates them.
pub const WEED_DEFAULTS: WeedOptions =
    WeedOptions { margin_mm: 3.0, lines: WeedLines::None, spacing_mm: 25.0, clearance_mm: 1.5 };

/// A line piece shorter than this is dropped: it weeds nothing and costs a blade lift.
pub const MIN_WEED_PIECE_MM: f64 = 2.0;

/// The most weed lines one pass may have on one axis. Lines are generated before preflight sees
/// the geometry, so a pass a kilometre wide (a stray scale, a huge SVG width) would otherwise
/// build millions of them while the desktop holds the document lock. Ten thousand at the 5 mm
/// minimum spacing is a 50 m roll, past any cutter here.
pub const MAX_WEED_LINES: f64 = 10_000.0;

/// The most segment tests one pass's lines may cost: the scan of each line against every segment,
/// and the inside test of each piece left over against the outlines whose box holds it. Charged as the work
/// is done (`Budget`), so it bounds the time whatever the shapes look like; the line cap alone left
/// a dense trace on a wide pass at billions of tests, on every keystroke in the dialog. Fifty
/// million is a fraction of a second.
pub const MAX_WEED_WORK: f64 = 50_000_000.0;

/// The most weed line pieces one pass may produce, so the output's memory is bounded too: a comb
/// of thin shapes yields a piece per gap per line.
pub const MAX_WEED_PIECES: usize = 100_000;

/// What a pass's lines may still spend. Running out refuses the pass rather than truncating it: a
/// pass with some of its lines missing would weed worse than the operator previewed, and say nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Budget { tests: f64, pieces: usize }

impl Budget {
    const FULL: Budget = Budget { tests: MAX_WEED_WORK, pieces: MAX_WEED_PIECES };
    /// At least one test per call, so even work over no segments (a pass of single points) counts.
    fn spend(&mut self, tests: usize) -> Option<()> {
        self.tests -= tests.max(1) as f64;
        (self.tests >= 0.0).then_some(())
    }
    fn keep_piece(&mut self) -> Option<()> {
        self.pieces = self.pieces.checked_sub(1)?;
        Some(())
    }
}

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
        // Both values in the sentence: from the CLI, the clearance may be a default nobody typed.
        if self.clearance_mm >= self.margin_mm {
            return Err(WeedError(format!(
                "the weed line clearance ({} mm) must be less than the margin ({} mm)",
                self.clearance_mm, self.margin_mm,
            )));
        }
        Ok(())
    }
}

/// The weed geometry for one pass: lines first, then the border, which is cut last so the sheet
/// stays held while the blade still has detail to cut. Nothing for a pass with no points, or
/// with bounds that are not finite: preflight then refuses the shape by its id. Refused when the
/// pass is so large or detailed that its lines would pass `MAX_WEED_LINES`, `MAX_WEED_WORK` or
/// `MAX_WEED_PIECES`. `opts` is assumed valid (`WeedOptions::validate`).
pub fn weed_geometry(shapes: &[PlannedShape], opts: &WeedOptions) -> Result<Vec<Polyline>, WeedError> {
    let mut budget = Budget::FULL;
    weed_geometry_within(shapes, opts, &mut budget)
}

/// The early refusal: whether a pass's lines can fit at all, before anything is built. `n <= cap`
/// is false for NaN, and the spacing must be positive, so a value nobody validated can neither
/// slip through nor send `steps` down an endless decreasing sequence.
fn lines_fit(rows: f64, columns: f64, segments: usize, spacing_mm: f64) -> bool {
    let within = |n: f64, cap: f64| n <= cap;
    spacing_mm > 0.0
        && within(rows, MAX_WEED_LINES) && within(columns, MAX_WEED_LINES)
        && within((rows + columns) * segments as f64, MAX_WEED_WORK)
}

/// `weed_geometry` against `budget`, which is left holding what was not spent. Apart so tests can
/// measure a pass's cost and set the limit exactly at it.
fn weed_geometry_within(shapes: &[PlannedShape], opts: &WeedOptions, budget: &mut Budget)
    -> Result<Vec<Polyline>, WeedError> {
    // ponytail: the border is the bounding box grown by the margin, not a contour. Ceiling: an
    // L-shaped or diagonal design leaves a lot of waste inside its box. Upgrade: a contour border
    // from #159's path offset.
    // ponytail: straight lines at a fixed spacing, each scanned against every segment and sorted,
    // and each piece tested inside against the filled shapes whose box holds it, so the cost is about
    // lines × (segments log segments + pieces × (shapes + nearby segments)), paid per keystroke in the dialog
    // (`MAX_WEED_WORK` bounds it, refusing the densest designs).
    // Ceiling: no diagonal lines, no lines that bend around shapes, and a dense comb is refused
    // rather than weeded. Upgrade: #222's line-fill engine, or a sorted sweep of each line's signed
    // crossings, which reads every piece's winding in O(crossings log crossings).
    let Some((x0, y0, x1, y1)) = bounds(shapes) else { return Ok(vec![]) };
    let m = opts.margin_mm;
    let (left, top, right, bottom) = (x0 - m, y0 - m, x1 + m, y1 + m);
    let horizontal = matches!(opts.lines, WeedLines::Horizontal | WeedLines::Both);
    let vertical = matches!(opts.lines, WeedLines::Vertical | WeedLines::Both);
    let segments = shapes.iter().flat_map(|s| &s.polylines).map(|p| p.len().saturating_sub(1)).sum::<usize>();
    let too_much = || WeedError(format!(
        "this pass ({:.0} × {:.0} mm, {segments} segments) is too large or detailed for weed lines {} mm apart; raise the spacing or leave the lines off",
        right - left, bottom - top, opts.spacing_mm,
    ));
    if horizontal || vertical {
        let across = |extent: f64, on: bool| if on { extent / opts.spacing_mm } else { 0.0 };
        let (rows, columns) = (across(bottom - top, horizontal), across(right - left, vertical));
        if !lines_fit(rows, columns, segments, opts.spacing_mm) {
            return Err(too_much());
        }
    }
    let mut out = vec![];
    if horizontal {
        let outlines: Vec<Vec<&Polyline>> = shapes.iter().map(|s| s.polylines.iter().collect()).collect();
        let fills = filled(&outlines);
        for y in steps(top, bottom, opts.spacing_mm) {
            let line = pieces(&outlines, &fills, y, left, right, opts.clearance_mm, segments, budget);
            for (a, b) in line.ok_or_else(too_much)? {
                out.push(vec![Point { x: a, y }, Point { x: b, y }]);
            }
        }
    }
    if vertical {
        // The same scan with the axes swapped, so one clipping routine serves both directions.
        let swapped: Vec<Vec<Polyline>> = shapes.iter()
            .map(|s| s.polylines.iter().map(|p| p.iter().map(|q| Point { x: q.y, y: q.x }).collect()).collect())
            .collect();
        let outlines: Vec<Vec<&Polyline>> = swapped.iter().map(|s| s.iter().collect()).collect();
        let fills = filled(&outlines);
        for x in steps(left, right, opts.spacing_mm) {
            let line = pieces(&outlines, &fills, x, top, bottom, opts.clearance_mm, segments, budget);
            for (a, b) in line.ok_or_else(too_much)? {
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
    Ok(out)
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
/// outline and lie outside every shape's filled area, or None once `budget` runs out. `counts` is
/// the pass's segment count, what the scan costs.
fn pieces(
    shapes: &[Vec<&Polyline>], fills: &[Fill], y: f64, lo: f64, hi: f64, clearance: f64,
    segments: usize, budget: &mut Budget,
) -> Option<Vec<(f64, f64)>> {
    budget.spend(segments)?;
    let mut blocked: Vec<(f64, f64)> = vec![];
    for seg in shapes.iter().flatten().flat_map(|p| p.windows(2)) {
        if let Some(r) = band_span(seg[0], seg[1], y, clearance) {
            blocked.push(r);
        }
    }
    // The sort is the scan's other cost, about b log b comparisons.
    budget.spend(blocked.len() * blocked.len().max(1).ilog2() as usize)?;
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
    // inside or wholly outside each shape, and its midpoint says which. Each shape is asked on its
    // own, since each is cut as its own piece: a hole drawn as a separate node is a disc of its own,
    // not a hole in the shape around it, so the waste between them is not weeded.
    let mut kept = vec![];
    for (a, b) in free.into_iter().filter(|&(a, b)| b - a >= MIN_WEED_PIECE_MM) {
        let mid = Point { x: (a + b) / 2.0, y };
        // One box check per shape, and a winding count only where the box holds the point: a piece
        // is near one or two shapes of a trace, not all thousand of them.
        budget.spend(fills.len())?;
        let mut inside = false;
        for fill in fills.iter().filter(|f| f.holds(mid)) {
            budget.spend(fill.segments)?;
            if winding(&fill.outlines, mid) != 0 {
                inside = true;
                break;
            }
        }
        if !inside {
            budget.keep_piece()?;
            kept.push((a, b));
        }
    }
    Some(kept)
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

/// Whether a polyline is closed: flattening pushes a subpath's exact start point on `Close`.
fn is_closed(p: &Polyline) -> bool { p.len() > 2 && p.first() == p.last() }

/// A shape's closed polylines with their box and segment count, for the inside test.
struct Fill<'a> { outlines: Vec<&'a Polyline>, x0: f64, y0: f64, x1: f64, y1: f64, segments: usize }

impl Fill<'_> {
    /// Outside its box a point winds zero, so only shapes whose box holds it are asked.
    fn holds(&self, p: Point) -> bool { p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1 }
}

/// Each shape's closed polylines, and only shapes that have one: an open path has no inside, so the
/// inside test (`winding`) never needs to visit it, and its cost is then what `Budget` charges.
fn filled<'a>(shapes: &[Vec<&'a Polyline>]) -> Vec<Fill<'a>> {
    shapes.iter()
        .filter_map(|s| {
            let outlines: Vec<&Polyline> = s.iter().copied().filter(|p| is_closed(p)).collect();
            let points = || outlines.iter().flat_map(|p| p.iter());
            let first = points().next()?;
            let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
            for q in points() {
                (x0, y0, x1, y1) = (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y));
            }
            let segments = outlines.iter().map(|p| p.len() - 1).sum();
            Some(Fill { outlines, x0, y0, x1, y1, segments })
        })
        .collect()
}

/// The winding number of a shape's closed polylines (`filled`) around `p`; non-zero is inside its filled
/// area. Non-zero, not even-odd, because that is how the app fills (`geometry::boolean`, and
/// SVG's default): a pentagram's centre is filled, and a line there would cut through the decal.
/// A glyph's counter winds the other way, so the waste inside an O is still weeded. An open path
/// has no inside.
fn winding(outline: &[&Polyline], p: Point) -> i32 {
    let mut w = 0;
    for poly in outline {
        for seg in poly.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            let side = (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y);
            if a.y <= p.y && b.y > p.y && side > 0.0 {
                w += 1;
            } else if b.y <= p.y && a.y > p.y && side < 0.0 {
                w -= 1;
            }
        }
    }
    w
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
        let out = weed_geometry(&shapes, &opts(WeedLines::None)).unwrap();
        assert_eq!(out, vec![rect(7.0, 17.0, 50.0, 18.0)]);
    }

    #[test]
    fn no_geometry_and_non_finite_bounds_give_nothing() {
        assert!(weed_geometry(&[], &opts(WeedLines::Both)).unwrap().is_empty());
        let bad = shape(vec![vec![Point { x: 0.0, y: 0.0 }, Point { x: f64::NAN, y: 1.0 }]]);
        assert!(weed_geometry(&[bad], &opts(WeedLines::Both)).unwrap().is_empty());
    }

    #[test]
    fn lines_are_spaced_from_the_border_edge_and_stop_short_of_the_far_edge() {
        // Two small rects far apart: the border spans 0..100 x 0..40 with margin 3.
        let shapes = [shape(vec![rect(3.0, 3.0, 2.0, 2.0)]), shape(vec![rect(95.0, 35.0, 2.0, 2.0)])];
        let out = weed_geometry(&shapes, &opts(WeedLines::Horizontal)).unwrap();
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
        let out = weed_geometry(&shapes, &o).unwrap();
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
        let out = weed_geometry(&shapes, &o).unwrap();
        let lines = &out[..out.len() - 1];
        assert!(lines.len() > 10, "expected lines around the shapes, got {}", lines.len());
        assert!(closest_approach(lines, &shapes) >= 2.0 - 1e-9);
    }

    /// `rect` wound the other way, as a glyph's counter is.
    fn hole(x: f64, y: f64, w: f64, h: f64) -> Polyline {
        let mut r = rect(x, y, w, h);
        r.reverse();
        r
    }

    #[test]
    fn no_piece_inside_a_filled_shape_but_one_inside_a_hole_stays() {
        // A 60 mm square ring with a 40 mm hole: the line at y = 30 crosses ring, hole, ring.
        let ring = shape(vec![rect(0.0, 0.0, 60.0, 60.0), hole(10.0, 10.0, 40.0, 40.0)]);
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 33.0, clearance_mm: 1.5 };
        let out = weed_geometry(&[ring], &o).unwrap();
        let lines = &out[..out.len() - 1];
        // y = 30: the 1.5 mm strips between border and ring are under 2 mm, the ring's body is
        // inside, and the hole winds back to zero. Only the hole survives.
        assert_eq!(lines, &[vec![Point { x: 11.5, y: 30.0 }, Point { x: 48.5, y: 30.0 }]]);
    }

    /// Non-zero, as the app fills: an inner contour wound the same way as the outer one is filled,
    /// like a pentagram's centre, so a line there would cut through the decal.
    #[test]
    fn an_inner_contour_wound_the_same_way_is_filled_not_weeded() {
        let doubled = shape(vec![rect(0.0, 0.0, 60.0, 60.0), rect(10.0, 10.0, 40.0, 40.0)]);
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 33.0, clearance_mm: 1.5 };
        let out = weed_geometry(&[doubled], &o).unwrap();
        assert_eq!(out.len(), 1, "only the border: {out:?}");
    }

    /// Two overlapping subpaths of one shape fill their overlap too.
    #[test]
    fn the_overlap_of_two_subpaths_is_filled_not_weeded() {
        let overlapping = shape(vec![rect(0.0, 0.0, 40.0, 40.0), rect(20.0, 0.0, 40.0, 40.0)]);
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 23.0, clearance_mm: 1.5 };
        let out = weed_geometry(&[overlapping], &o).unwrap();
        assert_eq!(out.len(), 1, "only the border: {out:?}");
    }

    /// Generated before preflight, so a huge pass must be refused here rather than build millions
    /// of lines under the document lock.
    #[test]
    fn a_pass_too_large_for_its_line_spacing_is_refused_before_any_line_is_built() {
        let wide = [shape(vec![rect(0.0, 0.0, 1.0e9, 10.0)])];
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Vertical, spacing_mm: 5.0, clearance_mm: 1.5 };
        assert_eq!(
            weed_geometry(&wide, &o).unwrap_err().to_string(),
            "this pass (1000000006 × 16 mm, 4 segments) is too large or detailed for weed lines 5 mm apart; raise the spacing or leave the lines off",
        );
        // Each axis counts its own lines: a tall pass refuses horizontal lines, and its few vertical
        // ones are fine.
        let tall = [shape(vec![rect(0.0, 0.0, 10.0, 1.0e9)])];
        assert!(weed_geometry(&tall, &WeedOptions { lines: WeedLines::Horizontal, ..o }).is_err());
        assert!(weed_geometry(&tall, &o).is_ok());
        // The border alone is still built: it is one polyline at any size, and preflight refuses it.
        assert_eq!(weed_geometry(&wide, &WeedOptions { lines: WeedLines::None, ..o }).unwrap().len(), 1);
        // 10 000 lines exactly is allowed: a 50 m pass at 5 mm.
        let roll = [shape(vec![rect(3.0, 3.0, 49_994.0, 10.0)])];
        assert!(weed_geometry(&roll, &o).is_ok());
        let longer = [shape(vec![rect(3.0, 3.0, 49_995.0, 10.0)])];
        assert!(weed_geometry(&longer, &o).is_err());
    }

    /// A pass of ordinary size can still be too detailed: lines × segments is the cost.
    #[test]
    fn a_pass_too_detailed_for_its_line_spacing_is_refused() {
        // 300 000 segments zigzagging inside 500 mm, and about 200 lines at 5 mm: 60 million tests.
        let trace: Polyline = (0..=300_000).map(|i| Point { x: (i % 500) as f64, y: (i % 499) as f64 }).collect();
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Both, spacing_mm: 5.0, clearance_mm: 1.5 };
        let err = weed_geometry(&[shape(vec![trace.clone()])], &o).unwrap_err().to_string();
        assert!(err.contains("300000 segments") && err.contains("too large or detailed"), "{err}");
        // A wider spacing means fewer lines and brings it under.
        assert!(weed_geometry(&[shape(vec![trace])], &WeedOptions { spacing_mm: 20.0, ..o }).is_ok());
    }

    #[test]
    fn a_spacing_that_is_not_a_positive_number_is_refused_not_looped_on() {
        for spacing_mm in [f64::NAN, -5.0, 0.0] {
            let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Both, spacing_mm, clearance_mm: 1.5 };
            assert!(weed_geometry(&[shape(vec![rect(0.0, 0.0, 10.0, 10.0)])], &o).is_err(), "{spacing_mm}");
        }
    }

    /// The early check on its own: a negative spacing gives negative line counts, which every cap
    /// admits, so only the spacing test refuses it before `steps` runs away.
    #[test]
    fn the_early_check_refuses_a_spacing_that_is_not_positive() {
        assert!(lines_fit(2.0, 2.0, 4, 5.0));
        assert!(!lines_fit(-2.0, -2.0, 4, -5.0));
        // Line counts every cap admits, so only the spacing refuses it.
        assert!(!lines_fit(1.0, 1.0, 4, 0.0));
        assert!(!lines_fit(f64::NAN, 0.0, 4, f64::NAN));
    }

    /// What a pass costs: the budget spent generating it, from a full one.
    fn cost(shapes: &[PlannedShape], o: &WeedOptions) -> Budget {
        let mut b = Budget::FULL;
        weed_geometry_within(shapes, o, &mut b).unwrap();
        Budget { tests: Budget::FULL.tests - b.tests, pieces: Budget::FULL.pieces - b.pieces }
    }

    /// A grid of closed squares, so lines in both directions find pieces and pay for inside tests.
    /// Not square, so the two directions cost differently and a charge to the wrong one would show.
    fn grid(columns: usize, rows: usize) -> Vec<PlannedShape> {
        (0..columns * rows)
            .map(|i| shape(vec![rect(6.0 * (i % columns) as f64, 6.0 * (i / columns) as f64, 1.0, 1.0)]))
            .collect()
    }

    /// Horizontal and vertical lines draw on one budget, and a pass that fits it exactly is kept.
    #[test]
    fn both_directions_share_one_budget_and_a_pass_that_fits_it_exactly_is_kept() {
        let shapes = grid(12, 8);
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Both, spacing_mm: 5.0, clearance_mm: 1.5 };
        let (h, v) = (cost(&shapes, &WeedOptions { lines: WeedLines::Horizontal, ..o }), cost(&shapes, &WeedOptions { lines: WeedLines::Vertical, ..o }));
        let both = cost(&shapes, &o);
        assert_ne!(h, v, "the grid must cost differently each way");
        assert_eq!(both, Budget { tests: h.tests + v.tests, pieces: h.pieces + v.pieces });
        assert!(both.pieces > 0 && both.tests > 0.0);

        let run = |b: Budget| weed_geometry_within(&shapes, &o, &mut b.clone());
        assert!(run(both).is_ok(), "exactly enough");
        assert!(run(Budget { tests: both.tests - 1.0, ..both }).is_err(), "one test short");
        assert!(run(Budget { pieces: both.pieces - 1, ..both }).is_err(), "one piece short");
        // Either direction alone fits a budget the two together overrun.
        let larger = Budget { tests: h.tests.max(v.tests), pieces: h.pieces.max(v.pieces) };
        assert!(run(larger).is_err());
    }

    /// The early estimate counts only each line's scan; a comb gives every line a piece per gap, and
    /// each piece an inside test against every outline. The budget charges that as it is spent.
    #[test]
    fn a_comb_that_passes_the_estimate_is_refused_by_the_work_it_actually_costs() {
        // 2000 closed 0.5 × 100 mm teeth 6 mm apart (2.5 mm left in each gap after the clearance):
        // about 21 lines, each with 2000 pieces, each tested against 8000 segments, so about 340
        // million tests while the estimate says 170 000.
        let teeth: Vec<PlannedShape> = (0..2000).map(|i| shape(vec![rect(6.0 * i as f64, 0.0, 0.5, 100.0)])).collect();
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 5.0, clearance_mm: 1.5 };
        let err = weed_geometry(&teeth, &o).unwrap_err().to_string();
        assert!(err.contains("8000 segments") && err.contains("too large or detailed"), "{err}");
    }

    /// Each line's sort is charged, not only its scan: an open zigzag that every line crosses a
    /// thousand times costs mostly sorting, which the scan and the pieces alone would not count.
    #[test]
    fn each_line_pays_for_sorting_what_blocks_it() {
        // 1000 uprights 6 mm apart, joined top and bottom alternately: about 2000 segments, about
        // 1000 of them across every line, so each sort is about 1000 × 9 comparisons.
        let zigzag: Polyline = (0..1000).flat_map(|i| {
            let x = 6.0 * i as f64;
            if i % 2 == 0 { [Point { x, y: 0.0 }, Point { x, y: 100.0 }] } else { [Point { x, y: 100.0 }, Point { x, y: 0.0 }] }
        }).collect();
        let shapes = [shape(vec![zigzag])];
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 5.0, clearance_mm: 1.5 };
        let c = cost(&shapes, &o);
        let lines = steps(-3.0, 103.0, 5.0).count() as f64;
        let segments = 1999.0;
        // Without the sort charge the cost is the scans plus one test per piece (nothing is closed,
        // so each piece pays only the minimum charge for its box checks).
        let scans_and_pieces = lines * segments + c.pieces as f64;
        assert!(c.tests > 3.0 * scans_and_pieces, "{} tests against {scans_and_pieces} for scans and pieces", c.tests);
    }

    /// A traced design at the defaults must fit the budget: 1000 closed 100-sided blobs (100 000
    /// segments) over 300 × 300 mm, lines both ways at 25 mm. Each piece's inside test must not
    /// walk every outline in the pass, or a trace like this is refused at the default spacing.
    #[test]
    fn a_dense_trace_at_the_default_spacing_is_weeded_not_refused() {
        let blobs: Vec<PlannedShape> = (0..1000).map(|i| {
            let (cx, cy) = (5.0 + 9.5 * (i % 32) as f64, 5.0 + 9.5 * (i / 32) as f64);
            let mut ring: Polyline = (0..100).map(|k| {
                let t = k as f64 * std::f64::consts::TAU / 100.0;
                Point { x: cx + 3.0 * t.cos(), y: cy + 3.0 * t.sin() }
            }).collect();
            ring.push(ring[0]);
            shape(vec![ring])
        }).collect();
        let o = WeedOptions { lines: WeedLines::Both, ..WEED_DEFAULTS };
        let c = cost(&blobs, &o);
        assert!(c.tests < MAX_WEED_WORK / 5.0, "{} tests: too close to the budget for a routine job", c.tests);
    }

    /// The output is bounded too: open sticks cost one test per piece, but there can be too many.
    #[test]
    fn a_pass_that_would_make_too_many_pieces_is_refused() {
        // 2000 open 300 mm sticks 6 mm apart, about 61 lines: about 122 000 pieces.
        let sticks: Vec<PlannedShape> = (0..2000)
            .map(|i| shape(vec![vec![Point { x: 6.0 * i as f64, y: 0.0 }, Point { x: 6.0 * i as f64, y: 300.0 }]]))
            .collect();
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 5.0, clearance_mm: 1.5 };
        assert!(weed_geometry(&sticks, &o).unwrap_err().to_string().contains("too large or detailed"));
        // Half the sticks fit.
        assert!(weed_geometry(&sticks[..1000], &o).is_ok());
    }

    #[test]
    fn an_open_path_blocks_by_clearance_only() {
        // An open U: the line through its arms keeps the piece between them, since nothing is filled.
        let u = vec![
            Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 40.0 }, Point { x: 40.0, y: 40.0 }, Point { x: 40.0, y: 0.0 },
        ];
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 23.0, clearance_mm: 1.5 };
        let out = weed_geometry(&[shape(vec![u])], &o).unwrap();
        let lines = &out[..out.len() - 1];
        // The strips outside the arms are 1.5 mm, under the minimum.
        assert_eq!(lines, &[vec![Point { x: 1.5, y: 20.0 }, Point { x: 38.5, y: 20.0 }]]);
    }

    #[test]
    fn pieces_shorter_than_the_minimum_are_dropped() {
        // Squares 4 mm apart with clearance 1.5 leave a 1 mm gap: too short to keep.
        let shapes = [shape(vec![rect(0.0, 0.0, 10.0, 10.0)]), shape(vec![rect(14.0, 0.0, 10.0, 10.0)])];
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Horizontal, spacing_mm: 8.0, clearance_mm: 1.5 };
        let out = weed_geometry(&shapes, &o).unwrap();
        assert_eq!(out.len(), 1, "only the border: {out:?}");
    }

    #[test]
    fn vertical_lines_mirror_horizontal_ones() {
        let shapes: Vec<PlannedShape> = (0..3).map(|i| shape(vec![rect(0.0, 20.0 * i as f64, 10.0, 10.0)])).collect();
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::Vertical, spacing_mm: 10.0, clearance_mm: 1.5 };
        let out = weed_geometry(&shapes, &o).unwrap();
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
            (WeedOptions { clearance_mm: 3.0, ..ok }, "the weed line clearance (3 mm) must be less than the margin (3 mm)"),
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
    fn the_defaults_are_valid_with_lines_on_too() {
        assert_eq!(WEED_DEFAULTS.validate(), Ok(()));
        assert_eq!(WeedOptions { lines: WeedLines::Both, ..WEED_DEFAULTS }.validate(), Ok(()));
    }

    #[test]
    fn a_border_only_cut_ignores_the_line_fields() {
        let o = WeedOptions { margin_mm: 3.0, lines: WeedLines::None, spacing_mm: 0.0, clearance_mm: 99.0 };
        assert_eq!(o.validate(), Ok(()));
    }
}
