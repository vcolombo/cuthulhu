// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Transport, TransportError};
use nusb::transfer::{RequestBuffer, TransferError};
use std::time::Duration;

const VID: u16 = 0x3844;
const PIDS: [u16; 2] = [0x0001, 0x0002]; // ponytail: Cameo 5 Alpha and Alpha Plus
const EP_OUT: u8 = 0x01;
const EP_IN: u8 = 0x82;

pub struct UsbTransport {
    iface: nusb::Interface,
    locator: String,
}

/// Maps a nusb transfer failure to a transport error, using `still_enumerated` (whether the
/// device is still listed by the OS) to tell a disconnect from a device-side fault.
///
/// nusb reports an unplug as `Disconnected` on some platforms but as `Unknown` on macOS, so
/// the error code alone cannot answer "is the cable still in?" — enumeration can.
fn classify_transfer_error(e: TransferError, still_enumerated: bool) -> TransportError {
    match e {
        TransferError::Disconnected => TransportError::Disconnected,
        _ if !still_enumerated => TransportError::Disconnected,
        other => TransportError::Io(other.to_string()),
    }
}

/// How a Cameo is named, and what that name survives.
///
/// A device's serial number is the machine itself talking, so a locator built from one still
/// names the same Cameo after a reboot, a replug, or a hub enumerating in a different order.
/// Bus and address survive none of those: they are where the OS happened to find it this time.
/// Two Cameos on one host that swapped addresses would both answer to the wrong name, and
/// nothing downstream could tell — the machine ids match, so the cut would simply go to the
/// other one.
///
/// The bus form remains for devices that report no serial number, where there is nothing
/// better to say. `Locator::is_stable` is what a caller asks before trusting a saved name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Locator {
    /// The device's own serial number.
    Serial(String),
    /// Where it was found, for a device that reports no serial number.
    BusAddress(u8, u8),
}

impl Locator {
    /// Whether this name still means the same machine after the OS re-enumerates.
    pub fn is_stable(&self) -> bool {
        matches!(self, Locator::Serial(_))
    }
}

impl std::fmt::Display for Locator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Locator::Serial(sn) => write!(f, "sn:{sn}"),
            Locator::BusAddress(bus, addr) => write!(f, "{bus}:{addr}"),
        }
    }
}

impl std::str::FromStr for Locator {
    type Err = ();
    fn from_str(s: &str) -> Result<Locator, ()> {
        if let Some(sn) = s.strip_prefix("sn:") {
            return if sn.is_empty() { Err(()) } else { Ok(Locator::Serial(sn.to_string())) };
        }
        let (bus, addr) = s.split_once(':').ok_or(())?;
        Ok(Locator::BusAddress(bus.parse().map_err(|_| ())?, addr.parse().map_err(|_| ())?))
    }
}

/// The locator for one enumerated device: its serial number when it reports one, else where it
/// was found. Split out from `list_locators` so the choice can be tested without hardware.
fn locator_for(serial: Option<&str>, bus: u8, addr: u8) -> Locator {
    match serial {
        // An empty serial number is a descriptor the device did not fill in, not an identity.
        Some(sn) if !sn.trim().is_empty() => Locator::Serial(sn.trim().to_string()),
        _ => Locator::BusAddress(bus, addr),
    }
}

/// Locators for every enumerated Cameo device, in enumeration order.
pub fn list_locators() -> Vec<String> {
    let Ok(devices) = nusb::list_devices() else { return Vec::new() };
    devices
        .filter(|d| d.vendor_id() == VID && PIDS.contains(&d.product_id()))
        .map(|d| locator_for(d.serial_number(), d.bus_number(), d.device_address()).to_string())
        .collect()
}

fn parse_locator(locator: &str) -> Option<Locator> {
    locator.parse().ok()
}

impl UsbTransport {
    /// Opens the first enumerated Cameo device. Kept for CLI back-compat; prefer `open_at`.
    pub fn open() -> Result<UsbTransport, TransportError> {
        let locator = list_locators().into_iter().next().ok_or(TransportError::NotFound)?;
        Self::open_at(&locator)
    }

    /// Opens the Cameo device the locator names (from `list_locators`) — by serial number where
    /// the device reports one, otherwise by the bus address it was last found at.
    ///
    /// Resolving a serial number against a fresh enumeration is the point: the device may be at
    /// a different address than when the locator was taken, and it is still the same machine.
    pub fn open_at(locator: &str) -> Result<UsbTransport, TransportError> {
        let wanted = parse_locator(locator).ok_or(TransportError::NotFound)?;
        let di = nusb::list_devices()
            .map_err(|e| TransportError::Io(e.to_string()))?
            .find(|d| {
                d.vendor_id() == VID
                    && PIDS.contains(&d.product_id())
                    && locator_for(d.serial_number(), d.bus_number(), d.device_address()) == wanted
            })
            .ok_or(TransportError::NotFound)?;
        let dev = di.open().map_err(|e| TransportError::Io(e.to_string()))?;
        let iface = dev
            .claim_interface(0)
            .map_err(|e| TransportError::Io(e.to_string()))?;
        Ok(UsbTransport { iface, locator: locator.to_string() })
    }

