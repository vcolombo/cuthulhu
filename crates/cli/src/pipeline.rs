// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{DeviceBackendFactory, Driver, Job, Settings};
use driver_registry::HardwareBackendFactory;

#[derive(Clone, Copy)]
pub enum Device { Cameo5, Puma }
impl Device {
    pub fn from_id(s: &str) -> Result<Device, String> {
        match s {
            "cameo5" => Ok(Device::Cameo5),
            "puma" => Ok(Device::Puma),
            _ => Err(format!("unknown device '{s}' (try: cameo5, puma)")),
        }
    }
    fn machine_id(&self) -> &'static str {
        match self {
            Device::Cameo5 => "cameo5",
            Device::Puma => "puma",
        }
    }
    pub fn driver(&self) -> Box<dyn Driver> {
        HardwareBackendFactory.driver_for(self.machine_id())
            .expect("Device variant always maps to a known machine_id")
    }
}

/// Bytes for pass `i` of `total`, framed exactly as `DeviceManager` transmits
/// them: `session_begin` before the first pass, `pass_park` between passes,
/// `session_end` after the last. Keeps `cut --by-color --dry-run` output
/// faithful to what a real multi-pass cut sends.
pub fn pass_stream_bytes(d: &dyn Driver, job: &Job, i: usize, total: usize) -> Result<Vec<u8>, String> {
    let mut bytes = if i == 0 { d.session_begin() } else { Vec::new() };
    bytes.extend(d.encode_pass(job).map_err(|e| format!("encode: {e:?}"))?);
    if i + 1 == total {
        bytes.extend(d.session_end());
    } else {
        bytes.extend(d.pass_park());
    }
    Ok(bytes)
}

/// Import `svg` into a fresh `Document` — the CLI has no editing model, so a
/// cut is planned against a document that exists only for this command.
pub fn doc_from_svg(svg: &[u8]) -> Result<document::Document, String> {
    let mut doc = document::Document::new();
    let (delta, _skipped) = fileio::import_svg(svg, &mut doc.ids, doc.root)
        .map_err(|e| format!("SVG parse: {e:?}"))?;
    doc.apply(delta);
    Ok(doc)
}

/// The stroke a plain cut gives every imported path. Opaque black, matching
/// `document::Style::default()`.
pub const CUT_STROKE: u32 = 0x000000FF;

/// Import `svg` for a plain (non-`--by-color`) cut: every path gets the same
/// stroke, so `plan_passes` finds all of it and groups it into exactly one
/// `ColorPass`.
///
/// This is what the plain path has always meant — cut everything in the file,
/// in one pass — stated explicitly so the cut can go through `plan_cut` and be
/// preflighted. It deliberately does not touch `plan_passes`' stroke rule; see
/// issue #68 for whether that rule should change at all.
pub fn doc_from_svg_all_cuttable(svg: &[u8]) -> Result<document::Document, String> {
    let mut doc = document::Document::new();
    let (mut delta, _skipped) = fileio::import_svg(svg, &mut doc.ids, doc.root)
        .map_err(|e| format!("SVG parse: {e:?}"))?;
    for op in delta.0.iter_mut() {
        if let document::NodeOp::Add { node, .. } = op {
            node.style.stroke = Some(CUT_STROKE);
        }
    }
    doc.apply(delta);
    Ok(doc)
}

/// The colours to cut, in cut order: apply `--order` (listed colours to the
/// front, in listed sequence; the rest keep their relative order) and then
/// `--skip-color`, in that order per the brief.
pub fn pass_order(
    planned: &[cutplan::ColorPass],
    skip_colors: &[String],
    order: Option<String>,
) -> Result<Vec<Option<u32>>, String> {
    let mut colors: Vec<Option<u32>> = planned.iter().map(|p| p.color).collect();

    if let Some(order) = order {
        let wanted: Vec<u32> = order.split(',').map(|s| parse_hex_color(s.trim())).collect::<Result<_, _>>()?;
        let mut front = vec![];
        for color in wanted {
            if let Some(i) = colors.iter().position(|c| *c == Some(color)) {
                front.push(colors.remove(i));
            }
        }
        front.extend(colors);
        colors = front;
    }

    let skip: Vec<u32> = skip_colors.iter().map(|s| parse_hex_color(s)).collect::<Result<_, _>>()?;
    colors.retain(|c| !c.is_some_and(|c| skip.contains(&c)));
    Ok(colors)
}

