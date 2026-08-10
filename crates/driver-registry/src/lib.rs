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

// The id each driver's own `MachineProfile` spells, bound once here so the
// table below and the enumerators agree with it.
const CAMEO5: &str = "cameo5";
const PUMA: &str = "puma";

struct Machine {
    id: &'static str,
    driver: fn() -> Box<dyn Driver + Send>,
    /// The attached hardware of this machine, if any. Per machine rather than
    /// one shared scan because the transports have nothing in common: USB is
    /// matched on VID/PID, serial is a list of ports that might be anything.
    enumerate: fn() -> Vec<DeviceInfo>,
    /// Whether an operator can name this machine's port with `--port`. A USB
    /// machine is discriminated by VID/PID and enumerates itself; a serial port
    /// identifies nothing, so it takes the operator's word. Naming a port for a
    /// USB machine would put its dialect on a wire nothing on it can read.
    serial: bool,
}

/// Every machine this build can drive. One row is the whole of adding a
/// machine: `list_devices` scans it, `driver_for` searches it, `machine_ids`
/// reads it, and `device_at_port` asks it what a port can name. A machine
/// therefore cannot be half-added — drivable but never enumerated, or
/// enumerated but missing from `cuthulhu list-devices` — which is what an id
/// list and a `match` and a hand-written scan left possible between them.
const MACHINES: [Machine; 2] = [
    Machine {
        id: CAMEO5,
        driver: || Box::new(SilhouetteDriver::new()),
        enumerate: cameo5_devices,
        serial: false,
    },
    Machine { id: PUMA, driver: || Box::new(HpglDriver::new()), enumerate: puma_devices, serial: true },
];

fn cameo5_devices() -> Vec<DeviceInfo> {
    driver_silhouette::list_locators()
        .into_iter()
        .map(|locator| DeviceInfo {
            instance_id: format!("usb:{locator}"),
            machine_id: CAMEO5.into(),
            transport: TransportKind::Usb { locator },
            candidate: false, // USB is discriminated by VID/PID — not a guess
            // Enumerated here, so it is on this computer. A Cut Host's cutters get their id
            // stamped on by whoever fetched them, because the daemon does not know its own.
            host: None,
        })
        .collect()
}

fn puma_devices() -> Vec<DeviceInfo> {
    driver_hpgl::list_ports()
        .into_iter()
        .map(|path| DeviceInfo {
            instance_id: format!("serial:{path}"),
            machine_id: PUMA.into(),
            transport: TransportKind::Serial { path, baud: 9600 },
            candidate: true, // any serial port could be a Puma — needs operator confirmation
            host: None,
        })
        .collect()
}

/// The machines a caller can offer as a choice, in a stable order.
pub fn machine_ids() -> Vec<&'static str> {
    MACHINES.iter().map(|m| m.id).collect()
}

/// Whether naming a port for `machine_id` means anything — false for an unknown
/// machine and for one that does not connect over serial. Worth asking before
/// the fact is needed, so that a message about a missing device can offer
/// `--port` to the machines it can actually help.
pub fn takes_a_named_port(machine_id: &str) -> bool {
    MACHINES.iter().any(|m| m.id == machine_id && m.serial)
}

/// The device an operator names with `--port`, or `None` if that machine cannot
/// be named that way. Enumeration cannot answer this: a serial port announces
/// nothing about what is on the other end, so the operator's word is all there
/// is — and it is only worth taking for a machine that speaks serial.
pub fn device_at_port(machine_id: &str, path: &str, baud: u32) -> Option<DeviceInfo> {
    takes_a_named_port(machine_id).then(|| DeviceInfo {
        instance_id: format!("serial:{path}"),
        machine_id: machine_id.to_string(),
        transport: TransportKind::Serial { path: path.to_string(), baud },
        candidate: true,
        host: None,
    })
}

/// Enumerates attached USB/serial hardware and builds the driver for it.
pub struct HardwareBackendFactory;

impl DeviceBackendFactory for HardwareBackendFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> {
        MACHINES.iter().flat_map(|m| (m.enumerate)()).collect()
    }

    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
        MACHINES.iter().find(|m| m.id == machine_id).map(|m| (m.driver)())
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

    /// `MACHINES` ties enumeration, resolution and every caller's list of
    /// choices together, but each driver still spells its own id independently
    /// in its `MachineProfile`. This pins that copy to the rest: an enumerated
    /// device must resolve to a driver that answers to the same id, or a
    /// connect would hand the wrong encoder to a machine. Unknown ids must stay
    /// unresolvable rather than defaulting.
    #[test]
    fn enumerated_machine_ids_resolve_to_drivers_that_claim_them() {
        for id in machine_ids() {
            let driver = HardwareBackendFactory.driver_for(id).expect("known machine id");
            assert_eq!(driver.profile().id, id);
        }
        assert!(HardwareBackendFactory.driver_for("unknown").is_none());
    }

    /// A row's enumerator must label its devices with that row's id, or a
    /// connect would look the driver up under a name the row does not answer
    /// to. Vacuous with nothing plugged in — which is the only state CI has —
    /// but it is the assertion that would catch a copy-pasted row.
    #[test]
    fn each_machine_enumerates_devices_under_its_own_id() {
        for m in MACHINES.iter() {
            for device in (m.enumerate)() {
                assert_eq!(device.machine_id, m.id);
            }
        }
    }

    /// `--port` is the operator saying "it is here". That is worth taking for a
    /// machine that speaks serial and meaningless for one that does not: a USB
    /// machine reached this way would get its dialect written to whatever
    /// happens to be on that port.
    #[test]
    fn only_a_serial_machine_can_be_named_at_a_port() {
        let puma = device_at_port(PUMA, "/dev/ttyUSB0", 19200).expect("the Puma is a serial machine");
        assert_eq!(puma.machine_id, PUMA);
        assert_eq!(puma.transport, TransportKind::Serial { path: "/dev/ttyUSB0".into(), baud: 19200 });
        assert!(puma.candidate, "an operator-named port is still unverified hardware");

        assert!(device_at_port(CAMEO5, "/dev/ttyUSB0", 9600).is_none(), "the Cameo is USB-only");
        assert!(device_at_port("unknown", "/dev/ttyUSB0", 9600).is_none());
    }
}
