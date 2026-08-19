// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{MachineProfile, MachineCaps, Settings};
use document::NodeId;
use geometry::Point;
use serde::Serialize;
use crate::passes::DocumentPass;

pub struct ConfiguredPass<'a> {
    pub pass: &'a DocumentPass,
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

/// What each rule refused, in the words an operator reads. It lives next to the rules
/// rather than in each caller because the desktop and the CLI used to write it twice,
/// and the CLI's copy fell through to `Debug` for four of these.
impl std::fmt::Display for PreflightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Not "no enabled pass": a colour nobody selected is simply not cut, so
            // there is no flag for an operator to go looking for (CONTEXT.md).
            PreflightError::NothingToCut =>
                write!(f, "no pass selected for this cut has any geometry"),
            PreflightError::NonFiniteGeometry(node) =>
                write!(f, "shape #{} has a coordinate that is not a finite number", node.0),
            PreflightError::DegeneratePolyline(node) =>
                write!(f, "shape #{} has a path with fewer than two points", node.0),
            // bounds is (0, 0, width_mm, height_mm) — the machine's area, not the shape's.
            PreflightError::OutOfBounds { node, bounds } =>
                write!(f, "shape #{} lies outside the {} x {} mm cutting area", node.0, bounds.2, bounds.3),
            // Already a whole clause naming the setting and its range, so a prefix would read twice.
            PreflightError::SettingsOutOfRange(message) => write!(f, "{message}"),
            PreflightError::MachineMismatch { document, device } =>
                write!(f, "this document is set up for `{document}`, but the connected machine is `{device}`"),
            // Megabytes, not the byte count the variant carries: the estimate weights 16
            // bytes per point by repeat_count, so printing it exactly claims a precision
            // it does not have. Divisor matches the rule's own `64 * 1024 * 1024`. Rounds
            // up so a value just over the limit cannot print as if it were at or under it.
            PreflightError::OutputTooLarge(bytes) =>
                write!(f, "the encoded cut is about {} MB, over the {} MB limit",
                       bytes.div_ceil(1024 * 1024), MAX_ENCODED_BYTES / (1024 * 1024)),
        }
    }
}
impl std::error::Error for PreflightError {}

impl PreflightError {
    /// Stable identifier for a caller that must branch on the *kind* of refusal rather
    /// than show its text — the desktop sends it as `IpcError::code`, and `CutDialog.tsx`
    /// keys off `stale_plan` instead of matching a sentence across a language boundary.
    pub fn code(&self) -> &'static str {
        match self {
            PreflightError::NothingToCut => "nothing_to_cut",
            PreflightError::NonFiniteGeometry(_) => "non_finite_geometry",
            PreflightError::DegeneratePolyline(_) => "degenerate_polyline",
            PreflightError::OutOfBounds { .. } => "out_of_bounds",
            PreflightError::SettingsOutOfRange(_) => "settings_out_of_range",
            PreflightError::MachineMismatch { .. } => "machine_mismatch",
            PreflightError::OutputTooLarge(_) => "output_too_large",
        }
    }
}

/// The estimate `preflight` and a Cut Host both weight geometry by. Not a
/// measurement — 16 bytes a point, times that Pass's own repeat_count.
pub const BYTES_PER_POINT: usize = 16;
/// The ceiling that estimate is refused against.
pub const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;

/// Whether a point falls off the machine's bed. Shared so that a Cut Host,
/// checking a dispatch against the machine actually attached, refuses at exactly
/// the same edge the desktop does.
pub fn point_out_of_bounds(p: &Point, profile: &MachineProfile) -> bool {
    p.x < 0.0 || p.x > profile.width_mm || p.y < 0.0 || p.y > profile.height_mm
}

/// One bound pair, in the units the operator types.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SettingRange {
    pub min: u32,
    pub max: u32,
}

impl SettingRange {
    const fn admits(&self, value: u32) -> bool {
        value >= self.min && value <= self.max
    }
}