/// Plan a `--by-color` cut from an SVG: import, order, select, and validate
/// through `cutplan::plan_cut` — the same entry point the desktop uses, so the
/// CLI gets preflight rather than sending unchecked geometry at the machine.
pub fn plan_cut_from_svg(
    svg: &[u8],
    device: Device,
    settings: &Settings,
    skip_colors: &[String],
    order: Option<String>,
    allow_out_of_bounds: bool,
) -> Result<cutplan::CutPlan, String> {
    let doc = doc_from_svg(svg)?;
    // Planned once: --order and --skip-color name colours, so the colours have
    // to be known before a selection can be built, and plan_cut cuts the very
    // passes handed to it here.
    let planned = cutplan::plan_passes(&doc).map_err(|e| format!("plan: {e:?}"))?;
    let colors = pass_order(&planned.passes, skip_colors, order)?;

    // One `--speed`/`--force` pair applies to every pass; the CLI has no
    // per-pass settings and no presets.
    let passes = colors
        .into_iter()
        .map(|color| cutplan::PassSelection { color, settings: settings.clone() })
        .collect();

    let driver = device.driver();
    // No revision to be stale against: the document was imported a few lines ago.
    let opts = cutplan::PlanOptions { passes, expect_revision: None, allow_out_of_bounds };
    cutplan::plan_cut(&planned, driver.profile(), &driver.caps(), &opts).map_err(describe_cut_error)
}

/// Plan a plain cut: all geometry, one pass, validated through `plan_cut` — the
/// same entry point the desktop and `--by-color` use.
pub fn plan_plain_cut(
    svg: &[u8],
    device: Device,
    settings: &Settings,
    allow_out_of_bounds: bool,
) -> Result<cutplan::CutPlan, String> {
    let doc = doc_from_svg_all_cuttable(svg)?;
    let planned = cutplan::plan_passes(&doc).map_err(|e| format!("plan: {e:?}"))?;
    // Checked here rather than left to `plan_cut`: with no passes at all, asking for
    // CUT_STROKE is an unmatched colour, and "no pass matches color" describes the
    // request instead of the file.
    if planned.passes.is_empty() {
        return Err("no cuttable paths in SVG".into());
    }
    let passes = vec![cutplan::PassSelection { color: Some(CUT_STROKE), settings: settings.clone() }];
    let driver = device.driver();
    let opts = cutplan::PlanOptions { passes, expect_revision: None, allow_out_of_bounds };
    cutplan::plan_cut(&planned, driver.profile(), &driver.caps(), &opts).map_err(describe_cut_error)
}

/// `CutError` as something to print at a terminal. Out-of-bounds names the
/// escape hatch, since that is the one refusal an operator may reasonably
/// want to overrule.
fn describe_cut_error(e: cutplan::CutError) -> String {
    use cutplan::preflight::PreflightError as P;
    match e {
        cutplan::CutError::StalePlan { .. } => "document changed while planning".into(),
        cutplan::CutError::UnknownPassColor(c) => format!("no pass matches color {c:?}"),
        cutplan::CutError::Preflight(P::NothingToCut) => "no cuttable paths in SVG".into(),
        cutplan::CutError::Preflight(P::OutOfBounds { node, bounds }) => format!(
            "shape {node:?} lies outside the {} x {} mm cutting area — pass --allow-out-of-bounds to send it anyway",
            bounds.2, bounds.3,
        ),
        cutplan::CutError::Preflight(P::SettingsOutOfRange(m)) => m.into(),
        cutplan::CutError::Preflight(e) => format!("preflight: {e:?}"),
    }
}

