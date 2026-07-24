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

/// Import `svg` into a fresh `Document`, plan passes, then apply `--order`
/// (reorder listed colors to the front, in listed sequence; unlisted passes
/// keep their original relative order after) and `--skip-color` (drop
/// matching passes), in that order per the brief.
pub fn plan_from_svg(
    svg: &[u8],
    skip_colors: &[String],
    order: Option<String>,
) -> Result<Vec<cutplan::ColorPass>, String> {
    let mut doc = document::Document::new();
    let (delta, _skipped) = fileio::import_svg(svg, &mut doc.ids, doc.root)
        .map_err(|e| format!("SVG parse: {e:?}"))?;
    doc.apply(delta);
    let planned = cutplan::plan_passes(&doc).map_err(|e| format!("plan: {e:?}"))?;
    let mut passes = planned.passes;

    if let Some(order) = order {
        let wanted: Vec<u32> = order.split(',').map(|s| parse_hex_color(s.trim())).collect::<Result<_, _>>()?;
        let mut front = vec![];
        for color in wanted {
            if let Some(i) = passes.iter().position(|p| p.color == Some(color)) {
                front.push(passes.remove(i));
            }
        }
        front.extend(passes);
        passes = front;
    }

    let skip: Vec<u32> = skip_colors.iter().map(|s| parse_hex_color(s)).collect::<Result<_, _>>()?;
    passes.retain(|p| !p.color.is_some_and(|c| skip.contains(&c)));
    Ok(passes)
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

/// Flatten one `ColorPass`'s shapes into a single `Job` for `DeviceManager::cut`.
pub fn cutpass_from_color_pass(pass: &cutplan::ColorPass, settings: &Settings) -> driver_core::manager::CutPass {
    let polylines = pass.shapes.iter().flat_map(|s| s.polylines.clone()).collect();
    driver_core::manager::CutPass { job: Job { polylines, settings: settings.clone() } }
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

    #[test]
    fn by_color_plans_from_svg_respects_skip_and_order() {
        let svg = two_color_svg();
        let plan = plan_from_svg(svg, &["ff0000ff".into()], Some("0000ffff,ff0000ff".into())).unwrap();
        assert_eq!(plan.len(), 1, "red skipped"); // order flag applied before skip filter
        assert_eq!(plan[0].color, Some(0x0000FFFF));
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
