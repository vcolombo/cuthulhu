// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Driver, DriverError, Job, MachineCaps, MachineProfile};

pub struct SilhouetteDriver { profile: MachineProfile }
impl SilhouetteDriver {
    pub fn new() -> Self {
        SilhouetteDriver { profile: MachineProfile {
            id: "cameo5".into(), name: "Silhouette Cameo 5 Alpha".into(),
            width_mm: 330.0, height_mm: 3000.0 } }
    }
}
fn su(mm: f64) -> i64 { (mm * 20.0).round() as i64 }   // 20 units/mm
fn push(s: &str, out: &mut Vec<u8>) { out.extend_from_slice(s.as_bytes()); out.push(0x03); }

impl Driver for SilhouetteDriver {
    fn profile(&self) -> &MachineProfile { &self.profile }
    fn caps(&self) -> MachineCaps {
        MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: false }
    }
    fn session_begin(&self) -> Vec<u8> { vec![0x1b, 0x04] } // ESC EOT init
    fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError> {
        let tool = 1;
        let mut out: Vec<u8> = Vec::new();
        push(&format!("J{tool}"), &mut out);
        if let Some(sp) = pass.settings.speed { push(&format!("!{sp},{tool}"), &mut out); }
        if let Some(fo) = pass.settings.force { push(&format!("FX{fo},{tool}"), &mut out); }
        for _ in 0..pass.settings.repeat_count.max(1) {
            for poly in &pass.polylines {
                if poly.is_empty() { continue; }
                let f = poly[0];                            // note (y,x) order
                push(&format!("M{},{}", su(f.y), su(f.x)), &mut out);
                for p in &poly[1..] { push(&format!("D{},{}", su(p.y), su(p.x)), &mut out); }
            }
        }
        Ok(out)
    }
    fn pass_park(&self) -> Vec<u8> {
        // ponytail: no documented safe-park command yet; head stays put between passes — hardware checklist validates
        Vec::new()
    }
    fn session_end(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push("SO0", &mut out);
        push("FN0", &mut out);
        out
    }
    fn abort_bytes(&self) -> Option<Vec<u8>> { None } // undocumented
}

#[cfg(test)]
mod tests {
    use super::*;
    use driver_core::{Job, Settings};
    use geometry::Point;

    fn square() -> Vec<Point> {
        [(0.0,0.0),(20.0,0.0),(20.0,20.0),(0.0,20.0),(0.0,0.0)]
            .iter().map(|&(x,y)| Point{x,y}).collect()
    }

    #[test]
    fn encodes_square_to_documented_gpgl_stream() {
        let d = SilhouetteDriver::new();
        let job = Job { polylines: vec![square()], settings: Settings::default() };
        let mut bytes = d.session_begin();
        bytes.extend(d.encode_pass(&job).unwrap());
        bytes.extend(d.session_end());
        // ESC EOT · J1 · M0,0 · D0,400 · D400,400 · D400,0 · D0,0 · SO0 · FN0  (20/mm, (y,x))
        let mut want = vec![0x1b, 0x04];
        for cmd in ["J1","M0,0","D0,400","D400,400","D400,0","D0,0","SO0","FN0"] {
            want.extend_from_slice(cmd.as_bytes()); want.push(0x03);
        }
        assert_eq!(bytes, want);
    }

    #[test]
    fn speed_and_force_emitted_only_when_set() {
        let d = SilhouetteDriver::new();
        let job = Job { polylines: vec![square()],
            settings: Settings { speed: Some(10), force: Some(20), repeat_count: 1 } };
        let s = String::from_utf8_lossy(&d.encode_pass(&job).unwrap()).to_string();
        assert!(s.contains("!10,1\u{3}") && s.contains("FX20,1\u{3}"));
    }

    #[test]
    fn session_framing_has_one_prologue_and_one_epilogue_across_two_passes() {
        let d = SilhouetteDriver::new();
        let job = |force| Job { polylines: vec![vec![Point{x:0.0,y:0.0}, Point{x:10.0,y:0.0}]],
                                settings: Settings { speed: Some(5), force: Some(force), repeat_count: 1 } };
        let mut bytes = d.session_begin();
        bytes.extend(d.encode_pass(&job(10)).unwrap());
        bytes.extend(d.pass_park());
        bytes.extend(d.encode_pass(&job(20)).unwrap());
        bytes.extend(d.session_end());
        let count = |needle: &[u8]| bytes.windows(needle.len()).filter(|w| *w == needle).count();
        assert_eq!(count(&[0x1b, 0x04]), 1, "exactly one ESC EOT prologue");
        assert_eq!(count(b"SO0"), 1, "exactly one feed-out epilogue");
        assert_eq!(count(b"FX10,1"), 1);
        assert_eq!(count(b"FX20,1"), 1, "per-pass settings present");
    }

    #[test]
    fn single_pass_session_is_byte_identical_to_sp2_encoding() {
        let d = SilhouetteDriver::new();
        let job = Job { polylines: vec![vec![Point{x:1.0,y:2.0}, Point{x:3.0,y:4.0}]],
                        settings: Settings { speed: Some(8), force: Some(12), repeat_count: 2 } };
        let mut session = d.session_begin();
        session.extend(d.encode_pass(&job).unwrap());
        session.extend(d.session_end());
        // must equal the pre-plan golden bytes for this job (copied from the SP2 encoder,
        // which looped `repeat_count` M/D passes inside a single J/speed/force/SO0/FN0 frame)
        fn sp2_golden_for_job() -> Vec<u8> {
            let mut want = vec![0x1b, 0x04];
            for cmd in ["J1","!8,1","FX12,1","M40,20","D80,60","M40,20","D80,60","SO0","FN0"] {
                want.extend_from_slice(cmd.as_bytes()); want.push(0x03);
            }
            want
        }
        assert_eq!(session, sp2_golden_for_job());
    }
}
