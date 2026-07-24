// SPDX-License-Identifier: GPL-3.0-or-later
use geometry::Polyline;
use std::collections::VecDeque;
use std::time::Duration;

pub mod manager;

pub struct Job { pub polylines: Vec<Polyline>, pub settings: Settings }

#[derive(Clone, Debug, PartialEq)]
pub struct Settings { pub speed: Option<u32>, pub force: Option<u32>, pub repeat_count: u32 }
impl Default for Settings { fn default() -> Self { Settings { speed: None, force: None, repeat_count: 1 } } }

#[derive(Clone, Debug, PartialEq)]
pub struct MachineProfile { pub id: String, pub name: String, pub width_mm: f64, pub height_mm: f64 }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MachineCaps { pub supports_speed: bool, pub supports_force: bool, pub needs_operator_pass_confirm: bool }

#[derive(Debug, PartialEq)]
pub enum DriverError { UnsupportedGeometry, Encode(String) }
#[derive(Debug, PartialEq)]
pub enum TransportError { NotFound, Timeout, WriteZero, Io(String) }

pub trait Driver {
    fn profile(&self) -> &MachineProfile;
    fn caps(&self) -> MachineCaps;
    fn session_begin(&self) -> Vec<u8>;
    fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError>;
    fn pass_park(&self) -> Vec<u8>;
    fn session_end(&self) -> Vec<u8>;
    fn abort_bytes(&self) -> Option<Vec<u8>>;
}
pub trait Transport: Send {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError>;
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError>;
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransportKind {
    Usb { locator: String }, // "bus:address"
    Serial { path: String, baud: u32 },
}
#[derive(Clone, Debug, PartialEq)]
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
}
