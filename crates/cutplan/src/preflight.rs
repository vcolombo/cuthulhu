// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{MachineProfile, MachineCaps, Settings};
use document::NodeId;
use crate::passes::ColorPass;

pub struct ConfiguredPass<'a> {
    pub pass: &'a ColorPass,
    pub settings: Settings,
    pub enabled: bool,
}

#[derive(Debug, PartialEq)]
pub enum PreflightError {
    NothingToCut,
    NonFiniteGeometry(NodeId),
    DegeneratePolyline(NodeId),
    OutOfBounds { node: NodeId, bounds: (f64, f64, f64, f64) },
    SettingsOutOfRange(&'static str),
    MachineMismatch { document: String, device: String },
    OutputTooLarge(usize),
}

/// Validate a cut job before encoding. Rules checked in order (first violation wins):
/// 1. All enabled passes empty → NothingToCut
/// 2. Any NaN/inf coordinate → NonFiniteGeometry
/// 3. Polyline < 2 points → DegeneratePolyline
/// 4. Geometry outside 0..width_mm × 0..height_mm → OutOfBounds (unless allow_out_of_bounds)
/// 5. repeat_count outside 1..=10 or (when caps support them) speed outside 1..=30 / force outside 1..=33
///    (Cameo bounds from docs/protocol/silhouette-cameo5.md §Command reference) → SettingsOutOfRange
/// 6. doc_machine_id set and ≠ profile.id → MachineMismatch
/// 7. Estimated encoded size (16 bytes/point × repeat_count) > 64 MB → OutputTooLarge
pub fn preflight(
    passes: &[ConfiguredPass],
    profile: &MachineProfile,
    caps: &MachineCaps,
    doc_machine_id: Option<&str>,
    allow_out_of_bounds: bool,
) -> Result<(), PreflightError> {
    // Rule 1: All enabled passes empty → NothingToCut
    let has_geometry = passes
        .iter()
        .filter(|p| p.enabled)
        .any(|p| !p.pass.shapes.is_empty());
    if !has_geometry {
        return Err(PreflightError::NothingToCut);
    }

    // Rules 2–3: Iterate through all geometry for NaN/inf and degenerate polylines.
    for pass in passes.iter().filter(|p| p.enabled) {
        for shape in &pass.pass.shapes {
            for polyline in &shape.polylines {
                // Rule 3: Polyline < 2 points → DegeneratePolyline
                if polyline.len() < 2 {
                    return Err(PreflightError::DegeneratePolyline(shape.node_id));
                }
                // Rule 2: Any NaN/inf coordinate → NonFiniteGeometry
                for point in polyline {
                    if !point.x.is_finite() || !point.y.is_finite() {
                        return Err(PreflightError::NonFiniteGeometry(shape.node_id));
                    }
                }
            }
        }
    }

    // Rule 4: Geometry outside 0..width_mm × 0..height_mm → OutOfBounds (unless allow_out_of_bounds)
    if !allow_out_of_bounds {
        for pass in passes.iter().filter(|p| p.enabled) {
            for shape in &pass.pass.shapes {
                for polyline in &shape.polylines {
                    for point in polyline {
                        if point.x < 0.0 || point.x > profile.width_mm ||
                           point.y < 0.0 || point.y > profile.height_mm {
                            return Err(PreflightError::OutOfBounds {
                                node: shape.node_id,
                                bounds: (0.0, 0.0, profile.width_mm, profile.height_mm),
                            });
                        }
                    }
                }
            }
        }
    }

    // Rule 5: repeat_count outside 1..=10 or speed/force out of bounds → SettingsOutOfRange
    for pass in passes.iter().filter(|p| p.enabled) {
        let settings = &pass.settings;

        // repeat_count must be 1..=10
        if settings.repeat_count < 1 || settings.repeat_count > 10 {
            return Err(PreflightError::SettingsOutOfRange("repeat_count must be 1..=10"));
        }

        // Speed bounds: 1..=30 when supported
        if let Some(speed) = settings.speed {
            if !caps.supports_speed {
                return Err(PreflightError::SettingsOutOfRange("speed not supported by device"));
            }
            if speed < 1 || speed > 30 {
                return Err(PreflightError::SettingsOutOfRange("speed must be 1..=30"));
            }
        }

        // Force bounds: 1..=33 when supported
        if let Some(force) = settings.force {
            if !caps.supports_force {
                return Err(PreflightError::SettingsOutOfRange("force not supported by device"));
            }
            if force < 1 || force > 33 {
                return Err(PreflightError::SettingsOutOfRange("force must be 1..=33"));
            }
        }
    }

    // Rule 6: doc_machine_id set and ≠ profile.id → MachineMismatch
    if let Some(doc_id) = doc_machine_id {
        if doc_id != profile.id {
            return Err(PreflightError::MachineMismatch {
                document: doc_id.to_string(),
                device: profile.id.clone(),
            });
        }
    }

    // Rule 7: Estimated encoded size > 64 MB → OutputTooLarge
    // Estimate: 16 bytes/point × repeat_count
    let mut total_points = 0usize;
    for pass in passes.iter().filter(|p| p.enabled) {
        for shape in &pass.pass.shapes {
            for polyline in &shape.polylines {
                total_points = total_points.saturating_add(polyline.len());
            }
        }
    }
    let max_repeat = passes
        .iter()
        .filter(|p| p.enabled)
        .map(|p| p.settings.repeat_count as usize)
        .max()
        .unwrap_or(1);
    let estimated_size = total_points.saturating_mul(16).saturating_mul(max_repeat);
    if estimated_size > 64 * 1024 * 1024 {
        return Err(PreflightError::OutputTooLarge(estimated_size));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passes::PlannedShape;
    use geometry::Point;

    fn pt(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn make_pass(color: Option<u32>, shapes: Vec<PlannedShape>) -> ColorPass {
        ColorPass { color, shapes }
    }

    fn make_shape(node_id: u64, polylines: Vec<Vec<Point>>) -> PlannedShape {
        PlannedShape {
            node_id: NodeId(node_id),
            polylines,
        }
    }

    fn make_configured_pass<'a>(pass: &'a ColorPass, settings: Settings, enabled: bool) -> ConfiguredPass<'a> {
        ConfiguredPass { pass, settings, enabled }
    }

    fn profile_100x100() -> MachineProfile {
        MachineProfile {
            id: "test-machine".to_string(),
            name: "Test Machine".to_string(),
            width_mm: 100.0,
            height_mm: 100.0,
        }
    }

    fn caps_no_speed_force() -> MachineCaps {
        MachineCaps {
            supports_speed: false,
            supports_force: false,
            needs_operator_pass_confirm: false,
        }
    }

    fn caps_with_speed_force() -> MachineCaps {
        MachineCaps {
            supports_speed: true,
            supports_force: true,
            needs_operator_pass_confirm: false,
        }
    }

    #[test]
    fn nothing_to_cut_when_all_enabled_passes_empty() {
        let pass = make_pass(Some(0xFF0000FF), vec![]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::NothingToCut));
    }

