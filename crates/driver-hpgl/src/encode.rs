// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Driver, DriverError, Job, MachineProfile};

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
    fn encode(&self, job: &Job) -> Result<Vec<u8>, DriverError> {
        // Ignores speed/force in V1 (GCC panel-set; see gcc-hpgl.md).
        let mut s = String::from("IN;");
        for _ in 0..job.settings.passes.max(1) {
            for poly in &job.polylines {
                if poly.is_empty() { continue; }
                let f = poly[0];                                   // (x,y) order
                s.push_str(&format!("PU{},{};", u(f.x), u(f.y)));
                for p in &poly[1..] { s.push_str(&format!("PD{},{};", u(p.x), u(p.y))); }
            }
        }
        s.push_str("PU;");
        Ok(s.into_bytes())
    }
    fn profile(&self) -> &MachineProfile { &self.profile }
}

#[cfg(test)]
mod tests {
    use super::*;
    use driver_core::{Driver, Job, Settings};
    use geometry::Point;
    #[test]
    fn encodes_square_to_documented_hpgl_stream() {
        let d = HpglDriver::new();
        let poly: Vec<Point> = [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0), (0.0, 0.0)]
            .iter()
            .map(|&(x, y)| Point { x, y })
            .collect();
        let job = Job { polylines: vec![poly], settings: Settings::default() };
        let s = String::from_utf8(d.encode(&job).unwrap()).unwrap();
        // 20mm → 800 units (1016/in), (x,y) order
        assert_eq!(s, "IN;PU0,0;PD800,0;PD800,800;PD0,800;PD0,0;PU;");
    }
}
