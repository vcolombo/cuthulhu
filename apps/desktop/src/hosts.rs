// SPDX-License-Identifier: GPL-3.0-or-later

//! The Cut Hosts this desktop has paired with.
//!
//! Holds a token per host, so the file is `0600` and written atomically. Losing it means
//! re-pairing every Pi by hand, which is the single most-reported complaint about the
//! comparable feature in Bambu Studio — persistence being dull and correct is the point.

use std::path::{Path, PathBuf};

use driver_core::HostId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// The next unused id.
///
/// Counts past the highest ever used rather than filling gaps: a saved reference to a host the
/// operator forgot must not quietly start meaning a different Pi.
///
// ponytail: ids are `host-N` because a handful of hosts on one desktop cannot exhaust them and
// they read well in a log. If hosts ever sync between machines this needs to be random instead,
// since two desktops would both mint `host-1`.
pub fn next_id(existing: &[PairedHost]) -> HostId {
    let highest = existing
        .iter()
        .filter_map(|h| h.id.0.strip_prefix("host-"))
        .filter_map(|n| n.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    HostId(format!("host-{}", highest + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_host(id: &str, name: &str) -> PairedHost {
        PairedHost {
            id: HostId(id.into()),
            name: name.into(),
            address: "cuthulhu-pi.local:7878".into(),
            fingerprint: "aa:bb:cc".into(),
            token: "s3cret".into(),
        }
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

    #[test]
    fn a_new_id_never_collides_with_one_already_paired() {
        assert_eq!(next_id(&[]), HostId("host-1".into()));
        assert_eq!(next_id(&[a_host("host-1", "a")]), HostId("host-2".into()));
        // A gap left by a forgotten host must not be reused: a stale reference to `host-2`
        // would silently start meaning a different Pi.
        assert_eq!(next_id(&[a_host("host-1", "a"), a_host("host-3", "c")]), HostId("host-4".into()));
    }

    /// An id nobody minted must not derail minting the next one.
    #[test]
    fn an_unrecognised_id_shape_does_not_stop_a_new_one() {
        assert_eq!(next_id(&[a_host("imported-by-hand", "a")]), HostId("host-1".into()));
    }
}
