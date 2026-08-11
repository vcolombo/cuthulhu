// SPDX-License-Identifier: GPL-3.0-or-later
//! What a Cut Host checks before it lets a dispatch reach a machine.
//!
//! Not a second Preflight. `cutplan::preflight` refuses a *document* and names the
//! shape that offended; by the time a dispatch crosses the wire the document is
//! gone and only Passes remain, so this refuses a Pass and names its index. Every
//! threshold the two share comes from `cutplan::preflight` — only the locator
//! differs.

use cutplan::preflight::{point_out_of_bounds, settings_out_of_range, BYTES_PER_POINT, MAX_ENCODED_BYTES};
use driver_core::manager::CutPass;
use driver_core::{MachineCaps, MachineProfile};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum PassFault {
    NonFinite(usize),
    Degenerate(usize),
    OutOfBounds { pass: usize, bounds: (f64, f64) },
    // `settings_out_of_range` hands back `&'static str`, but a Cut Host's refusal
    // now crosses the wire (`Refusal::Preflight`), and `Deserialize` cannot produce
    // a `&'static str` from bytes read off a socket — only an owned `String` can
    // survive that hop.
    Settings { pass: usize, message: String },
    TooLarge(usize),
    /// An empty dispatch: no Pass to number, so no index.
    NoPasses,
}

/// Passes are numbered from 1 here, unlike the indices everywhere else in this
/// crate: this string is read by whoever is standing at the cutter.
impl std::fmt::Display for PassFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PassFault::NonFinite(i) =>
                write!(f, "pass {} has a coordinate that is not a finite number", i + 1),
            PassFault::Degenerate(i) =>
                write!(f, "pass {} has a path with fewer than two points", i + 1),
            PassFault::OutOfBounds { pass, bounds } =>
                write!(f, "pass {} lies outside the {} x {} mm cutting area", pass + 1, bounds.0, bounds.1),
            PassFault::Settings { pass, message } => write!(f, "pass {}: {message}", pass + 1),
            PassFault::TooLarge(bytes) =>
                write!(f, "the encoded cut is about {} MB, over the {} MB limit",
                       bytes.div_ceil(1024 * 1024), MAX_ENCODED_BYTES / (1024 * 1024)),
            PassFault::NoPasses => write!(f, "this cut has no passes"),
        }
    }
}
impl std::error::Error for PassFault {}

