// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Transport, TransportError};
use std::io::Read;
use std::time::Duration;

/// Names of all serial ports currently visible to the OS (not narrowed to Puma devices —
/// serial has no VID/PID-equivalent discriminator here, so the caller must ask the operator).
pub fn list_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(|p| p.port_name).collect())
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
    fn open_nonexistent_port_reports_io_error() {
        match SerialTransport::open("/dev/does-not-exist-xyz", 9600) {
            Err(driver_core::TransportError::Io(_)) => {}
            Err(other) => panic!("{other:?}"),
            Ok(_) => panic!("unexpectedly opened nonexistent port"),
        }
    }
}
