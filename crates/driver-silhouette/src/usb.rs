// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Transport, TransportError};
use nusb::transfer::RequestBuffer;
use std::time::Duration;

const VID: u16 = 0x3844;
const PIDS: [u16; 2] = [0x0001, 0x0002]; // ponytail: Cameo 5 Alpha and Alpha Plus
const EP_OUT: u8 = 0x01;
const EP_IN: u8 = 0x82;

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
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError> {
        let req_buf = RequestBuffer::new(buf.len());
        let xfer = self.iface.bulk_in(EP_IN, req_buf);

        // ponytail: nusb bulk_in has no timeout; spawn thread + channel to enforce it.
        // On a genuine timeout (device hung but enumerated) the thread stays blocked in
        // block_on forever and leaks — one thread + one live transfer handle per timed-out
        // read. Acceptable for low-frequency status polling; upgrade path is nusb's Queue
        // interface with real cancellation if tight-loop reads ever need this.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let completion = futures_lite::future::block_on(xfer);
            let _ = tx.send(completion);
        });

        match rx.recv_timeout(timeout) {
            Ok(completion) => {
                completion.status.map_err(|e| TransportError::Io(format!("{e:?}")))?;
                let data = completion.data;
                let n = data.len();
                buf[..n].copy_from_slice(&data);
                Ok(n)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(TransportError::Timeout),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err(TransportError::Io("transfer thread panicked".to_string()))
            }
        }
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
