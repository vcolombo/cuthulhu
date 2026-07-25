// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Driver, DriverError, Job, MachineCaps, MachineProfile};

pub struct HpglDriver { profile: MachineProfile }
impl HpglDriver {
    pub fn new() -> Self {
        HpglDriver { profile: MachineProfile {
            id: "puma".into(), name: "GCC Puma IV".into(), width_mm: 600.0, height_mm: 5000.0 } }
    }
}
impl Default for HpglDriver {
    fn default() -> Self { Self::new() }
}
fn u(mm: f64) -> i64 { (mm / 25.4 * 1016.0).round() as i64 }   // 1016 units/inch

impl Driver for HpglDriver {
    fn profile(&self) -> &MachineProfile { &self.profile }
    fn caps(&self) -> MachineCaps {
        MachineCaps { supports_speed: false, supports_force: false, needs_operator_pass_confirm: true }
    }
    fn session_begin(&self) -> Vec<u8> { b"IN;".to_vec() }
    fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError> {
        // Ignores speed/force in V1 (GCC panel-set; see gcc-hpgl.md).
        let mut s = String::new();
        for _ in 0..pass.settings.repeat_count.max(1) {
            for poly in &pass.polylines {
                if poly.is_empty() { continue; }
                let f = poly[0];                                   // (x,y) order
                s.push_str(&format!("PU{},{};", u(f.x), u(f.y)));
                for p in &poly[1..] { s.push_str(&format!("PD{},{};", u(p.x), u(p.y))); }
            }
        }
        Ok(s.into_bytes())
    }
    fn pass_park(&self) -> Vec<u8> { b"PU;".to_vec() }
    fn session_end(&self) -> Vec<u8> { b"PU;".to_vec() }
    fn abort_bytes(&self) -> Option<Vec<u8>> { Some(b"PU;".to_vec()) } // queued best-effort pen-up
}

#[cfg(test)]
mod tests {
    use super::*;
    use driver_core::{Job, Settings};
    use geometry::Point;

    #[test]
    fn encodes_square_to_documented_hpgl_stream() {
        let d = HpglDriver::new();
        let poly: Vec<Point> = [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0), (0.0, 0.0)]
            .iter()
            .map(|&(x, y)| Point { x, y })
            .collect();
        let job = Job { polylines: vec![poly], settings: Settings::default() };
        let mut bytes = d.session_begin();
        bytes.extend(d.encode_pass(&job).unwrap());
        bytes.extend(d.session_end());
        let s = String::from_utf8(bytes).unwrap();
        // 20mm → 800 units (1016/in), (x,y) order
        assert_eq!(s, "IN;PU0,0;PD800,0;PD800,800;PD0,800;PD0,0;PU;");
    }

    #[test]
    fn session_framing_has_one_prologue_and_one_epilogue_across_two_passes() {
        let d = HpglDriver::new();
        let job = |x: f64| Job { polylines: vec![vec![Point{x:0.0,y:0.0}, Point{x, y:0.0}]],
                                settings: Settings { speed: None, force: None, repeat_count: 1 } };
        let mut bytes = d.session_begin();
        bytes.extend(d.encode_pass(&job(10.0)).unwrap());
        bytes.extend(d.pass_park());
        bytes.extend(d.encode_pass(&job(20.0)).unwrap());
        bytes.extend(d.session_end());
        let count = |needle: &[u8]| bytes.windows(needle.len()).filter(|w| *w == needle).count();
        assert_eq!(count(b"IN;"), 1, "exactly one prologue");
        assert_eq!(count(b"PD400,0;"), 1, "per-pass geometry present");
        assert_eq!(count(b"PD800,0;"), 1, "per-pass geometry present");
        assert!(bytes.ends_with(b"PU;"), "session ends with epilogue");
    }

    #[test]
    fn single_pass_session_is_byte_identical_to_sp2_encoding() {
        let d = HpglDriver::new();
        let job = Job { polylines: vec![vec![Point{x:1.0,y:2.0}, Point{x:3.0,y:4.0}]],
                        settings: Settings { speed: None, force: None, repeat_count: 2 } };
        let mut session = d.session_begin();
        session.extend(d.encode_pass(&job).unwrap());
        session.extend(d.session_end());
        // must equal the pre-plan golden bytes for this job (copied from the SP2 encoder,
        // which looped `repeat_count` PU/PD passes inside a single IN;/PU; frame)
        let want = b"IN;PU40,80;PD120,160;PU40,80;PD120,160;PU;".to_vec();
        assert_eq!(session, want);
    }

    #[test]
    fn caps_and_abort_bytes_match_the_documented_contract() {
        let d = HpglDriver::new();
        assert_eq!(d.caps(), MachineCaps { supports_speed: false, supports_force: false, needs_operator_pass_confirm: true });
        assert_eq!(d.abort_bytes(), Some(b"PU;".to_vec()));
    }
}
