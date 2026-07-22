// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Serialize, Deserialize};
use crate::affine::{Point, Polyline, Rect, Affine};

#[derive(Debug, PartialEq)]
pub enum GeomError { Parse(String), Degenerate, NoFont }

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Seg { Move(Point), Line(Point), Cubic(Point, Point, Point), Close }

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
pub struct Path { pub segs: Vec<Seg> }

impl Path {
    /// Minimal SVG path parser: absolute M, L, C, Z (comma/space separated).
    pub fn from_svg(d: &str) -> Result<Path, GeomError> {
        parse_svg_path(d)
    }

    /// Emit SVG path string: M/L/C/Z commands.
    pub fn to_svg(&self) -> String {
        let mut s = String::new();
        for seg in &self.segs {
            match seg {
                Seg::Move(p) => s.push_str(&format!("M{},{} ", p.x, p.y)),
                Seg::Line(p) => s.push_str(&format!("L{},{} ", p.x, p.y)),
                Seg::Cubic(c1, c2, e) => s.push_str(&format!("C{},{} {},{} {},{} ", c1.x, c1.y, c2.x, c2.y, e.x, e.y)),
                Seg::Close => s.push_str("Z "),
            }
        }
        s.trim().to_string()
    }

    pub fn transformed(&self, m: &Affine) -> Path {
        let tp = |p: &Point| { let (x, y) = m.apply(p.x, p.y); Point { x, y } };
        Path { segs: self.segs.iter().map(|s| match s {
            Seg::Move(p) => Seg::Move(tp(p)),
            Seg::Line(p) => Seg::Line(tp(p)),
            Seg::Cubic(a, b, c) => Seg::Cubic(tp(a), tp(b), tp(c)),
            Seg::Close => Seg::Close,
        }).collect() }
    }

    pub fn bounds(&self) -> Rect {
        let pts = self.flatten(0.1);
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for poly in &pts { for p in poly {
            minx = minx.min(p.x); miny = miny.min(p.y); maxx = maxx.max(p.x); maxy = maxy.max(p.y);
        }}
        Rect { x: minx, y: miny, w: maxx - minx, h: maxy - miny }
    }

    pub fn flatten(&self, tol: f64) -> Vec<Polyline> {
        let mut out = vec![];
        let mut cur: Polyline = vec![];
        let mut start = Point { x: 0.0, y: 0.0 };
        let mut here = start;
        for s in &self.segs {
            match s {
                Seg::Move(p) => { if !cur.is_empty() { out.push(std::mem::take(&mut cur)); }
                                  cur.push(*p); start = *p; here = *p; }
                Seg::Line(p) => { cur.push(*p); here = *p; }
                Seg::Cubic(c1, c2, e) => { subdivide(here, *c1, *c2, *e, tol, &mut cur); here = *e; }
                Seg::Close => { cur.push(start); here = start; }
            }
        }
        if !cur.is_empty() { out.push(cur); }
        out
    }
}

fn subdivide(p0: Point, p1: Point, p2: Point, p3: Point, tol: f64, out: &mut Polyline) {
    // flatness: max control-point deviation from the chord
    let d1 = dist_to_line(p1, p0, p3);
    let d2 = dist_to_line(p2, p0, p3);
    if d1.max(d2) <= tol { out.push(p3); return; }
    let (l, r) = split_cubic(p0, p1, p2, p3);
    subdivide(l.0, l.1, l.2, l.3, tol, out);
    subdivide(r.0, r.1, r.2, r.3, tol, out);
}

fn mid(a: Point, b: Point) -> Point { Point { x: (a.x + b.x) / 2.0, y: (a.y + b.y) / 2.0 } }

fn split_cubic(p0: Point, p1: Point, p2: Point, p3: Point)
    -> ((Point,Point,Point,Point),(Point,Point,Point,Point)) {
    let p01 = mid(p0,p1); let p12 = mid(p1,p2); let p23 = mid(p2,p3);
    let p012 = mid(p01,p12); let p123 = mid(p12,p23); let m = mid(p012,p123);
    ((p0,p01,p012,m),(m,p123,p23,p3))
}

fn dist_to_line(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx*dx + dy*dy).sqrt();
    if len == 0.0 { return ((p.x-a.x).powi(2) + (p.y-a.y).powi(2)).sqrt(); }
    ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / len
}

fn parse_svg_path(d: &str) -> Result<Path, GeomError> {
    let mut segs = vec![];
    let mut nums: Vec<f64> = vec![];
    let mut cmd: Option<char> = None;
    let flush = |cmd: char, nums: &mut Vec<f64>, segs: &mut Vec<Seg>| -> Result<(), GeomError> {
        match cmd {
            'M' => { segs.push(Seg::Move(Point { x: nums[0], y: nums[1] })); }
            'L' => { segs.push(Seg::Line(Point { x: nums[0], y: nums[1] })); }
            'C' => { segs.push(Seg::Cubic(
                Point{x:nums[0],y:nums[1]}, Point{x:nums[2],y:nums[3]}, Point{x:nums[4],y:nums[5]})); }
            'Z' => { segs.push(Seg::Close); }
            _ => return Err(GeomError::Parse(format!("cmd {cmd}"))),
        }
        nums.clear(); Ok(())
    };
    let mut buf = String::new();
    for ch in d.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphabetic() {
            if let Some(c) = cmd { if !buf.is_empty() { nums.push(buf.parse().map_err(|_| GeomError::Parse("nan".into()))?); buf.clear(); } flush(c, &mut nums, &mut segs)?; }
            cmd = Some(ch);
        } else if ch == ',' || ch.is_whitespace() {
            if !buf.is_empty() { nums.push(buf.parse().map_err(|_| GeomError::Parse("nan".into()))?); buf.clear(); }
        } else { buf.push(ch); }
    }
    if let Some(c) = cmd { flush(c, &mut nums, &mut segs)?; }
    Ok(Path { segs })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_square_flattens_to_its_own_points() {
        let p = Path::from_svg("M0,0 L20,0 L20,20 L0,20 Z").unwrap();
        let polys = p.flatten(0.1);
        assert_eq!(polys.len(), 1);
        let pts: Vec<(f64,f64)> = polys[0].iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(pts, vec![(0.0,0.0),(20.0,0.0),(20.0,20.0),(0.0,20.0),(0.0,0.0)]);
    }
    #[test]
    fn cubic_flatten_stays_within_tolerance_endpoints() {
        let p = Path::from_svg("M0,0 C0,10 10,10 10,0").unwrap();
        let polys = p.flatten(0.05);
        assert_eq!(polys[0].first().unwrap().x, 0.0);
        assert_eq!(polys[0].last().unwrap(), &Point { x: 10.0, y: 0.0 });
        assert!(polys[0].len() > 2); // subdivided
    }
    #[test]
    fn transformed_bounds_shift() {
        let p = Path::from_svg("M0,0 L10,0 L10,10 L0,10 Z")
            .unwrap().transformed(&Affine::translate(5.0, 0.0));
        let b = p.bounds();
        assert_eq!((b.x, b.w), (5.0, 10.0));
    }
}
