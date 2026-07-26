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

/// Names of serial ports that could plausibly be a cutter (not narrowed to Puma devices —
/// serial has no VID/PID-equivalent discriminator here, so the caller must ask the operator).
pub fn list_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| usable_ports(ports.into_iter().map(|p| p.port_name).collect()))
        .unwrap_or_default()
}

pub struct SerialTransport { port: Box<dyn serialport::SerialPort> }
impl SerialTransport {
    pub fn open(port: &str, baud: u32) -> Result<SerialTransport, TransportError> {
        let p = serialport::new(port, baud)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .timeout(Duration::from_secs(5))
            .open().map_err(|e| TransportError::Io(e.to_string()))?;
        Ok(SerialTransport { port: p })
    }
}
impl Transport for SerialTransport {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
        use std::io::Write;
        self.port.write_all(bytes).map_err(|e| TransportError::Io(e.to_string()))?;
        Ok(bytes.len())
    }
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError> {
        self.port.set_timeout(timeout).map_err(|e| TransportError::Io(e.to_string()))?;
        match self.port.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Err(TransportError::Timeout),
            Err(e) => Err(TransportError::Io(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
