// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{DeviceBackendFactory, DeviceInfo, Driver, Job, Settings, Transport, TransportError, TransportKind};
use driver_hpgl::HpglDriver;
use driver_silhouette::SilhouetteDriver;

/// The CLI's `DeviceBackendFactory`: enumerates real USB/serial hardware and builds the
/// in-tree drivers. `Device::driver()` routes through this so the `cut` command exercises
/// the same contract a future `DeviceManager` will use.
pub struct CliBackendFactory;

impl DeviceBackendFactory for CliBackendFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> {
        let mut devices: Vec<DeviceInfo> = driver_silhouette::list_locators()
            .into_iter()
            .map(|locator| DeviceInfo {
                instance_id: format!("usb:{locator}"),
                machine_id: "cameo5".into(),
                transport: TransportKind::Usb { locator },
                candidate: false, // USB is discriminated by VID/PID — not a guess
            })
            .collect();
        devices.extend(driver_hpgl::list_ports().into_iter().map(|path| DeviceInfo {
            instance_id: format!("serial:{path}"),
            machine_id: "puma".into(),
            transport: TransportKind::Serial { path, baud: 9600 },
            candidate: true, // any serial port could be a Puma — needs operator confirmation
        }));
        devices
    }

    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
        match machine_id {
            "cameo5" => Some(Box::new(SilhouetteDriver::new())),
            "puma" => Some(Box::new(HpglDriver::new())),
            _ => None,
        }
    }

    fn open_transport(&self, info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
        match &info.transport {
            TransportKind::Usb { locator } => Ok(Box::new(driver_silhouette::UsbTransport::open_at(locator)?)),
            TransportKind::Serial { path, baud } => Ok(Box::new(driver_hpgl::SerialTransport::open(path, *baud)?)),
        }
    }
}

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
        CliBackendFactory.driver_for(self.machine_id())
            .expect("Device variant always maps to a known machine_id")
    }
}

pub fn build_bytes(svg: &[u8], device: Device, settings: &Settings) -> Result<Vec<u8>, String> {
    let imp = fileio::svg_to_paths(svg).map_err(|e| format!("SVG parse: {e:?}"))?;
    let polylines = imp.paths.iter()
        .flat_map(|(path, _)| path.flatten(0.1))
        .collect::<Vec<_>>();
    if polylines.is_empty() { return Err("no cuttable paths in SVG".into()); }
    let job = Job { polylines, settings: settings.clone() };
    let d = device.driver();
    let mut bytes = d.session_begin();
    bytes.extend(d.encode_pass(&job).map_err(|e| format!("encode: {e:?}"))?);
    bytes.extend(d.session_end());
    Ok(bytes)
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

/// `--allow-out-of-bounds` relaxes a preflight rule, and only `--by-color`
/// cuts are preflighted. Accepting it on the plain path would say a check was
/// overruled when no check ran.
pub fn check_out_of_bounds_scope(allow_out_of_bounds: bool, by_color: bool) -> Result<(), String> {
    if allow_out_of_bounds && !by_color {
        return Err("--allow-out-of-bounds applies to --by-color cuts; the plain cut path runs no preflight".into());
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

/// `#RRGGBB` for the operator prompt — drop the alpha byte.
pub fn format_pass_color(color: Option<u32>) -> String {
    match color {
        Some(c) => format!("#{:06x}", c >> 8),
        None => "none".into(),
    }
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
    fn allow_out_of_bounds_without_by_color_is_an_error() {
        // Silently accepting it would imply preflight ran and was relaxed, when
        // the plain cut path runs no preflight at all.
        assert!(check_out_of_bounds_scope(true, false).is_err());
        assert!(check_out_of_bounds_scope(true, true).is_ok());
        assert!(check_out_of_bounds_scope(false, false).is_ok());
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

    #[test]
    fn factory_resolves_drivers_for_known_machine_ids_only() {
        let f = CliBackendFactory;
        assert!(f.driver_for("cameo5").is_some());
        assert!(f.driver_for("puma").is_some());
        assert!(f.driver_for("unknown").is_none());
    }

    #[test]
    fn device_driver_routes_through_the_factory() {
        // Regression guard: Device::driver() must keep resolving via CliBackendFactory,
        // not a hardcoded match, so the cut path and the enumeration path agree.
        assert_eq!(Device::Cameo5.driver().profile().id, "cameo5");
        assert_eq!(Device::Puma.driver().profile().id, "puma");
    }
}
