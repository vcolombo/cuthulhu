// SPDX-License-Identifier: GPL-3.0-or-later

//! The Cut Hosts this desktop has paired with.
//!
//! Holds a token per host, so the file is `0600` and written atomically. Losing it means
//! re-pairing every Pi by hand, which is the single most-reported complaint about the
//! comparable feature in Bambu Studio — persistence being dull and correct is the point.

use std::path::{Path, PathBuf};

use driver_core::{DeviceInfo, HostId};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedHost {
    pub id: HostId,
    /// What the operator calls it. Free to change without invalidating anything, which is why
    /// the id is not derived from it.
    pub name: String,
    /// `host:port`. Also free to change — a Pi can get a new lease.
    pub address: String,
    /// The certificate fingerprint accepted at pairing. A change is a refusal, never a prompt.
    pub fingerprint: String,
    pub token: String,
}

/// Hand-written rather than derived, mirroring `cut_host::config::Config`: `token` is the secret
/// that authorizes this desktop to make a blade move, and a derived `Debug` would print it
/// verbatim into whatever log or panic message formatted a `PairedHost`.
impl std::fmt::Debug for PairedHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairedHost")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("address", &self.address)
            .field("fingerprint", &self.fingerprint)
            .field("token", &"<redacted>")
            .finish()
    }
}

#[derive(Debug)]
pub enum HostsError {
    Unreadable(String),
    Malformed(String),
    Unwritable(String),
}

impl std::fmt::Display for HostsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostsError::Unreadable(m) => write!(f, "the paired hosts could not be read ({m})"),
            HostsError::Malformed(m) => write!(f, "the paired hosts file could not be understood ({m})"),
            HostsError::Unwritable(m) => write!(f, "the paired hosts could not be saved ({m})"),
        }
    }
}
impl std::error::Error for HostsError {}

/// Every paired host, or an empty list when none has been.
///
/// A missing file is an empty list; a corrupt one is an error. The distinction matters: read as
/// empty, a corrupt file looks exactly like a fresh install, and the operator would pair over the
/// top of hosts they still have.
pub fn load(path: &Path) -> Result<Vec<PairedHost>, HostsError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(HostsError::Unreadable(e.to_string())),
    };
    serde_json::from_str(&text).map_err(|e| HostsError::Malformed(e.to_string()))
}

/// Write atomically, so an interrupted save leaves the previous list intact rather than a
/// truncated one. The temp file goes in the destination directory because a rename across
/// filesystems is not atomic.
pub fn save(path: &Path, hosts: &[PairedHost]) -> Result<(), HostsError> {
    let dir = path.parent().ok_or_else(|| HostsError::Unwritable("no parent directory".into()))?;
    std::fs::create_dir_all(dir).map_err(|e| HostsError::Unwritable(e.to_string()))?;

    let json = serde_json::to_string_pretty(hosts).map_err(|e| HostsError::Unwritable(e.to_string()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| HostsError::Unwritable(e.to_string()))?;
    std::io::Write::write_all(&mut tmp, json.as_bytes())
        .map_err(|e| HostsError::Unwritable(e.to_string()))?;

    // Set on the temp file before the rename, so the tokens are never briefly world-readable
    // at the destination — and re-applied on every save, since a rewrite would otherwise
    // inherit whatever the temp file's default mode happened to be.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o600))
            .map_err(|e| HostsError::Unwritable(e.to_string()))?;
    }

    tmp.persist(path).map_err(|e| HostsError::Unwritable(e.to_string()))?;
    Ok(())
}

/// Beside `presets.json`, which is where this application already keeps user state.
pub fn default_hosts_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("cuthulhu").join("hosts.json"))
}

/// What startup calls: never lets a missing config directory, or a missing, unreadable, or
/// corrupt `hosts.json`, keep the app from launching on its local cutter — pairing a Pi is meant
/// to be optional, so a broken save of that preference must not become a reason to refuse to
/// start. `on_error` is how the operator is still told, since silently discarding a corrupt file
/// is how they'd end up re-pairing over the top of hosts they still have without knowing why.
pub fn load_or_warn(path: Option<&Path>, on_error: impl FnOnce(&HostsError)) -> Vec<PairedHost> {
    let Some(path) = path else { return Vec::new() };
    match load(path) {
        Ok(hosts) => hosts,
        Err(e) => {
            on_error(&e);
            Vec::new()
        }
    }
}

