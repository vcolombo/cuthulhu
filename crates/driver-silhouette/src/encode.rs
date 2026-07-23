// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Driver, Job, MachineProfile, DriverError};

pub struct SilhouetteDriver { profile: MachineProfile }
impl SilhouetteDriver {
    pub fn new() -> Self {
        SilhouetteDriver { profile: MachineProfile {
            id: "cameo5".into(), name: "Silhouette Cameo 5 Alpha".into(),
            width_mm: 330.0, height_mm: 3000.0 } }
    }
}
fn su(mm: f64) -> i64 { (mm * 20.0).round() as i64 }   // 20 units/mm

impl Driver for SilhouetteDriver {
    fn encode(&self, job: &Job) -> Result<Vec<u8>, DriverError> {
        let tool = 1;
        let mut out: Vec<u8> = vec![0x1b, 0x04];            // ESC EOT init
        let push = |s: String, out: &mut Vec<u8>| { out.extend_from_slice(s.as_bytes()); out.push(0x03); };
        push(format!("J{tool}"), &mut out);
        if let Some(sp) = job.settings.speed { push(format!("!{sp},{tool}"), &mut out); }
        if let Some(fo) = job.settings.force { push(format!("FX{fo},{tool}"), &mut out); }
        for _ in 0..job.settings.passes.max(1) {
            for poly in &job.polylines {
                if poly.is_empty() { continue; }
                let f = poly[0];                            // note (y,x) order
                push(format!("M{},{}", su(f.y), su(f.x)), &mut out);
                for p in &poly[1..] { push(format!("D{},{}", su(p.y), su(p.x)), &mut out); }
            }
        }
        push("SO0".into(), &mut out);
        push("FN0".into(), &mut out);
        Ok(out)
    }
    fn profile(&self) -> &MachineProfile { &self.profile }
}

#[cfg(test)]
mod tests {
    use super::*;
    use driver_core::{Job, Settings, Driver};
    use geometry::Point;

    fn square() -> Vec<Point> {
        [(0.0,0.0),(20.0,0.0),(20.0,20.0),(0.0,20.0),(0.0,0.0)]
            .iter().map(|&(x,y)| Point{x,y}).collect()
    }

    #[test]
    fn encodes_square_to_documented_gpgl_stream() {
        let d = SilhouetteDriver::new();
        let job = Job { polylines: vec![square()], settings: Settings::default() };
        let bytes = d.encode(&job).unwrap();
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
            settings: Settings { speed: Some(10), force: Some(20), passes: 1 } };
        let s = String::from_utf8_lossy(&d.encode(&job).unwrap()).to_string();
        assert!(s.contains("!10,1\u{3}") && s.contains("FX20,1\u{3}"));
    }
}
