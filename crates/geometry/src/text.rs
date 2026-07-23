// SPDX-License-Identifier: GPL-3.0-or-later
use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::affine::Point;
use crate::path::{GeomError, Path, Seg};

/// Glyph outline -> Path segs, converting font units to mm and quads to cubics.
/// Font y is up (ascender positive); we negate to match the rest of the crate's
/// y-down convention, keeping the baseline at y=0.
struct GlyphOutline<'a> {
    segs: &'a mut Vec<Seg>,
    origin_x: f64,
    scale: f64,
    cur: Point,
    start: Point,
}

impl GlyphOutline<'_> {
    fn pt(&self, x: f32, y: f32) -> Point {
        Point { x: self.origin_x + x as f64 * self.scale, y: -(y as f64) * self.scale }
    }
}

impl OutlineBuilder for GlyphOutline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.pt(x, y);
        self.segs.push(Seg::Move(p));
        self.cur = p; self.start = p;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.pt(x, y);
        self.segs.push(Seg::Line(p));
        self.cur = p;
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        // elevate quadratic -> cubic: c1 = p0 + 2/3(q-p0), c2 = p1 + 2/3(q-p1)
        let q = self.pt(x1, y1);
        let end = self.pt(x, y);
        let c1 = Point { x: self.cur.x + 2.0 / 3.0 * (q.x - self.cur.x), y: self.cur.y + 2.0 / 3.0 * (q.y - self.cur.y) };
        let c2 = Point { x: end.x + 2.0 / 3.0 * (q.x - end.x), y: end.y + 2.0 / 3.0 * (q.y - end.y) };
        self.segs.push(Seg::Cubic(c1, c2, end));
        self.cur = end;
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let c1 = self.pt(x1, y1);
        let c2 = self.pt(x2, y2);
        let end = self.pt(x, y);
        self.segs.push(Seg::Cubic(c1, c2, end));
        self.cur = end;
    }
    fn close(&mut self) {
        self.segs.push(Seg::Close);
        self.cur = self.start;
    }
}

/// Glyph outlines for `text` set in `family` at `size_mm` (font units-per-em -> size_mm).
/// Simple per-character glyph lookup + horizontal advance, no kerning/shaping (ponytail:
/// good enough for laser-cut labels; add rustybuzz shaping if ligatures/kerning matter later).
pub fn text_to_path(family: &str, size_mm: f64, text: &str) -> Result<Path, GeomError> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let query = fontdb::Query { families: &[fontdb::Family::Name(family)], ..Default::default() };
    let id = db.query(&query).ok_or(GeomError::NoFont)?;
    db.with_face_data(id, |data, face_index| -> Result<Path, GeomError> {
        let face = Face::parse(data, face_index).map_err(|_| GeomError::NoFont)?;
        let scale = size_mm / face.units_per_em() as f64;
        let mut segs = vec![];
        let mut x = 0.0f64;
        for ch in text.chars() {
            let gid = match face.glyph_index(ch) {
                Some(g) if g != GlyphId(0) => g,
                _ => { x += size_mm * 0.3; continue; } // missing glyph: skip outline, advance a fallback space
            };
            let mut builder = GlyphOutline {
                segs: &mut segs, origin_x: x, scale,
                cur: Point { x: 0.0, y: 0.0 }, start: Point { x: 0.0, y: 0.0 },
            };
            face.outline_glyph(gid, &mut builder);
            let adv = face.glyph_hor_advance(gid).unwrap_or(0) as f64;
            x += adv * scale;
        }
        Ok(Path { segs })
    }).ok_or(GeomError::NoFont)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helvetica_text_produces_nonempty_positive_bounds_path() {
        // macOS ships Helvetica; treat a missing font here as an environment gap, not a code bug.
        match text_to_path("Helvetica", 10.0, "Ab") {
            Ok(p) => {
                assert!(!p.segs.is_empty());
                let b = p.bounds();
                assert!(b.w > 0.0, "width was {}", b.w);
                assert!(b.h > 0.0, "height was {}", b.h);
            }
            Err(GeomError::NoFont) => panic!("Helvetica not found on this machine (font discovery environment gap)"),
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }

    #[test]
    fn unknown_family_is_no_font_error() {
        assert_eq!(text_to_path("Definitely Not A Real Font Family 12345", 10.0, "x"), Err(GeomError::NoFont));
    }
}
