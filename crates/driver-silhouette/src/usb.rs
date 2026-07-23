// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Transport, TransportError};

const VID: u16 = 0x3844;
const PIDS: [u16; 2] = [0x0001, 0x0002]; // ponytail: Cameo 5 Alpha and Alpha Plus
const EP_OUT: u8 = 0x01;

pub struct UsbTransport {
    iface: nusb::Interface,
}

impl UsbTransport {
    pub fn open() -> Result<UsbTransport, TransportError> {
        let di = nusb::list_devices()
            .map_err(|e| TransportError::Io(e.to_string()))?
            .find(|d| d.vendor_id() == VID && PIDS.contains(&d.product_id()))
            .ok_or(TransportError::NotFound)?;
        let dev = di.open().map_err(|e| TransportError::Io(e.to_string()))?;
        let iface = dev
            .claim_interface(0)
            .map_err(|e| TransportError::Io(e.to_string()))?;
        Ok(UsbTransport { iface })
    }
}

impl Transport for UsbTransport {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
        let xfer = self.iface.bulk_out(EP_OUT, bytes.to_vec());
        let completion = futures_lite::future::block_on(xfer);
        completion.status.map_err(|e| TransportError::Io(format!("{e:?}")))?;
        Ok(bytes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn open_without_device_reports_not_found() {
        // CI has no Cameo attached → must be a typed NotFound, never a panic.
        match UsbTransport::open() {
            Err(TransportError::NotFound) => {}
            Err(e) => panic!("expected NotFound, got: {e:?}"),
            Ok(_) => panic!("device unexpectedly found"),
        }
    }
}
