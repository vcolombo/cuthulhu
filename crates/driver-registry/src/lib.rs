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

/// An instance id names one physical machine, and has to keep meaning that machine after a
/// reboot, a replug, or a hub enumerating in a different order.
///
/// So it is built from what the hardware says about itself — a serial number — and falls back to
/// where it was found only when the device reports nothing better. The two forms are spelled
/// differently on purpose: `usb:sn:…` promises the same machine, `usb:at:…` promises only the
/// same socket, and a caller saving one is entitled to know which it has.
///
/// This matters most where it is least visible. Two cutters of the same model on one host both
/// pass the machine-id check, so a swapped address is not a refusal — it is a Job cut on the
/// wrong machine, with no error anywhere.
fn usb_instance_id(locator: &str) -> String {
    match locator.parse::<driver_silhouette::Locator>() {
        Ok(l) if l.is_stable() => format!("usb:{l}"),
        _ => format!("usb:at:{locator}"),
    }
}

fn serial_instance_id(port: &driver_hpgl::PortId) -> String {
    match &port.serial_number {
        Some(sn) => format!("serial:sn:{sn}"),
        None => format!("serial:at:{}", port.path),
    }
}

fn cameo5_devices() -> Vec<DeviceInfo> {
    driver_silhouette::list_locators()
        .into_iter()
        .map(|locator| DeviceInfo {
            instance_id: usb_instance_id(&locator),
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
        .map(|port| DeviceInfo {
            instance_id: serial_instance_id(&port),
            machine_id: PUMA.into(),
            transport: TransportKind::Serial { path: port.path, baud: 9600 },
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
        // `at:`, not `sn:` — the operator named a socket, and a socket is all this promises.
        instance_id: format!("serial:at:{path}"),
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

    /// The bug these pin: an id built from where a device was found stops meaning that device
    /// the moment the OS enumerates differently. Two cutters of the same model then both pass
    /// the machine-id check, so a swap is not refused anywhere — it is a Job cut on the wrong
    /// machine.
    #[test]
    fn a_device_that_reports_a_serial_number_is_named_by_it() {
        assert_eq!(usb_instance_id("sn:CAMEO-A"), "usb:sn:CAMEO-A");
        assert_eq!(
            serial_instance_id(&driver_hpgl::PortId {
                path: "/dev/ttyUSB0".into(),
                serial_number: Some("FT-A".into()),
            }),
            "serial:sn:FT-A"
        );
    }

    /// Same machine, different socket: the id must not move with the socket.
    #[test]
    fn moving_a_device_to_another_socket_does_not_rename_it() {
        assert_eq!(usb_instance_id("sn:CAMEO-A"), usb_instance_id("sn:CAMEO-A"));

        let before = driver_hpgl::PortId {
            path: "/dev/ttyUSB0".into(),
            serial_number: Some("FT-A".into()),
        };
        let after_a_reboot = driver_hpgl::PortId {
            path: "/dev/ttyUSB1".into(),
            serial_number: Some("FT-A".into()),
        };
        assert_eq!(serial_instance_id(&before), serial_instance_id(&after_a_reboot));
    }

    /// Different machines in the same socket must not share an id, which is the half a
    /// path-based scheme gets wrong in the dangerous direction.
    #[test]
    fn two_devices_in_one_socket_do_not_share_an_id() {
        let a = driver_hpgl::PortId {
            path: "/dev/ttyUSB0".into(),
            serial_number: Some("FT-A".into()),
        };
        let b = driver_hpgl::PortId {
            path: "/dev/ttyUSB0".into(),
            serial_number: Some("FT-B".into()),
        };
        assert_ne!(serial_instance_id(&a), serial_instance_id(&b));
        assert_ne!(usb_instance_id("sn:CAMEO-A"), usb_instance_id("sn:CAMEO-B"));
    }

    /// A device with nothing to say about itself still has to be usable — named by socket, and
    /// spelled so a reader can tell that is all it promises.
    #[test]
    fn a_device_without_an_identity_is_named_by_its_socket_and_says_so() {
        assert_eq!(usb_instance_id("1:4"), "usb:at:1:4");
        assert_eq!(
            serial_instance_id(&driver_hpgl::PortId {
                path: "/dev/ttyUSB0".into(),
                serial_number: None,
            }),
            "serial:at:/dev/ttyUSB0"
        );
        // An operator naming a port with `--port` has named a socket, not a machine.
        let named = device_at_port(PUMA, "/dev/ttyUSB0", 9600).expect("the Puma is a serial machine");
        assert_eq!(named.instance_id, "serial:at:/dev/ttyUSB0");
    }

    /// The two forms must never collide: a device named by socket must not be mistaken for one
    /// named by identity, in either direction.
    #[test]
    fn a_socket_named_device_cannot_be_confused_with_an_identified_one() {
        assert_ne!(usb_instance_id("sn:1:4"), usb_instance_id("1:4"));
        let ids = [
            usb_instance_id("sn:CAMEO-A"),
            usb_instance_id("1:4"),
            serial_instance_id(&driver_hpgl::PortId {
                path: "/dev/ttyUSB0".into(),
                serial_number: Some("FT-A".into()),
            }),
            serial_instance_id(&driver_hpgl::PortId {
                path: "/dev/ttyUSB0".into(),
                serial_number: None,
            }),
        ];
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "every id form must be distinguishable: {ids:?}");
    }
}