    /// Whether this transport's device is still enumerated by the OS.
    fn still_enumerated(&self) -> bool {
        list_locators().contains(&self.locator)
    }
}

impl Transport for UsbTransport {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
        let xfer = self.iface.bulk_out(EP_OUT, bytes.to_vec());
        let completion = futures_lite::future::block_on(xfer);
        completion.status.map_err(|e| classify_transfer_error(e, self.still_enumerated()))?;
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
                completion.status.map_err(|e| classify_transfer_error(e, self.still_enumerated()))?;
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
        // Only meaningful with no Cameo attached (CI, and dev machines between hardware
        // runs). Skip rather than fail when one is plugged in — the assertion is about the
        // empty-enumeration path, not about the developer's desk.
        if !list_locators().is_empty() {
            eprintln!("skipped: a Cameo is attached");
            return;
        }
        match UsbTransport::open() {
            Err(TransportError::NotFound) => {}
            Err(e) => panic!("expected NotFound, got: {e:?}"),
            Ok(_) => panic!("device unexpectedly found"),
        }
    }

    #[test]
    fn gone_device_classifies_as_disconnected_whatever_the_transfer_error() {
        // macOS reports a mid-transfer unplug as Unknown, Linux as Disconnected; both mean
        // the cable is out, and the operator must be told that rather than "Unknown".
        assert_eq!(
            classify_transfer_error(TransferError::Disconnected, true),
            TransportError::Disconnected
        );
        assert_eq!(
            classify_transfer_error(TransferError::Unknown, false),
            TransportError::Disconnected
        );
    }

    #[test]
    fn present_device_keeps_a_readable_fault_message() {
        // Still enumerated → a genuine device-side fault, reported with nusb's Display text
        // rather than the bare Debug name.
        let err = classify_transfer_error(TransferError::Stall, true);
        match err {
            TransportError::Io(msg) => {
                assert!(!msg.is_empty(), "fault message must not be empty");
                assert_ne!(msg, "Stall", "expected Display text, not the Debug variant name");
            }
            other => panic!("expected Io, got: {other:?}"),
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

    /// The bug this guards: two Cameos that swap bus addresses across a reboot. Named by
    /// address, each would answer to the other's locator and the cut would go to the wrong
    /// machine — the machine ids match, so nothing downstream could refuse it.
    #[test]
    fn a_serial_number_outranks_where_the_device_was_found() {
        let before = locator_for(Some("CAMEO-A"), 1, 4);
        let after_a_reboot_at_a_different_address = locator_for(Some("CAMEO-A"), 2, 9);
        assert_eq!(before, after_a_reboot_at_a_different_address);
        assert!(before.is_stable());

        let other = locator_for(Some("CAMEO-B"), 1, 4);
        assert_ne!(before, other, "two Cameos at the same address are still two Cameos");
    }

    /// A device that reports no serial number has nothing better to say than where it is, and
    /// must say so — `is_stable` is what a caller asks before trusting a saved name.
    #[test]
    fn a_device_without_a_serial_number_falls_back_to_its_address() {
        let l = locator_for(None, 1, 4);
        assert_eq!(l, Locator::BusAddress(1, 4));
        assert!(!l.is_stable());

        // A descriptor the device left blank is not an identity.
        assert_eq!(locator_for(Some(""), 1, 4), Locator::BusAddress(1, 4));
        assert_eq!(locator_for(Some("   "), 1, 4), Locator::BusAddress(1, 4));
    }

    #[test]
    fn a_locator_round_trips_through_its_text_form() {
        for l in [Locator::Serial("CAMEO-A".into()), Locator::BusAddress(1, 4)] {
            assert_eq!(l.to_string().parse::<Locator>(), Ok(l.clone()), "{l:?}");
        }
        assert_eq!("sn:CAMEO-A".parse::<Locator>(), Ok(Locator::Serial("CAMEO-A".into())));
        assert_eq!("1:4".parse::<Locator>(), Ok(Locator::BusAddress(1, 4)));
    }

    /// A serial number containing a colon must not be read as a bus address, and an empty one
    /// must not become a device that answers to `sn:`.
    #[test]
    fn a_malformed_locator_is_refused_rather_than_guessed() {
        assert_eq!("sn:12:34".parse::<Locator>(), Ok(Locator::Serial("12:34".into())));
        assert!("sn:".parse::<Locator>().is_err());
        assert!("not-a-locator".parse::<Locator>().is_err());
        assert!("1:notanumber".parse::<Locator>().is_err());
    }
}
