// SPDX-License-Identifier: GPL-3.0-or-later
use geometry::Polyline;

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
pub enum TransportError { NotFound, Io(String) }

pub trait Driver {
    fn profile(&self) -> &MachineProfile;
    fn caps(&self) -> MachineCaps;
    fn session_begin(&self) -> Vec<u8>;
    fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError>;
    fn pass_park(&self) -> Vec<u8>;
    fn session_end(&self) -> Vec<u8>;
    fn abort_bytes(&self) -> Option<Vec<u8>>;
}
pub trait Transport {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError>;
}

#[derive(Default)]
pub struct MockTransport { pub written: Vec<u8> }
impl Transport for MockTransport {
    fn write(&mut self, b: &[u8]) -> Result<usize, TransportError> {
        self.written.extend_from_slice(b); Ok(b.len())
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
}
