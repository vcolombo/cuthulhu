// SPDX-License-Identifier: GPL-3.0-or-later
use geometry::Polyline;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::Duration;

pub mod manager;
pub mod status;
pub use status::{Actions, ByteProgress, CutStatus, Ended, PassPosition, Phase};

#[derive(Clone, Debug, PartialEq)]
pub struct Job { pub polylines: Vec<Polyline>, pub settings: Settings }

#[derive(Clone, Debug, PartialEq)]
pub struct Settings { pub speed: Option<u32>, pub force: Option<u32>, pub repeat_count: u32 }
impl Default for Settings { fn default() -> Self { Settings { speed: None, force: None, repeat_count: 1 } } }

#[derive(Clone, Debug, PartialEq)]
pub struct MachineProfile { pub id: String, pub name: String, pub width_mm: f64, pub height_mm: f64 }

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineCaps { pub supports_speed: bool, pub supports_force: bool, pub needs_operator_pass_confirm: bool }

#[derive(Debug, PartialEq)]
pub enum DriverError { UnsupportedGeometry, Encode(String) }
#[derive(Debug, PartialEq)]
pub enum TransportError { NotFound, Disconnected, Timeout, WriteZero, Io(String) }

pub trait Driver {
    fn profile(&self) -> &MachineProfile;
    fn caps(&self) -> MachineCaps;
    fn session_begin(&self) -> Vec<u8>;
    fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError>;
    fn pass_park(&self) -> Vec<u8>;
    /// Bytes that query device status for completion polling; the device replies
    /// with a single status char (`0` ready / `1` moving / `2` unloaded) plus a
    /// terminator. Default is a bare ENQ; drivers whose dialect frames it
    /// differently (e.g. Silhouette's ESC-prefixed `1b 05`) override this.
    fn status_query(&self) -> Vec<u8> {
        vec![0x05]
    }
    fn session_end(&self) -> Vec<u8>;
    fn abort_bytes(&self) -> Option<Vec<u8>>;
}
pub trait Transport: Send {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError>;
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TransportKind {
    Usb { locator: String }, // "bus:address"
    Serial { path: String, baud: u32 },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub instance_id: String,
    pub machine_id: String,
    pub transport: TransportKind,
    pub candidate: bool,
}

pub trait DeviceBackendFactory: Send + Sync {
    fn list_devices(&self) -> Vec<DeviceInfo>;
    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>>;
    fn open_transport(&self, info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError>;
}

pub fn write_all(t: &mut dyn Transport, mut bytes: &[u8]) -> Result<(), TransportError> {
    while !bytes.is_empty() {
        match t.write(bytes)? {
            0 => return Err(TransportError::WriteZero),
            n => bytes = &bytes[n..],
        }
    }
    Ok(())
}

/// The bytes that open Pass `index`: the session prologue on the first Pass, then
/// the encoded Pass itself.
///
/// `DeviceManager` writes these, waits for the machine, then writes `close_pass`.
/// The two together are one Pass on the wire, so a caller that wants the whole Pass
/// at once — `cuthulhu cut --dry-run` — concatenates them rather than restating when
/// a prologue is owed.
pub fn open_pass(d: &dyn Driver, job: &Job, index: usize) -> Result<Vec<u8>, DriverError> {
    let mut bytes = if index == 0 { d.session_begin() } else { Vec::new() };
    bytes.extend(d.encode_pass(job)?);
    Ok(bytes)
}

/// The bytes that close Pass `index` of `total`: park between Passes, end the
/// session after the last one.
pub fn close_pass(d: &dyn Driver, index: usize, total: usize) -> Vec<u8> {
    if index + 1 < total { d.pass_park() } else { d.session_end() }
}

#[derive(Default)]
pub struct MockTransport {
    pub written: Vec<u8>,
    pub reads: VecDeque<Result<Vec<u8>, TransportError>>,
    pub write_results: VecDeque<Result<usize, TransportError>>,
}
impl Transport for MockTransport {
    fn write(&mut self, b: &[u8]) -> Result<usize, TransportError> {
        match self.write_results.pop_front() {
            Some(result) => match result {
                Ok(n) => {
                    let clamped = n.min(b.len());
                    self.written.extend_from_slice(&b[..clamped]);
                    Ok(clamped)
                }
                Err(e) => Err(e),
            },
            None => {
                self.written.extend_from_slice(b);
                Ok(b.len())
            }
        }
    }
    fn read(&mut self, buf: &mut [u8], _timeout: Duration) -> Result<usize, TransportError> {
        match self.reads.pop_front() {
            Some(result) => match result {
                Ok(data) => {
                    let n = data.len().min(buf.len());
                    buf[..n].copy_from_slice(&data[..n]);
                    Ok(n)
                }
                Err(e) => Err(e),
            },
            None => Err(TransportError::Timeout),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mock_transport_records_all_bytes() {
        let mut t = MockTransport::default();
        t.write(b"AB").unwrap();
        t.write(b"C").unwrap();
        assert_eq!(t.written, b"ABC");
    }
    #[test]
    fn default_settings_leave_speed_force_unset() {
        let s = Settings::default();
        assert!(s.speed.is_none() && s.force.is_none() && s.repeat_count == 1);
    }
    #[test]
    fn write_all_loops_partial_writes_and_flags_zero() {
        let mut t = MockTransport::default();
        t.write_results.push_back(Ok(2)); // partial: only 2 of 5 accepted
        write_all(&mut t, b"HELLO").unwrap();
        assert_eq!(t.written, b"HELLO");

        let mut z = MockTransport::default();
        z.write_results.push_back(Ok(0));
        assert_eq!(write_all(&mut z, b"X"), Err(TransportError::WriteZero));
    }
    #[test]
    fn mock_read_replays_script_then_times_out() {
        let mut t = MockTransport::default();
        t.reads.push_back(Ok(b"ready".to_vec()));
        let mut buf = [0u8; 8];
        let n = t.read(&mut buf, Duration::from_millis(10)).unwrap();
        assert_eq!(&buf[..n], b"ready");
        assert_eq!(t.read(&mut buf, Duration::from_millis(10)), Err(TransportError::Timeout));
    }
    #[test]
    fn mock_write_clamps_scripted_count_to_buffer_length() {
        let mut t = MockTransport::default();
        t.write_results.push_back(Ok(6)); // script says 6 bytes
        let result = t.write(b"HELLO").unwrap(); // but buffer is only 5
        assert_eq!(result, 5); // should return 5, not 6
        assert_eq!(t.written, b"HELLO"); // and only append 5 bytes
    }

    struct FakeFactory;
    impl DeviceBackendFactory for FakeFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> {
            vec![
                DeviceInfo {
                    instance_id: "usb:1:4".into(),
                    machine_id: "cameo5".into(),
                    transport: TransportKind::Usb { locator: "1:4".into() },
                    candidate: false,
                },
                DeviceInfo {
                    instance_id: "serial:/dev/ttyUSB0".into(),
                    machine_id: "puma".into(),
                    transport: TransportKind::Serial { path: "/dev/ttyUSB0".into(), baud: 9600 },
                    candidate: true,
                },
            ]
        }
        fn driver_for(&self, _: &str) -> Option<Box<dyn Driver + Send>> { None }
        fn open_transport(&self, _: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            Err(TransportError::NotFound)
        }
    }
    #[test]
    fn serial_devices_are_candidates_requiring_user_selection() {
        let f = FakeFactory;
        let serial: Vec<_> = f.list_devices().into_iter()
            .filter(|d| matches!(d.transport, TransportKind::Serial { .. })).collect();
        assert!(serial.iter().all(|d| d.candidate), "serial ports can't be assumed to be Pumas");
    }

    /// Distinguishable constants for the three framing methods, so a test can say
    /// which one landed and in what order. `profile`/`caps` diverge: framing reads
    /// neither, and a test that starts needing them is testing something else.
    struct FramingDriver;
    impl Driver for FramingDriver {
        fn profile(&self) -> &MachineProfile { unreachable!("framing does not read the profile") }
        fn caps(&self) -> MachineCaps { unreachable!("framing does not read the caps") }
        fn session_begin(&self) -> Vec<u8> { b"BEGIN".to_vec() }
        fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError> {
            Ok(format!("PASS{}", pass.polylines.len()).into_bytes())
        }
        fn pass_park(&self) -> Vec<u8> { b"PARK".to_vec() }
        fn session_end(&self) -> Vec<u8> { b"END".to_vec() }
        fn abort_bytes(&self) -> Option<Vec<u8>> { None }
    }

    #[test]
    fn only_the_first_pass_carries_the_session_prologue() {
        let job = Job { polylines: Vec::new(), settings: Settings::default() };
        assert_eq!(open_pass(&FramingDriver, &job, 0).unwrap(), b"BEGINPASS0".to_vec());
        assert_eq!(open_pass(&FramingDriver, &job, 1).unwrap(), b"PASS0".to_vec());
    }

    #[test]
    fn a_pass_parks_unless_it_is_the_last_one() {
        assert_eq!(close_pass(&FramingDriver, 0, 2), b"PARK".to_vec(), "another Pass follows, so park");
        assert_eq!(close_pass(&FramingDriver, 1, 2), b"END".to_vec(), "the last Pass closes the session");
        // The boundary a caller gets wrong: a one-Pass job's only Pass is also its last,
        // so it must close rather than park.
        assert_eq!(close_pass(&FramingDriver, 0, 1), b"END".to_vec());
    }
}