    #[test]
    fn nothing_to_cut_ignores_disabled_passes_with_content() {
        let shape = make_shape(1, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), false)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::NothingToCut));
    }

    #[test]
    fn non_finite_geometry_detects_nan() {
        let shape = make_shape(1, vec![vec![pt(10.0, 10.0), pt(f64::NAN, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::NonFiniteGeometry(NodeId(1))));
    }

    #[test]
    fn non_finite_geometry_detects_inf() {
        let shape = make_shape(2, vec![vec![pt(10.0, 10.0), pt(f64::INFINITY, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::NonFiniteGeometry(NodeId(2))));
    }

    #[test]
    fn degenerate_polyline_single_point() {
        let shape = make_shape(3, vec![vec![pt(10.0, 10.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::DegeneratePolyline(NodeId(3))));
    }

    #[test]
    fn degenerate_polyline_empty() {
        let shape = make_shape(4, vec![vec![]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::DegeneratePolyline(NodeId(4))));
    }

    #[test]
    fn out_of_bounds_x_negative() {
        let shape = make_shape(5, vec![vec![pt(-1.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(
            result,
            Err(PreflightError::OutOfBounds {
                node: NodeId(5),
                bounds: (0.0, 0.0, 100.0, 100.0),
            })
        );
    }

    #[test]
    fn out_of_bounds_x_exceeds_width() {
        let shape = make_shape(6, vec![vec![pt(10.0, 10.0), pt(110.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(
            result,
            Err(PreflightError::OutOfBounds {
                node: NodeId(6),
                bounds: (0.0, 0.0, 100.0, 100.0),
            })
        );
    }

    #[test]
    fn out_of_bounds_y_negative() {
        let shape = make_shape(7, vec![vec![pt(10.0, -5.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(
            result,
            Err(PreflightError::OutOfBounds {
                node: NodeId(7),
                bounds: (0.0, 0.0, 100.0, 100.0),
            })
        );
    }

    #[test]
    fn out_of_bounds_y_exceeds_height() {
        let shape = make_shape(8, vec![vec![pt(10.0, 10.0), pt(20.0, 110.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(
            result,
            Err(PreflightError::OutOfBounds {
                node: NodeId(8),
                bounds: (0.0, 0.0, 100.0, 100.0),
            })
        );
    }

    #[test]
    fn allow_out_of_bounds_flag_permits_geometry_outside_bounds() {
        let shape = make_shape(9, vec![vec![pt(-10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, true);
        assert!(result.is_ok());
    }

    #[test]
    fn repeat_count_below_1_rejected() {
        let shape = make_shape(10, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let settings = Settings { speed: None, force: None, repeat_count: 0 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("repeat_count must be 1..=10")));
    }

    #[test]
    fn repeat_count_above_10_rejected() {
        let shape = make_shape(11, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let settings = Settings { speed: None, force: None, repeat_count: 11 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("repeat_count must be 1..=10")));
    }

    #[test]
    fn speed_unsupported_by_device_rejected() {
        let shape = make_shape(12, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let settings = Settings { speed: Some(15), force: None, repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("speed not supported by device")));
    }

    #[test]
    fn speed_below_1_rejected() {
        let shape = make_shape(13, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let settings = Settings { speed: Some(0), force: None, repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_with_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("speed must be 1..=30")));
    }

    #[test]
    fn speed_above_30_rejected() {
        let shape = make_shape(14, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let settings = Settings { speed: Some(31), force: None, repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_with_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("speed must be 1..=30")));
    }

    #[test]
    fn force_unsupported_by_device_rejected() {
        let shape = make_shape(15, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let settings = Settings { speed: None, force: Some(15), repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("force not supported by device")));
    }

    #[test]
    fn force_below_1_rejected() {
        let shape = make_shape(16, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let settings = Settings { speed: None, force: Some(0), repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_with_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("force must be 1..=33")));
    }

    #[test]
    fn force_above_33_rejected() {
        let shape = make_shape(17, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let settings = Settings { speed: None, force: Some(34), repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_with_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("force must be 1..=33")));
    }

    #[test]
    fn machine_mismatch_doc_id_differs() {
        let shape = make_shape(18, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(
            &configured,
            &profile_100x100(),
            &caps_no_speed_force(),
            Some("wrong-machine"),
            false,
        );
        assert_eq!(
            result,
            Err(PreflightError::MachineMismatch {
                document: "wrong-machine".to_string(),
                device: "test-machine".to_string(),
            })
        );
    }

    #[test]
    fn machine_match_doc_id_same() {
        let shape = make_shape(19, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(
            &configured,
            &profile_100x100(),
            &caps_no_speed_force(),
            Some("test-machine"),
            false,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn output_too_large_exceeds_64mb() {
        // Estimate: 16 bytes/point × repeat_count
        // 64 MB = 67108864 bytes
        // With 500000 points × repeat_count=10: 500000 × 16 × 10 = 80,000,000 bytes > 64 MB
        let mut points = vec![];
        for i in 0..500000 {
            let x = (i % 100) as f64;
            let y = ((i / 100) % 100) as f64;
            points.push(pt(x, y));
        }
        let shape = make_shape(20, vec![points]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let settings = Settings { speed: None, force: None, repeat_count: 10 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert!(matches!(result, Err(PreflightError::OutputTooLarge(_))));
    }

    #[test]
    fn happy_path_valid_cut() {
        let shape = make_shape(21, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0), pt(30.0, 10.0)]]);
        let pass = make_pass(Some(0xFF0000FF), vec![shape]);
        let settings = Settings { speed: Some(15), force: Some(20), repeat_count: 3 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_with_speed_force(), None, false);
        assert_eq!(result, Ok(()));
    }
}