/// Every bound a Settings value is refused against.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsRanges {
    pub speed: SettingRange,
    pub force: SettingRange,
    pub repeat_count: SettingRange,
}

/// Public because the preset editor asks for these over IPC rather than restating them: a
/// second copy of a bound in TypeScript offers the operator a speed this crate then refuses.
/// Same arrangement as `trace::control_specs()` for the tracer's ranges (see
/// `ipc::trace_controls`).
///
/// Cameo bounds from docs/protocol/silhouette-cameo5.md §Settings ranges.
pub const SETTINGS_RANGES: SettingsRanges = SettingsRanges {
    speed: SettingRange { min: 1, max: 30 },
    force: SettingRange { min: 1, max: 33 },
    repeat_count: SettingRange { min: 1, max: 10 },
};

/// The one comparison against `SETTINGS_RANGES`, so a cut's Settings and a stored preset's
/// cannot disagree about where an edge is. A caller passes `None` for a value that is not to
/// be judged at all.
fn out_of_range(
    speed: Option<u32>,
    force: Option<u32>,
    repeat_count: u32,
) -> Option<&'static str> {
    if !SETTINGS_RANGES.repeat_count.admits(repeat_count) {
        return Some("repeat_count must be 1..=10");
    }
    if speed.is_some_and(|v| !SETTINGS_RANGES.speed.admits(v)) {
        return Some("speed must be 1..=30");
    }
    if force.is_some_and(|v| !SETTINGS_RANGES.force.admits(v)) {
        return Some("force must be 1..=33");
    }
    None
}

/// The setting that is out of range and the sentence saying so, or `None`.
/// A setting the machine does not support is ignored rather than refused —
/// the Drivers skip those values, so refusing them would reject a cut over a
/// number that will never reach the wire.
pub fn settings_out_of_range(s: &Settings, caps: &MachineCaps) -> Option<&'static str> {
    out_of_range(
        s.speed.filter(|_| caps.supports_speed),
        s.force.filter(|_| caps.supports_force),
        s.repeat_count,
    )
}

/// The same bounds with no machine to ask. A stored preset is checked whole: the fields a
/// machine ignores are still written to its file, and the operator who typed them there gets
/// them back the next time it is edited.
pub fn preset_settings_out_of_range(s: &crate::presets::PresetSettings) -> Option<&'static str> {
    out_of_range(s.speed, s.force, s.repeat_count)
}

