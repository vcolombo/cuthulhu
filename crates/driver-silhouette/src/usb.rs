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

/// Locators ("bus:address") for every enumerated Cameo device, in enumeration order.
pub fn list_locators() -> Vec<String> {
    let Ok(devices) = nusb::list_devices() else { return Vec::new() };
    devices
        .filter(|d| d.vendor_id() == VID && PIDS.contains(&d.product_id()))
        .map(|d| format!("{}:{}", d.bus_number(), d.device_address()))
        .collect()
}

fn parse_locator(locator: &str) -> Option<(u8, u8)> {
    let (bus, addr) = locator.split_once(':')?;
    Some((bus.parse().ok()?, addr.parse().ok()?))
}

impl UsbTransport {
    /// Opens the first enumerated Cameo device. Kept for CLI back-compat; prefer `open_at`.
    pub fn open() -> Result<UsbTransport, TransportError> {
        let locator = list_locators().into_iter().next().ok_or(TransportError::NotFound)?;
        Self::open_at(&locator)
    }

    /// Opens the Cameo device at the given "bus:address" locator (from `list_locators`).
    pub fn open_at(locator: &str) -> Result<UsbTransport, TransportError> {
        let (bus, addr) = parse_locator(locator).ok_or(TransportError::NotFound)?;
        let di = nusb::list_devices()
            .map_err(|e| TransportError::Io(e.to_string()))?
            .find(|d| {
                d.vendor_id() == VID && PIDS.contains(&d.product_id())
                    && d.bus_number() == bus && d.device_address() == addr
            })
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
    #[test]
    fn open_at_unknown_locator_reports_not_found() {
        match UsbTransport::open_at("99:99") {
            Err(TransportError::NotFound) => {}
            Err(e) => panic!("expected NotFound, got: {e:?}"),
            Ok(_) => panic!("device unexpectedly found"),
        }
    }
    #[test]
    fn open_at_malformed_locator_reports_not_found() {
        match UsbTransport::open_at("not-a-locator") {
            Err(TransportError::NotFound) => {}
            Err(e) => panic!("expected NotFound, got: {e:?}"),
            Ok(_) => panic!("device unexpectedly found"),
        }
    }
}
