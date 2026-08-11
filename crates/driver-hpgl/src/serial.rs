// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Transport, TransportError};
use std::io::Read;
use std::time::Duration;

/// macOS exposes these permanently; none is ever a cutter.
const NEVER_A_CUTTER: [&str; 3] = ["Bluetooth-Incoming-Port", "debug-console", "wlan-debug"];

/// Drops ports that cannot be a cutter, keeping everything else — serial has no
/// VID/PID-equivalent discriminator, so surviving ports are still candidates the operator
/// must confirm.
///
/// Two rules, both name-based: `port_type` is not usable here because macOS reports every
/// port as `PciPort`, Bluetooth included.
///
/// 1. `/dev/tty.X` is dropped when `/dev/cu.X` is also present. macOS exposes each device
///    twice; the callout node (`cu.`) is the one to talk to, since the dial-in node blocks
///    on carrier detect. Linux names (`/dev/ttyUSB0`) lack the dot and are never matched.
/// 2. Known macOS system pseudo-ports are dropped by name.
fn usable_ports(names: Vec<String>) -> Vec<String> {
    let callouts: std::collections::HashSet<&str> =
        names.iter().filter_map(|n| n.strip_prefix("/dev/cu.")).collect();
    names
        .iter()
        .filter(|n| {
            let bare = n
                .strip_prefix("/dev/cu.")
                .or_else(|| n.strip_prefix("/dev/tty."));
            match bare {
                Some(b) if NEVER_A_CUTTER.contains(&b) => false,
                _ => match n.strip_prefix("/dev/tty.") {
                    Some(dialin) => !callouts.contains(dialin),
                    None => true,
                },
            }
        })
        .cloned()
        .collect()
}

/// One enumerated serial port: the node to open, and what the adapter says about itself.
///
/// `serial_number` is the USB-serial adapter's own, not the cutter's — a Puma is a plain RS-232
/// machine and says nothing over the wire about which one it is. That is still the best identity
/// available, because the adapter is bolted to one cutter and moves with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortId {
    /// The device node to open. Assigned in probe order, so it is not an identity.
    pub path: String,
    /// The adapter's serial number, where it reports one.
    pub serial_number: Option<String>,
}

impl PortId {
    /// Whether this port still means the same adapter after the OS re-enumerates.
    ///
    /// False means the port is named by `/dev/ttyUSB*`-style probe order, which a reboot or a
    /// replug can hand to a different device. A caller that saves a reference to this port must
    /// say so rather than implying the name will hold.
    pub fn is_stable(&self) -> bool {
        self.serial_number.is_some()
    }
}