/// Validate a cut job before encoding. Rules checked in order (first violation wins):
/// 1. All enabled passes empty → NothingToCut
/// 2. Any NaN/inf coordinate → NonFiniteGeometry
/// 3. Polyline < 2 points → DegeneratePolyline
/// 4. Geometry outside 0..width_mm × 0..height_mm → OutOfBounds (unless allow_out_of_bounds)
/// 5. repeat_count outside 1..=10 or speed outside 1..=30 / force outside 1..=33 when set
///    (Cameo bounds from docs/protocol/silhouette-cameo5.md §Settings ranges) → SettingsOutOfRange
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

    // Rule 2: Scan all enabled geometry for NaN/inf coordinates first (rule 2 before rule 3)
    for pass in passes.iter().filter(|p| p.enabled) {
        for shape in &pass.pass.shapes {
            for polyline in &shape.polylines {
                for point in polyline {
                    if !point.x.is_finite() || !point.y.is_finite() {
                        return Err(PreflightError::NonFiniteGeometry(shape.node_id));
                    }
                }
            }
        }
    }

    // Rule 3: Polyline < 2 points → DegeneratePolyline (checked after NaN/inf)
    for pass in passes.iter().filter(|p| p.enabled) {
        for shape in &pass.pass.shapes {
            for polyline in &shape.polylines {
                if polyline.len() < 2 {
                    return Err(PreflightError::DegeneratePolyline(shape.node_id));
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
                        if point_out_of_bounds(point, profile) {
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
        if let Some(message) = settings_out_of_range(&pass.settings, caps) {
            return Err(PreflightError::SettingsOutOfRange(message));
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
    // Estimate: 16 bytes/point × that pass's own repeat_count, summed per pass —
    // exact weighting, unlike an all-passes max which over-rejects mixed-repeat jobs.
    let mut estimated_size = 0usize;
    for pass in passes.iter().filter(|p| p.enabled) {
        let mut pass_points = 0usize;
        for shape in &pass.pass.shapes {
            for polyline in &shape.polylines {
                pass_points = pass_points.saturating_add(polyline.len());
            }
        }
        let pass_bytes = pass_points
            .saturating_mul(BYTES_PER_POINT)
            .saturating_mul(pass.settings.repeat_count as usize);
        estimated_size = estimated_size.saturating_add(pass_bytes);
    }
    if estimated_size > MAX_ENCODED_BYTES {
        return Err(PreflightError::OutputTooLarge(estimated_size));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass_key::PassKey;
    use crate::passes::PlannedShape;
    use geometry::Point;

    fn pt(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn make_pass(key: PassKey, shapes: Vec<PlannedShape>) -> DocumentPass {
        DocumentPass { key, shapes }
    }

    fn make_shape(node_id: u64, polylines: Vec<Vec<Point>>) -> PlannedShape {
        PlannedShape {
            node_id: NodeId(node_id),
            polylines,
        }
    }

    fn make_configured_pass<'a>(pass: &'a DocumentPass, settings: Settings, enabled: bool) -> ConfiguredPass<'a> {
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
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::NothingToCut));
    }

    #[test]
    fn nothing_to_cut_ignores_disabled_passes_with_content() {
        let shape = make_shape(1, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), false)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::NothingToCut));
    }

    #[test]
    fn non_finite_geometry_detects_nan() {
        let shape = make_shape(1, vec![vec![pt(10.0, 10.0), pt(f64::NAN, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::NonFiniteGeometry(NodeId(1))));
    }

    #[test]
    fn non_finite_geometry_detects_inf() {
        let shape = make_shape(2, vec![vec![pt(10.0, 10.0), pt(f64::INFINITY, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::NonFiniteGeometry(NodeId(2))));
    }

    #[test]
    fn degenerate_polyline_single_point() {
        let shape = make_shape(3, vec![vec![pt(10.0, 10.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::DegeneratePolyline(NodeId(3))));
    }

    #[test]
    fn degenerate_polyline_empty() {
        let shape = make_shape(4, vec![vec![]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::DegeneratePolyline(NodeId(4))));
    }

    #[test]
    fn non_finite_wins_over_degenerate_polyline() {
        // Rule 2 (NaN/inf) checked before rule 3 (degenerate polyline):
        // job with early degenerate polyline + later NaN → NonFiniteGeometry wins
        let shape1 = make_shape(100, vec![vec![pt(10.0, 10.0)]]);  // degenerate: 1 point
        let shape2 = make_shape(101, vec![vec![pt(20.0, 20.0), pt(f64::NAN, 30.0)]]); // has NaN
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape1, shape2]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::NonFiniteGeometry(NodeId(101))));
    }

    #[test]
    fn out_of_bounds_x_negative() {
        let shape = make_shape(5, vec![vec![pt(-1.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
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
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
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
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
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
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
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
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let configured = vec![make_configured_pass(&pass, Settings::default(), true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, true);
        assert!(result.is_ok());
    }

    #[test]
    fn repeat_count_below_1_rejected() {
        let shape = make_shape(10, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let settings = Settings { speed: None, force: None, repeat_count: 0 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("repeat_count must be 1..=10")));
    }

    #[test]
    fn repeat_count_above_10_rejected() {
        let shape = make_shape(11, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let settings = Settings { speed: None, force: None, repeat_count: 11 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("repeat_count must be 1..=10")));
    }

    #[test]
    fn speed_unsupported_by_device_ignored() {
        // Unsupported speed is ignored (drivers skip it); should pass preflight
        let shape = make_shape(12, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let settings = Settings { speed: Some(15), force: None, repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert!(result.is_ok());
    }

    #[test]
    fn speed_below_1_rejected() {
        let shape = make_shape(13, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let settings = Settings { speed: Some(0), force: None, repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_with_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("speed must be 1..=30")));
    }

    #[test]
    fn speed_above_30_rejected() {
        let shape = make_shape(14, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let settings = Settings { speed: Some(31), force: None, repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_with_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("speed must be 1..=30")));
    }

    #[test]
    fn force_unsupported_by_device_ignored() {
        // Unsupported force is ignored (drivers skip it); should pass preflight
        let shape = make_shape(15, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let settings = Settings { speed: None, force: Some(15), repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert!(result.is_ok());
    }

    #[test]
    fn force_below_1_rejected() {
        let shape = make_shape(16, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let settings = Settings { speed: None, force: Some(0), repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_with_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("force must be 1..=33")));
    }

    #[test]
    fn force_above_33_rejected() {
        let shape = make_shape(17, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let settings = Settings { speed: None, force: Some(34), repeat_count: 1 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_with_speed_force(), None, false);
        assert_eq!(result, Err(PreflightError::SettingsOutOfRange("force must be 1..=33")));
    }

    #[test]
    fn machine_mismatch_doc_id_differs() {
        let shape = make_shape(18, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
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
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
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
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let settings = Settings { speed: None, force: None, repeat_count: 10 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert!(matches!(result, Err(PreflightError::OutputTooLarge(_))));
    }

    #[test]
    fn output_size_weights_each_pass_by_its_own_repeat_count() {
        // Big pass at repeat 1, tiny pass at repeat 10. Per-pass weighting:
        // 500000×16×1 + 10×16×10 ≈ 8 MB → fine. The old all-passes max formula
        // charged the big pass at repeat 10 too (80 MB) and over-rejected this.
        let mut points = vec![];
        for i in 0..500000 {
            points.push(pt((i % 100) as f64, ((i / 100) % 100) as f64));
        }
        let big = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![make_shape(20, vec![points])]);
        let tiny = make_pass(PassKey::Color(Some(0x00FF00FF)), vec![make_shape(21, vec![vec![pt(0.0, 0.0); 10]])]);
        let configured = vec![
            make_configured_pass(&big, Settings { speed: None, force: None, repeat_count: 1 }, true),
            make_configured_pass(&tiny, Settings { speed: None, force: None, repeat_count: 10 }, true),
        ];
        let result = preflight(&configured, &profile_100x100(), &caps_no_speed_force(), None, false);
        assert!(result.is_ok());
    }

    #[test]
    fn happy_path_valid_cut() {
        let shape = make_shape(21, vec![vec![pt(10.0, 10.0), pt(20.0, 20.0), pt(30.0, 10.0)]]);
        let pass = make_pass(PassKey::Color(Some(0xFF0000FF)), vec![shape]);
        let settings = Settings { speed: Some(15), force: Some(20), repeat_count: 3 };
        let configured = vec![make_configured_pass(&pass, settings, true)];
        let result = preflight(&configured, &profile_100x100(), &caps_with_speed_force(), None, false);
        assert_eq!(result, Ok(()));
    }

    /// Both the whole table at once: a new variant fails to compile the match in
    /// `Display`/`code`, and a reworded one fails here. These strings are what an
    /// operator reads, so they are worth pinning — four of them used to reach a
    /// CLI user as `preflight: MachineMismatch { .. }`, which is why this type
    /// gained `Display` at all.
    #[test]
    fn every_refusal_has_a_sentence_and_a_code() {
        let cases: Vec<(PreflightError, &str, &str)> = vec![
            (
                PreflightError::NothingToCut,
                "nothing_to_cut",
                "no pass selected for this cut has any geometry",
            ),
            (
                PreflightError::NonFiniteGeometry(NodeId(3)),
                "non_finite_geometry",
                "shape #3 has a coordinate that is not a finite number",
            ),
            (
                PreflightError::DegeneratePolyline(NodeId(4)),
                "degenerate_polyline",
                "shape #4 has a path with fewer than two points",
            ),
            (
                PreflightError::OutOfBounds { node: NodeId(5), bounds: (0.0, 0.0, 304.8, 304.8) },
                "out_of_bounds",
                "shape #5 lies outside the 304.8 x 304.8 mm cutting area",
            ),
            (
                PreflightError::SettingsOutOfRange("speed must be 1..=30"),
                "settings_out_of_range",
                "speed must be 1..=30",
            ),
            (
                PreflightError::MachineMismatch { document: "puma".into(), device: "cameo5".into() },
                "machine_mismatch",
                "this document is set up for `puma`, but the connected machine is `cameo5`",
            ),
            (
                PreflightError::OutputTooLarge(80_000_000),
                "output_too_large",
                "the encoded cut is about 77 MB, over the 64 MB limit",
            ),
        ];
        for (error, code, message) in cases {
            assert_eq!(error.code(), code, "code for {error:?}");
            assert_eq!(error.to_string(), message, "message for {error:?}");
        }
    }

    /// A bound moved in `SETTINGS_RANGES` and a sentence left behind names an edge that is no
    /// longer the edge — the operator reads it, types that number, and is refused again. The
    /// messages are `&'static str`, so only a test can hold the two together.
    #[test]
    fn every_range_refusal_names_the_bound_it_enforced() {
        let caps = caps_with_speed_force();
        let cases = [
            (
                Settings {
                    speed: None,
                    force: None,
                    repeat_count: SETTINGS_RANGES.repeat_count.max + 1,
                },
                SETTINGS_RANGES.repeat_count,
            ),
            (
                Settings { speed: Some(SETTINGS_RANGES.speed.max + 1), force: None, repeat_count: 1 },
                SETTINGS_RANGES.speed,
            ),
            (
                Settings { speed: None, force: Some(SETTINGS_RANGES.force.max + 1), repeat_count: 1 },
                SETTINGS_RANGES.force,
            ),
        ];
        for (settings, range) in cases {
            let message = settings_out_of_range(&settings, &caps)
                .expect("a value past the maximum is refused");
            let stated = format!("{}..={}", range.min, range.max);
            assert!(
                message.contains(&stated),
                "`{message}` does not state the range it enforced ({stated})",
            );
        }
    }

    /// The preset editor reads these keys off the wire. Renaming a field on either side leaves
    /// it with no bounds at all, which reads as a field with no limits.
    #[test]
    fn settings_ranges_serialize_in_the_casing_the_ui_reads() {
        assert_eq!(
            serde_json::to_value(SETTINGS_RANGES).unwrap(),
            serde_json::json!({
                "speed": { "min": 1, "max": 30 },
                "force": { "min": 1, "max": 33 },
                "repeatCount": { "min": 1, "max": 10 },
            }),
        );
    }

    /// A machine that ignores speed is still handed a preset that states one, and that number
    /// stays in its file for the next machine and the next edit. So a stored preset is checked
    /// whole, where a cut's Settings is checked against what the machine honours.
    #[test]
    fn a_stored_preset_is_refused_a_speed_a_cut_would_have_ignored() {
        let stored = crate::presets::PresetSettings {
            speed: Some(SETTINGS_RANGES.speed.max + 1),
            force: None,
            repeat_count: 1,
        };
        assert_eq!(
            settings_out_of_range(
                &Settings { speed: stored.speed, force: None, repeat_count: stored.repeat_count },
                &caps_no_speed_force(),
            ),
            None,
            "premise: a machine without speed support has its speed ignored, not refused",
        );
        assert_eq!(
            preset_settings_out_of_range(&stored),
            Some("speed must be 1..=30"),
            "a preset's out-of-range speed was let through to its file",
        );
    }
}