/// Mark every device as belonging to `id`.
///
/// A Cut Host does not know its own `HostId` — the desktop mints it at pairing and the daemon
/// has never heard of it — so everything a host returns arrives saying `host: None`, which is
/// the value that means "plugged into this computer". Whoever fetches must stamp, or every
/// remote cutter routes to the local Transport.
pub fn stamp_host(id: &HostId, devices: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
    devices
        .into_iter()
        .map(|d| DeviceInfo { host: Some(id.clone()), ..d })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use driver_core::{DeviceInfo, TransportKind};

    fn a_device(instance: &str) -> DeviceInfo {
        DeviceInfo {
            instance_id: instance.into(),
            machine_id: "cameo5".into(),
            transport: TransportKind::Usb { locator: "sn:CAMEO-A".into() },
            candidate: false,
            host: None,
        }
    }

    /// A Cut Host does not know its own id, so everything it returns says `host: None` — which
    /// is the value that means "plugged into this computer". Unstamped, every remote cutter
    /// would be routed to the local DeviceManager.
    #[test]
    fn fetched_devices_are_stamped_with_the_host_they_came_from() {
        let id = HostId("host-1".into());
        let stamped = stamp_host(&id, vec![a_device("usb:sn:A"), a_device("serial:sn:B")]);

        assert_eq!(stamped.len(), 2);
        assert!(stamped.iter().all(|d| d.host.as_ref() == Some(&id)), "{stamped:?}");
        assert_eq!(stamped[0].instance_id, "usb:sn:A", "the cutter's own id is untouched");
    }

    #[test]
    fn stamping_nothing_yields_nothing() {
        assert!(stamp_host(&HostId("host-1".into()), Vec::new()).is_empty());
    }

    fn a_host(id: &str, name: &str) -> PairedHost {
        PairedHost {
            id: HostId(id.into()),
            name: name.into(),
            address: "cuthulhu-pi.local:7878".into(),
            fingerprint: "aa:bb:cc".into(),
            token: "s3cret".into(),
        }
    }

    /// A stray `{:?}` in a log or panic message must not leak the token, mirroring
    /// `cut_host::config::Config`'s hand-written `Debug`.
    #[test]
    fn debug_never_prints_the_token() {
        let debugged = format!("{:?}", a_host("host-1", "Workshop Pi"));
        assert!(!debugged.contains("s3cret"), "{debugged}");
        assert!(debugged.contains("<redacted>"), "{debugged}");
    }

    #[test]
    fn hosts_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        let written = vec![a_host("host-1", "Workshop Pi"), a_host("host-2", "Spare")];

        save(&path, &written).unwrap();
        assert_eq!(load(&path).unwrap(), written);
    }

    /// A user who has never paired anything is not an error state.
    #[test]
    fn a_missing_file_is_no_hosts_rather_than_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&dir.path().join("nothing-here.json")).unwrap(), Vec::new());
    }

    /// The file holds every client's token, so it must not be world-readable. Checked because a
    /// mode set at creation is easy to lose to a later rewrite.
    #[cfg(unix)]
    #[test]
    fn the_file_is_not_readable_by_anyone_else() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        save(&path, &[a_host("host-1", "Workshop Pi")]).unwrap();
        save(&path, &[a_host("host-1", "Renamed")]).unwrap(); // a rewrite must not widen it

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "hosts.json holds tokens: {mode:o}");
    }

    /// A corrupt file must not be silently read as "no hosts paired" — that would look exactly
    /// like a fresh install and invite the user to re-pair over the top of it.
    #[test]
    fn a_corrupt_file_is_an_error_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(load(&path), Err(HostsError::Malformed(_))));
    }

    /// Pairing a Pi is optional. A corrupt `hosts.json` must not be a reason the app refuses to
    /// start on its local cutter — it must be reported and treated as no hosts, not propagated.
    #[test]
    fn a_corrupt_file_yields_no_hosts_instead_of_blocking_startup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        std::fs::write(&path, "{ not json").unwrap();

        let mut warned = false;
        let hosts = load_or_warn(Some(&path), |_| warned = true);
        assert!(hosts.is_empty(), "a corrupt file must not surface as paired hosts");
        assert!(warned, "the operator should still be told, just not blocked by it");
    }

    /// A system with no config directory at all is the same story: no hosts, no crash.
    #[test]
    fn no_config_directory_yields_no_hosts() {
        let hosts = load_or_warn(None, |_| panic!("nothing to warn about"));
        assert!(hosts.is_empty());
    }

    /// A file that exists but cannot be read as text (here, it is actually a directory) must not
    /// be confused with a missing one — `NotFound` means "no hosts yet", anything else is real.
    #[test]
    fn an_unreadable_path_is_an_error_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        std::fs::create_dir(&path).unwrap(); // reading a directory as a file fails non-NotFound
        assert!(matches!(load(&path), Err(HostsError::Unreadable(_))));
    }
}