/// Rules in the same order `cutplan::preflight` uses, so a dispatch that the
/// desktop already accepted fails here for the same reason it would have there —
/// only against the machine actually attached.
pub fn check_passes(
    passes: &[CutPass],
    profile: &MachineProfile,
    caps: &MachineCaps,
) -> Result<(), PassFault> {
    for (i, pass) in passes.iter().enumerate() {
        for polyline in &pass.job.polylines {
            for point in polyline {
                if !point.x.is_finite() || !point.y.is_finite() {
                    return Err(PassFault::NonFinite(i));
                }
            }
        }
    }
    for (i, pass) in passes.iter().enumerate() {
        for polyline in &pass.job.polylines {
            if polyline.len() < 2 {
                return Err(PassFault::Degenerate(i));
            }
        }
    }
    for (i, pass) in passes.iter().enumerate() {
        for polyline in &pass.job.polylines {
            for point in polyline {
                if point_out_of_bounds(point, profile) {
                    return Err(PassFault::OutOfBounds {
                        pass: i,
                        bounds: (profile.width_mm, profile.height_mm),
                    });
                }
            }
        }
    }
    for (i, pass) in passes.iter().enumerate() {
        if let Some(message) = settings_out_of_range(&pass.job.settings, caps) {
            return Err(PassFault::Settings { pass: i, message: message.to_string() });
        }
    }

    let mut estimated = 0usize;
    for pass in passes {
        let points: usize = pass.job.polylines.iter()
            .fold(0usize, |acc, p| acc.saturating_add(p.len()));
        estimated = estimated.saturating_add(
            points.saturating_mul(BYTES_PER_POINT)
                  .saturating_mul(pass.job.settings.repeat_count as usize),
        );
    }
    if estimated > MAX_ENCODED_BYTES {
        return Err(PassFault::TooLarge(estimated));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use driver_core::manager::CutPass;
    use driver_core::{Job, MachineCaps, MachineProfile, Settings};
    use geometry::Point;

    fn profile() -> MachineProfile {
        MachineProfile { id: "cameo5".into(), name: "Cameo".into(), width_mm: 300.0, height_mm: 200.0 }
    }
    fn caps() -> MachineCaps {
        MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: false }
    }
    fn pass_with(polylines: Vec<Vec<Point>>, settings: Settings) -> CutPass {
        CutPass { job: Job { polylines, settings } }
    }
    fn square() -> Vec<Point> {
        vec![
            Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 },
            Point { x: 10.0, y: 10.0 }, Point { x: 0.0, y: 0.0 },
        ]
    }

    #[test]
    fn a_sound_dispatch_passes() {
        let p = pass_with(vec![square()], Settings::default());
        assert!(check_passes(&[p], &profile(), &caps()).is_ok());
    }

    #[test]
    fn a_non_finite_coordinate_names_its_pass() {
        let bad = pass_with(vec![vec![Point { x: 0.0, y: 0.0 }, Point { x: f64::NAN, y: 1.0 }]], Settings::default());
        let good = pass_with(vec![square()], Settings::default());
        assert!(matches!(check_passes(&[good, bad], &profile(), &caps()), Err(PassFault::NonFinite(1))));
    }

    #[test]
    fn a_one_point_polyline_is_degenerate() {
        let bad = pass_with(vec![vec![Point { x: 1.0, y: 1.0 }]], Settings::default());
        assert!(matches!(check_passes(&[bad], &profile(), &caps()), Err(PassFault::Degenerate(0))));
    }

    /// The rule the host exists to enforce: the client planned against a bed it
    /// believed was there, and this is a smaller one.
    #[test]
    fn geometry_off_the_attached_machines_bed_is_refused() {
        let bad = pass_with(
            vec![vec![Point { x: 0.0, y: 0.0 }, Point { x: 400.0, y: 0.0 }]],
            Settings::default(),
        );
        match check_passes(&[bad], &profile(), &caps()) {
            Err(PassFault::OutOfBounds { pass, bounds }) => {
                assert_eq!(pass, 0);
                assert_eq!(bounds, (300.0, 200.0));
            }
            other => panic!("expected OutOfBounds, got {other:?}"),
        }
    }

    #[test]
    fn settings_outside_the_attached_machines_range_are_refused() {
        let bad = pass_with(vec![square()], Settings { speed: Some(99), force: None, repeat_count: 1 });
        assert!(matches!(check_passes(&[bad], &profile(), &caps()), Err(PassFault::Settings { pass: 0, .. })));

        let repeats = pass_with(vec![square()], Settings { speed: None, force: None, repeat_count: 99 });
        assert!(matches!(check_passes(&[repeats], &profile(), &caps()), Err(PassFault::Settings { pass: 0, .. })));
    }

    /// A machine that does not support a setting ignores it, so a value the Driver
    /// will never encode must not refuse the cut.
    #[test]
    fn a_setting_the_machine_ignores_is_not_out_of_range() {
        let no_speed = MachineCaps { supports_speed: false, supports_force: false, needs_operator_pass_confirm: true };
        let p = pass_with(vec![square()], Settings { speed: Some(99), force: Some(99), repeat_count: 1 });
        assert!(check_passes(&[p], &profile(), &no_speed).is_ok());
    }

    #[test]
    fn an_oversized_cut_is_refused() {
        // 16 bytes/point × repeat_count, over 64 MB.
        let many = vec![Point { x: 1.0, y: 1.0 }; 500_000];
        let p = pass_with(vec![many], Settings { speed: None, force: None, repeat_count: 10 });
        assert!(matches!(check_passes(&[p], &profile(), &caps()), Err(PassFault::TooLarge(_))));
    }

    /// The refusal crosses the wire as this sentence, so it has to read as one.
    #[test]
    fn a_fault_reads_as_a_sentence() {
        let f = PassFault::OutOfBounds { pass: 2, bounds: (300.0, 300.0) };
        assert_eq!(f.to_string(), "pass 3 lies outside the 300 x 300 mm cutting area");
        assert_eq!(PassFault::NonFinite(0).to_string(), "pass 1 has a coordinate that is not a finite number");
    }
}
