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

/// Sorted, deduped primary family names of every installed face. Fresh scan per
/// call — a picker enumerates once per dialog open, and the scan is a directory
/// walk, not something worth caching against font installs mid-session.
pub fn list_font_families() -> Vec<String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    // Primary name only: localized aliases would show as duplicates in a picker,
    // and the query below matches any alias, so a primary name always round-trips.
    let mut names: Vec<String> =
        db.faces().filter_map(|f| f.families.first().map(|(name, _)| name.clone())).collect();
    names.sort_unstable(); // ponytail: locale-aware collation if anyone ever cares
    names.dedup();
    names
}

/// Exact family first, then the generic sans-serif alias (fontconfig may remap it),
/// then any installed face at all. The last step is what guarantees text renders on
/// systems without the requested family: substituting a font the operator can see
/// beats refusing, and preview and cut plan resolve through this same function, so
/// what gets cut is what was shown.
fn resolve_face(db: &fontdb::Database, family: &str) -> Option<fontdb::ID> {
    db.query(&fontdb::Query {
        families: &[fontdb::Family::Name(family), fontdb::Family::SansSerif],
        ..Default::default()
    })
    .or_else(|| db.faces().next().map(|f| f.id))
}

/// Split out of `text_to_path`'s closure so the BadFont branch is testable: fontdb
/// parses files on insert and refuses corrupt ones, so garbage face data can only be
/// fed in through this seam.
fn outline_with_face(data: &[u8], face_index: u32, size_mm: f64, text: &str) -> Result<Path, GeomError> {
    let face = Face::parse(data, face_index).map_err(|_| GeomError::BadFont)?;
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
    // A face that drew nothing for text that asked for something (a symbol font and
    // Latin letters, a bitmap-only face) would otherwise persist as an invisible empty
    // node that plans as an empty job. Whitespace-only text legitimately has no outlines.
    if segs.is_empty() && text.chars().any(|c| !c.is_whitespace()) {
        return Err(GeomError::NoGlyphs);
    }
    Ok(Path { segs })
}

/// Glyph outlines for `text` set in `family` at `size_mm` (font units-per-em -> size_mm).
/// A family with no installed match silently substitutes via `resolve_face`; only a
/// system with zero faces is `NoFont`, a matched face that won't parse is `BadFont`,
/// and a face that draws nothing for non-whitespace text is `NoGlyphs`.
/// Simple per-character glyph lookup + horizontal advance, no kerning/shaping (ponytail:
/// good enough for laser-cut labels; add rustybuzz shaping if ligatures/kerning matter later).
pub fn text_to_path(family: &str, size_mm: f64, text: &str) -> Result<Path, GeomError> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let id = resolve_face(&db, family).ok_or(GeomError::NoFont)?;
    db.with_face_data(id, |data, face_index| outline_with_face(data, face_index, size_mm, text))
        .ok_or(GeomError::BadFont)?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Picks whatever font family is actually installed, instead of hardcoding "Helvetica"
    /// (macOS-only). Returns None on a headless CI box with zero system faces.
    fn any_available_family() -> Option<String> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let name = db.faces().next().and_then(|f| f.families.first().map(|(name, _)| name.clone()));
        name
    }

    #[test]
    fn available_font_text_produces_nonempty_positive_bounds_path() {
        match any_available_family() {
            Some(family) => match text_to_path(&family, 10.0, "Ab") {
                Ok(p) => {
                    assert!(!p.segs.is_empty());
                    let b = p.bounds();
                    assert!(b.w > 0.0, "width was {}", b.w);
                    assert!(b.h > 0.0, "height was {}", b.h);
                }
                Err(e) => panic!("unexpected error for family {family:?}: {e:?}"),
            },
            // headless CI with zero system fonts: assert the real NoFont path instead.
            None => assert_eq!(text_to_path("Whatever", 10.0, "Ab"), Err(GeomError::NoFont)),
        }
    }

    #[test]
    fn unknown_family_falls_back_to_an_installed_font() {
        match any_available_family() {
            Some(_) => match text_to_path("Definitely Not A Real Font Family 12345", 10.0, "Ab") {
                Ok(p) => {
                    assert!(!p.segs.is_empty());
                    let b = p.bounds();
                    assert!(b.w > 0.0, "width was {}", b.w);
                    assert!(b.h > 0.0, "height was {}", b.h);
                }
                Err(e) => panic!("fallback should have substituted a font: {e:?}"),
            },
            None => assert_eq!(
                text_to_path("Definitely Not A Real Font Family 12345", 10.0, "Ab"),
                Err(GeomError::NoFont)
            ),
        }
    }

    #[test]
    fn garbage_face_data_is_bad_font_not_no_font() {
        assert_eq!(outline_with_face(b"not a font", 0, 10.0, "x"), Err(GeomError::BadFont));
    }

    #[test]
    fn whitespace_only_text_is_an_empty_path_not_an_error() {
        if any_available_family().is_some() {
            let p = text_to_path("Whatever", 10.0, "   ").expect("whitespace has no outlines by design");
            assert!(p.segs.is_empty());
        }
    }

    #[test]
    fn text_no_face_can_draw_is_no_glyphs_not_an_invisible_node() {
        // Unicode noncharacters: permanently unassigned, so no ordinary font maps them.
        // macOS's LastResort face maps *every* codepoint (cmap format 13), so skip
        // families that would defeat the premise rather than hardcode a platform list.
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let family = db.faces()
            .filter_map(|f| f.families.first().map(|(name, _)| name.clone()))
            .find(|name| !name.contains("LastResort"));
        if let Some(family) = family {
            assert_eq!(text_to_path(&family, 10.0, "\u{FDD0}\u{FDD1}"), Err(GeomError::NoGlyphs));
        }
    }

    #[test]
    fn list_font_families_is_sorted_deduped_and_nonempty_with_fonts() {
        let families = list_font_families();
        assert!(families.windows(2).all(|w| w[0] <= w[1]), "not sorted: {families:?}");
        let mut deduped = families.clone();
        deduped.dedup();
        assert_eq!(families, deduped, "contains duplicates");
        assert_eq!(families.is_empty(), any_available_family().is_none());
    }
}
