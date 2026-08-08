// SPDX-License-Identifier: GPL-3.0-or-later
//! What a client and a Cut Host say to each other.
//!
//! Every type here is either `driver-core`'s own or a thin wrapper on one. That is
//! deliberate: the desktop's local path and its remote path must not grow two
//! vocabularies for the same cut, so the protocol adds a device id and an envelope
//! and invents nothing else.

use driver_core::manager::{CutPass, DeviceError, DeviceEvent};
use driver_core::{CutStatus, DeviceInfo};
use serde::{Deserialize, Serialize};

/// A client's own name for one dispatch, so a retry after a dropped reply cannot
/// cut the same material twice. The host, not the client, decides what it means:
/// an id it has already seen starts nothing.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DispatchId(pub String);

/// Everything a reattaching client needs about one cutter in a single value.
/// `job_id` rides alongside `CutStatus` because the status alone cannot say
/// *whose* finished Job it is describing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceSnapshot {
    pub info: DeviceInfo,
    pub status: CutStatus,
    pub job_id: Option<u64>,
}

/// `device` is always a `DeviceInfo::instance_id`.
///
/// There is deliberately no `Connect`/`Disconnect`: the host connects each cutter
/// it enumerates and holds it, so two clients cannot race over one cutter's
/// connection state and a client that dies mid-Job cannot orphan a Transport.
#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    ListDevices,
    Snapshot,
    Dispatch {
        dispatch_id: DispatchId,
        device: String,
        machine_id: String,
        passes: Vec<CutPass>,
    },
    Cancel { device: String },
    Resume { device: String },
    ConfirmPassDone { device: String },
}

/// `Accepted` carries no `job_id`: `DeviceManager::cut` does not return one until
/// the Job reaches its first pause point or finishes (`manager.rs:648-668`), which
/// for a pollable machine is the end of the cut. The client reads `job_id` off the
/// event stream instead.
#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Devices(Vec<DeviceInfo>),
    Snapshots(Vec<DeviceSnapshot>),
    Accepted { dispatch_id: DispatchId },
    Ok,
    Refused(Refusal),
}

/// `Preflight` carries the sentence rather than a code: the rule that refused owns
/// its own words, and a second copy of them on the client is the drift PR #90
/// removed.
#[derive(Debug, Serialize, Deserialize)]
pub enum Refusal {
    UnknownDevice(String),
    MachineMismatch { dispatched: String, attached: String },
    Preflight(String),
    Device(DeviceError),
}

/// `driver-core`'s own event, plus which cutter it came from. One client
/// connection carries every cutter on a host, so the id is what separates them.
#[derive(Debug, Serialize, Deserialize)]
pub struct Event {
    pub device: String,
    pub event: DeviceEvent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use driver_core::manager::{CutPass, DeviceEvent, DeviceEventKind};
    use driver_core::{CutStatus, DeviceInfo, Job, Settings, TransportKind};
    use geometry::Point;

    fn a_device() -> DeviceInfo {
        DeviceInfo {
            instance_id: "usb:1:4".into(),
            machine_id: "cameo5".into(),
            transport: TransportKind::Usb { locator: "1:4".into() },
            candidate: false,
        }
    }

    fn a_pass() -> CutPass {
        CutPass {
            job: Job {
                polylines: vec![vec![Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 }]],
                settings: Settings { speed: Some(5), force: Some(10), repeat_count: 1 },
            },
        }
    }

    /// A round trip that loses a field is the failure this guards: the host would
    /// cut geometry the client did not send, or drop Settings and cut at defaults.
    #[test]
    fn a_dispatch_survives_the_wire_unchanged() {
        let sent = Request::Dispatch {
            dispatch_id: DispatchId("d-1".into()),
            device: "usb:1:4".into(),
            machine_id: "cameo5".into(),
            passes: vec![a_pass()],
        };
        let json = serde_json::to_string(&sent).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        match back {
            Request::Dispatch { dispatch_id, device, machine_id, passes } => {
                assert_eq!(dispatch_id.0, "d-1");
                assert_eq!(device, "usb:1:4");
                assert_eq!(machine_id, "cameo5");
                assert_eq!(passes.len(), 1);
                assert_eq!(passes[0].job, a_pass().job);
            }
            other => panic!("round trip changed the variant: {other:?}"),
        }
    }

    #[test]
    fn every_other_request_variant_round_trips() {
        for sent in [
            Request::ListDevices,
            Request::Snapshot,
            Request::Cancel { device: "usb:1:4".into() },
            Request::Resume { device: "usb:1:4".into() },
            Request::ConfirmPassDone { device: "usb:1:4".into() },
        ] {
            let json = serde_json::to_string(&sent).unwrap();
            let back: Request = serde_json::from_str(&json).unwrap();
            assert_eq!(format!("{back:?}"), format!("{sent:?}"));
        }
    }

    /// `CutStatus` is what a reattaching client renders from, so it is the one type
    /// whose round trip has to carry every field — phase, ending, actions, Pass
    /// position and byte progress together.
    #[test]
    fn a_snapshot_carries_the_whole_status() {
        let sent = Response::Snapshots(vec![DeviceSnapshot {
            info: a_device(),
            status: CutStatus::disconnected(),
            job_id: Some(7),
        }]);
        let json = serde_json::to_string(&sent).unwrap();
        let back: Response = serde_json::from_str(&json).unwrap();
        match back {
            Response::Snapshots(s) => {
                assert_eq!(s[0].info, a_device());
                assert_eq!(s[0].status, CutStatus::disconnected());
                assert_eq!(s[0].job_id, Some(7));
            }
            other => panic!("round trip changed the variant: {other:?}"),
        }
    }

    #[test]
    fn an_event_keeps_its_device_and_its_job() {
        let sent = Event {
            device: "usb:1:4".into(),
            event: DeviceEvent {
                job_id: 3,
                kind: DeviceEventKind::Progress { pass_index: 1, submitted_bytes: 40, total_bytes: 100 },
                status: CutStatus::disconnected(),
            },
        };
        let json = serde_json::to_string(&sent).unwrap();
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back.device, "usb:1:4");
        assert_eq!(back.event.job_id, 3);
        assert!(matches!(
            back.event.kind,
            DeviceEventKind::Progress { pass_index: 1, submitted_bytes: 40, total_bytes: 100 }
        ));
    }
}
