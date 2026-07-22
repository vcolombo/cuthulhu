// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Transport, TransportError};
use std::time::Duration;

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