/// Parse an 8-hex-digit `RRGGBBAA` string into a `0xRRGGBBAA` color.
/// Parses an 8-digit `RRGGBBAA` hex color. The length check is required: without
/// it a 6-digit `RRGGBB` parses as `0x00RRGGBB` and silently matches nothing.
pub fn parse_hex_color(s: &str) -> Result<u32, String> {
    if s.len() != 8 {
        return Err(format!("bad color '{s}': expected 8 hex digits (RRGGBBAA)"));
    }
    u32::from_str_radix(s, 16).map_err(|e| format!("bad color '{s}': {e}"))
}

/// `--skip-color` and `--order` select and sequence colours, which only a
/// `--by-color` cut has. A plain cut puts every colour in one pass, so these
/// flags cannot do anything there and are refused rather than ignored.
pub fn check_color_flag_scope(
    skip_colors: &[String],
    order: &Option<String>,
    by_color: bool,
) -> Result<(), String> {
    if by_color {
        return Ok(());
    }
    if !skip_colors.is_empty() {
        return Err("--skip-color applies to --by-color cuts; a plain cut is one pass over every colour".into());
    }
    if order.is_some() {
        return Err("--order applies to --by-color cuts; a plain cut is one pass over every colour".into());
    }
    Ok(())
}

/// `--by-color` needs a human at the keyboard between passes; a plan with
/// only one pass never pauses, so it's allowed even without a TTY.
pub fn check_interactive(is_tty: bool, pass_count: usize) -> Result<(), String> {
    if !is_tty && pass_count > 1 {
        return Err("--by-color requires an interactive terminal".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_color_svg() -> &'static [u8] {
        br##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect width="5" height="5" stroke="#ff0000" fill="none"/>
            <circle cx="10" cy="10" r="3" stroke="#0000ff" fill="none"/>
        </svg>"##
    }

    fn cut_settings() -> Settings {
        Settings { speed: None, force: None, repeat_count: 1 }
    }

    #[test]
    fn by_color_plans_from_svg_respects_skip_and_order() {
        let doc = doc_from_svg(two_color_svg()).unwrap();
        let planned = cutplan::plan_passes(&doc).unwrap();
        let colors = pass_order(&planned.passes, &["ff0000ff".into()], Some("0000ffff,ff0000ff".into())).unwrap();
        assert_eq!(colors.len(), 1, "red skipped"); // order flag applied before skip filter
        assert_eq!(colors[0], Some(0x0000FFFF));
    }

    #[test]
    fn out_of_bounds_geometry_is_refused_unless_allowed() {
        // 1512px @96dpi = 400mm wide, past the Cameo's 330mm bed.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect width="1512" height="10" stroke="#ff0000" fill="none"/>
        </svg>"##;

        let err = plan_cut_from_svg(svg, Device::Cameo5, &cut_settings(), &[], None, false).unwrap_err();
        assert!(err.contains("outside"), "expected an out-of-bounds refusal, got: {err}");

        assert!(
            plan_cut_from_svg(svg, Device::Cameo5, &cut_settings(), &[], None, true).is_ok(),
            "--allow-out-of-bounds must let it through",
        );
    }

    #[test]
    fn settings_out_of_range_are_refused_before_reaching_the_machine() {
        let bad = Settings { speed: Some(99), force: None, repeat_count: 1 };
        let err = plan_cut_from_svg(two_color_svg(), Device::Cameo5, &bad, &[], None, false).unwrap_err();
        assert!(err.contains("speed"), "expected a settings-range refusal, got: {err}");
    }

    #[test]
    fn an_svg_with_nothing_stroked_is_refused_by_name() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect width="5" height="5" fill="#ff0000"/>
        </svg>"##;
        let err = plan_cut_from_svg(svg, Device::Cameo5, &cut_settings(), &[], None, false).unwrap_err();
        assert_eq!(err, "no cuttable paths in SVG");
    }

    #[test]
    fn noninteractive_multicolor_is_error() {
        assert_eq!(
            check_interactive(false, 2),
            Err("--by-color requires an interactive terminal".into())
        );
        assert!(check_interactive(false, 1).is_ok());
        assert!(check_interactive(true, 2).is_ok());
    }

    #[test]
    fn parse_hex_color_requires_eight_digits() {
        assert_eq!(parse_hex_color("ff0000ff"), Ok(0xFF0000FF));
        assert!(parse_hex_color("ff0000").is_err(), "6-digit RRGGBB must be rejected, not zero-padded");
        assert!(parse_hex_color("nothex12").is_err());
    }

    /// A fill-only SVG is what Illustrator, Inkscape and most clipart emit. The plain
    /// cut path has always cut it, so routing that path through `plan_passes` — which
    /// skips strokeless shapes — must not change what it cuts.
    #[test]
    fn fill_only_svg_plans_exactly_one_pass() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <rect width="5" height="5" fill="#ff0000"/><rect x="6" width="5" height="5" fill="#00ff00"/></svg>"##;
        let doc = doc_from_svg_all_cuttable(svg).expect("import");
        let planned = cutplan::plan_passes(&doc).expect("plan");
        assert_eq!(planned.passes.len(), 1, "all geometry belongs to one pass");
        assert_eq!(planned.passes[0].color, Some(CUT_STROKE));
    }

    #[test]
    fn plain_cut_plans_one_pass() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <rect width="5" height="5" fill="#ff0000"/></svg>"##;
        let plan = plan_plain_cut(svg, Device::Cameo5, &Settings::default(), false).expect("plan");
        assert_eq!(plan.passes.len(), 1);
    }

    /// The whole point of the change: the plain path is preflighted. A shape past the
    /// bed's edge was silently sent to the machine before.
    #[test]
    fn plain_cut_refuses_out_of_bounds_geometry() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10000mm" height="10mm">
            <rect x="9000" width="500" height="5" fill="#000000"/></svg>"##;
        let err = plan_plain_cut(svg, Device::Cameo5, &Settings::default(), false)
            .expect_err("out of bounds must be refused");
        assert!(err.contains("outside"), "unexpected message: {err}");
        // ...and the escape hatch works, now that there is a check to overrule.
        assert!(plan_plain_cut(svg, Device::Cameo5, &Settings::default(), true).is_ok());
    }

    /// With no paths at all, `plan_passes` yields no passes, so the requested colour
    /// matches nothing. Without the empty check that surfaces as `UnknownPassColor`,
    /// which reads as an internal error rather than "there is nothing here".
    #[test]
    fn plain_cut_of_an_empty_svg_says_nothing_to_cut() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"></svg>"##;
        let err = plan_plain_cut(svg, Device::Cameo5, &Settings::default(), false).expect_err("empty");
        assert_eq!(err, "no cuttable paths in SVG");
    }

    /// `--skip-color` and `--order` name colours, and a plain cut deliberately
    /// collapses every colour into one pass. Accepting them silently — which is what
    /// happened before — reports success for a flag that did nothing.
    #[test]
    fn colour_flags_are_refused_without_by_color() {
        let red = vec!["FF0000FF".to_string()];
        let err = check_color_flag_scope(&red, &None, false).expect_err("must refuse");
        assert!(err.contains("--skip-color"), "unexpected message: {err}");
        let err = check_color_flag_scope(&[], &Some("FF0000FF".into()), false).expect_err("must refuse");
        assert!(err.contains("--order"), "unexpected message: {err}");
        // Both are fine with --by-color, and absence is fine either way.
        assert!(check_color_flag_scope(&red, &Some("FF0000FF".into()), true).is_ok());
        assert!(check_color_flag_scope(&[], &None, false).is_ok());
    }

    #[test]
    fn device_driver_routes_through_the_factory() {
        // Regression guard: Device::driver() must keep resolving via the shared
        // registry, not a hardcoded match, so the cut path and the enumeration
        // path agree. (Which ids the registry knows is its own test.)
        assert_eq!(Device::Cameo5.driver().profile().id, "cameo5");
        assert_eq!(Device::Puma.driver().profile().id, "puma");
    }
}