/// The adapter's serial number, or `None` when it reports none or the port is not USB. Split out
/// so the rule can be tested without hardware.
fn port_serial_number(info: &serialport::SerialPortInfo) -> Option<String> {
    match &info.port_type {
        // An empty descriptor is one the adapter did not fill in, not an identity.
        serialport::SerialPortType::UsbPort(usb) => usb
            .serial_number
            .as_deref()
            .map(str::trim)
            .filter(|sn| !sn.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

/// Serial ports that could plausibly be a cutter (not narrowed to Puma devices — serial has no
/// VID/PID-equivalent discriminator here, so the caller must ask the operator).
pub fn list_ports() -> Vec<PortId> {
    let Ok(ports) = serialport::available_ports() else { return Vec::new() };
    let keep = usable_ports(ports.iter().map(|p| p.port_name.clone()).collect());
    ports
        .iter()
        .filter(|p| keep.contains(&p.port_name))
        .map(|p| PortId { path: p.port_name.clone(), serial_number: port_serial_number(p) })
        .collect()
}

/// Whether `path` names one of the `enumerated` ports, resolving symlinks before deciding.
///
/// A port can be opened through an alias — Linux's `/dev/serial/by-id/usb-...` is the stable
/// name to prefer — while enumeration reports the underlying `/dev/ttyUSB0`. Comparing the
/// two verbatim would call a healthy device unplugged and turn its every read timeout into a
/// disconnect, which is worse than the opaque error this whole check exists to fix.
///
/// A path that resolves to nothing is absent: the device node itself is gone.
fn port_is_present(path: &str, enumerated: &[String]) -> bool {
    if enumerated.iter().any(|p| p == path) {
        return true;
    }
    let Ok(ours) = std::fs::canonicalize(path) else { return false };
    enumerated
        .iter()
        .any(|p| std::fs::canonicalize(p).map(|other| other == ours).unwrap_or(false))
}

/// Maps a serial I/O failure to a transport error, using `still_present` (whether the OS
/// still lists the port) to tell an unplugged cable from a device-side error.
///
/// A vanished port is reported as a disconnect even when the failure was a timeout: the
/// completion poll treats `Timeout` as "keep polling" until its deadline, so a device that
/// has been unplugged would otherwise burn that whole deadline before failing.
fn classify_io_error(e: std::io::Error, still_present: bool) -> TransportError {
    if !still_present {
        return TransportError::Disconnected;
    }
    match e.kind() {
        std::io::ErrorKind::TimedOut => TransportError::Timeout,
        _ => TransportError::Io(e.to_string()),
    }
}

pub struct SerialTransport { port: Box<dyn serialport::SerialPort>, path: String }
impl SerialTransport {
    pub fn open(port: &str, baud: u32) -> Result<SerialTransport, TransportError> {
        let p = serialport::new(port, baud)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .timeout(Duration::from_secs(5))
            .open().map_err(|e| TransportError::Io(e.to_string()))?;
        Ok(SerialTransport { port: p, path: port.to_string() })
    }

    /// Whether the OS still lists this transport's port.
    ///
    /// Enumerates raw rather than through `list_ports`, which drops dial-in duplicates and
    /// system pseudo-ports: a transport opened on a path the filter hides (the CLI takes any
    /// `--port`) must not read as unplugged.
    fn still_present(&self) -> bool {
        match serialport::available_ports() {
            Ok(ports) => {
                let names: Vec<String> = ports.into_iter().map(|p| p.port_name).collect();
                port_is_present(&self.path, &names)
            }
            // Enumeration itself failed — an unreadable port table is not evidence the
            // cable is out.
            Err(_) => true,
        }
    }
}
impl Transport for SerialTransport {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
        use std::io::Write;
        self.port.write_all(bytes).map_err(|e| classify_io_error(e, self.still_present()))?;
        Ok(bytes.len())
    }
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError> {
        // Classified like the read below: if the port vanished, set_timeout is where the
        // failure lands first, and it should say disconnect rather than Io.
        self.port.set_timeout(timeout).map_err(|e| classify_io_error(e.into(), self.still_present()))?;
        match self.port.read(buf) {
            Ok(n) => Ok(n),
            Err(e) => Err(classify_io_error(e, self.still_present())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::io::ErrorKind;
    #[cfg(unix)]
    #[test]
    fn a_port_opened_through_an_alias_counts_as_present() {
        // Linux exposes stable aliases like /dev/serial/by-id/usb-FTDI-..., while
        // available_ports reports the underlying /dev/ttyUSB0. Comparing the names verbatim
        // calls a healthy device unplugged, which turns its every read timeout into a
        // disconnect and fails the job on the first poll.
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("ttyUSB0");
        std::fs::write(&real, b"").unwrap();
        let alias = dir.path().join("by-id-usb-cutter");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let enumerated = vec![real.to_string_lossy().into_owned()];

        assert!(port_is_present(&alias.to_string_lossy(), &enumerated), "alias resolves to an enumerated port");
        assert!(port_is_present(&real.to_string_lossy(), &enumerated), "the plain name still matches");
        let absent = dir.path().join("ttyUSB9");
        assert!(!port_is_present(&absent.to_string_lossy(), &enumerated), "a path that resolves to nothing is absent");
    }

    #[test]
    fn vanished_port_is_a_disconnect_whatever_the_os_said() {
        // Unplugging a USB-serial adapter surfaces as ENXIO, EIO or BrokenPipe depending on
        // platform and timing. None of those names the cause; enumeration does.
        for kind in [ErrorKind::Other, ErrorKind::BrokenPipe, ErrorKind::NotFound] {
            assert_eq!(
                classify_io_error(io::Error::new(kind, "boom"), false),
                TransportError::Disconnected,
                "{kind:?} on a port the OS no longer lists is a disconnect"
            );
        }
    }

    #[test]
    fn timeout_on_a_present_port_stays_a_timeout_but_on_a_gone_one_does_not() {
        // The completion poll treats Timeout as "keep polling" until its deadline. A device
        // that has been unplugged would burn that whole deadline before failing.
        assert_eq!(
            classify_io_error(io::Error::new(ErrorKind::TimedOut, "timed out"), true),
            TransportError::Timeout
        );
        assert_eq!(
            classify_io_error(io::Error::new(ErrorKind::TimedOut, "timed out"), false),
            TransportError::Disconnected
        );
    }

    #[test]
    fn present_port_keeps_the_os_message() {
        match classify_io_error(io::Error::new(ErrorKind::PermissionDenied, "access denied"), true) {
            TransportError::Io(msg) => assert!(msg.contains("access denied"), "got: {msg}"),
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn usable_ports_drops_system_ports_and_dialin_duplicates() {
        // Real enumeration from a macOS dev machine, plus a USB-serial adapter.
        let observed = [
            "/dev/cu.debug-console",
            "/dev/tty.debug-console",
            "/dev/cu.wlan-debug",
            "/dev/tty.wlan-debug",
            "/dev/cu.Bluetooth-Incoming-Port",
            "/dev/tty.Bluetooth-Incoming-Port",
            "/dev/cu.usbserial-1420",
            "/dev/tty.usbserial-1420",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(usable_ports(observed), vec!["/dev/cu.usbserial-1420"]);
    }

    #[test]
    fn usable_ports_keeps_linux_names_and_unpaired_dialin_nodes() {
        let names = ["/dev/ttyUSB0", "/dev/ttyACM0", "/dev/tty.orphan"].map(String::from).to_vec();
        // No dot after "tty" in the Linux names, and no callout twin for the orphan, so all survive.
        assert_eq!(usable_ports(names.clone()), names);
    }

    #[test]
    fn open_nonexistent_port_reports_io_error() {
        match SerialTransport::open("/dev/does-not-exist-xyz", 9600) {
            Err(driver_core::TransportError::Io(_)) => {}
            Err(other) => panic!("{other:?}"),
            Ok(_) => panic!("unexpectedly opened nonexistent port"),
        }
    }

    fn usb_port(path: &str, serial: Option<&str>) -> serialport::SerialPortInfo {
        serialport::SerialPortInfo {
            port_name: path.to_string(),
            port_type: serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
                vid: 0x1a86,
                pid: 0x7523,
                serial_number: serial.map(str::to_string),
                manufacturer: None,
                product: None,
                #[cfg(feature = "usbportinfo-interface")]
                interface: None,
            }),
        }
    }

    /// The bug this guards: two identical adapters that swap `/dev/ttyUSB*` numbers across a
    /// reboot. Named by path, each would answer to the other's id, and because both are Pumas
    /// the machine-id check downstream cannot tell them apart — the cut goes to the wrong one.
    #[test]
    fn an_adapters_serial_number_survives_a_path_change() {
        let before = PortId { path: "/dev/ttyUSB0".into(), serial_number: Some("FT-A".into()) };
        let after = PortId { path: "/dev/ttyUSB1".into(), serial_number: Some("FT-A".into()) };
        assert_eq!(before.serial_number, after.serial_number, "same adapter, new node");
        assert!(before.is_stable());

        let other = PortId { path: "/dev/ttyUSB0".into(), serial_number: Some("FT-B".into()) };
        assert_ne!(before.serial_number, other.serial_number, "same node, different adapter");
    }

    #[test]
    fn a_port_reports_whether_its_name_can_be_trusted() {
        assert!(PortId { path: "/dev/ttyUSB0".into(), serial_number: Some("FT-A".into()) }.is_stable());
        assert!(!PortId { path: "/dev/ttyUSB0".into(), serial_number: None }.is_stable());
    }

    #[test]
    fn a_serial_number_is_read_from_a_usb_port_and_trimmed() {
        assert_eq!(port_serial_number(&usb_port("/dev/ttyUSB0", Some("FT-A"))), Some("FT-A".into()));
        assert_eq!(port_serial_number(&usb_port("/dev/ttyUSB0", Some("  FT-A  "))), Some("FT-A".into()));
    }

    /// A descriptor the adapter left blank is not an identity, and neither is a non-USB port.
    /// Both must fall back to the path rather than colliding on an empty id.
    #[test]
    fn a_blank_or_non_usb_port_reports_no_serial_number() {
        assert_eq!(port_serial_number(&usb_port("/dev/ttyUSB0", None)), None);
        assert_eq!(port_serial_number(&usb_port("/dev/ttyUSB0", Some(""))), None);
        assert_eq!(port_serial_number(&usb_port("/dev/ttyUSB0", Some("   "))), None);

        let native = serialport::SerialPortInfo {
            port_name: "/dev/ttyS0".to_string(),
            port_type: serialport::SerialPortType::PciPort,
        };
        assert_eq!(port_serial_number(&native), None);
    }
}
