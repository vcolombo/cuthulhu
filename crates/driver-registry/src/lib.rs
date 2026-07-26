// SPDX-License-Identifier: GPL-3.0-or-later
//! The one place that maps real hardware to the in-tree Drivers and Transports.
//!
//! Lives in its own crate because both entry points need it: the CLI's `cut`
//! command and the desktop app's `DeviceManager`. Neither can own it — a
//! binary's factory is not reachable from the other binary, and `driver-core`
//! holds only the trait (it must not depend on concrete drivers).
use driver_core::{DeviceBackendFactory, DeviceInfo, Driver, Transport, TransportError, TransportKind};
use driver_hpgl::HpglDriver;
use driver_silhouette::SilhouetteDriver;

// Enumeration and resolution have to agree on these, and only the resolution
// half is reachable from a test — `list_devices` talks to real hardware. One
// binding each keeps the two halves from drifting where nothing would catch it.
const CAMEO5: &str = "cameo5";
const PUMA: &str = "puma";

/// Enumerates attached USB/serial hardware and builds the driver for it.
pub struct HardwareBackendFactory;

impl DeviceBackendFactory for HardwareBackendFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> {
        let mut devices: Vec<DeviceInfo> = driver_silhouette::list_locators()
            .into_iter()
            .map(|locator| DeviceInfo {
                instance_id: format!("usb:{locator}"),
                machine_id: CAMEO5.into(),
                transport: TransportKind::Usb { locator },
                candidate: false, // USB is discriminated by VID/PID — not a guess
            })
            .collect();
        devices.extend(driver_hpgl::list_ports().into_iter().map(|path| DeviceInfo {
            instance_id: format!("serial:{path}"),
            machine_id: PUMA.into(),
            transport: TransportKind::Serial { path, baud: 9600 },
            candidate: true, // any serial port could be a Puma — needs operator confirmation
        }));
        devices
    }

    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
        match machine_id {
            CAMEO5 => Some(Box::new(SilhouetteDriver::new())),
            PUMA => Some(Box::new(HpglDriver::new())),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `CAMEO5`/`PUMA` tie enumeration to resolution, but each driver still
    /// spells its own id independently in its `MachineProfile`. This pins that
    /// third copy to the other two: an enumerated device must resolve to a
    /// driver that answers to the same id, or a connect would hand the wrong
    /// encoder to a machine. Unknown ids must stay unresolvable rather than
    /// defaulting — which also proves the `match` arms above are const
    /// patterns and not catch-all bindings.
    #[test]
    fn enumerated_machine_ids_resolve_to_drivers_that_claim_them() {
        for id in [CAMEO5, PUMA] {
            let driver = HardwareBackendFactory.driver_for(id).expect("known machine id");
            assert_eq!(driver.profile().id, id);
        }
        assert!(HardwareBackendFactory.driver_for("unknown").is_none());
    }
}
