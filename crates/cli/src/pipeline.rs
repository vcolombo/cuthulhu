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

#[cfg(test)]
mod tests {
    use super::*;

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
