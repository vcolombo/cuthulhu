// SPDX-License-Identifier: GPL-3.0-or-later
use document::Document;
use serde::{Deserialize, Serialize};
use crate::IoError;

/// The manifest schema version this build writes. Bump it in the same change that appends a
/// step to `STEPS`, never on its own — `every_version_has_a_migration_step` fails otherwise.
pub(crate) const MANIFEST_VERSION: u32 = 2;

/// Every manifest written before the envelope existed: a bare `serde_json::to_string` of
/// `Document` (`Document::snapshot_json`) with no version field at all. It is numbered 1 rather
/// than 0 so that every version a file may declare names a real schema, and so
/// `STEPS[version - LEGACY_UNVERSIONED ..]` is always in range.
pub(crate) const LEGACY_UNVERSIONED: u32 = 1;

/// One entry per version step, in order: `STEPS[i]` migrates `LEGACY_UNVERSIONED + i` to the
/// version after it. A file that declares version `v` runs every step from `v` onward, so no
/// version can skip one. That ordering is the whole difference between this and the absent-field
/// defaults inside `Document`'s own serde (`NodeWire`): those cannot say *when* a field appeared,
/// only that it is missing now.
///
/// A step takes an already-deserialized `Document`, so it can rewrite **values** and nothing
/// else. A version that changes the payload's *shape* — renames a field, restructures an enum,
/// makes a new field required — cannot be repaired here, because `read_manifest` would have
/// failed before reaching this loop. That version diverges at the version-keyed parse instead,
/// where the legacy-bare and envelope arms already split: it gets its own wire type, converted
/// into the current `Document` before these steps run. Nothing pre-builds that arm, because a
/// speculative wire type would freeze a guess about a schema nobody has designed yet; what is
/// pre-built is the version that tells you which arm to write.
const STEPS: &[fn(&mut Document)] = &[legacy_machine_ids];

/// What `save_project` writes. The document is serialized straight from the borrow rather than
/// through `snapshot_json`, whose bare shape stays the IPC and e2e-fake contract.
#[derive(Serialize)]
struct ManifestWire<'a> {
    version: u32,
    document: &'a Document,
}

/// What an enveloped manifest is read as. `version` is deliberately absent here: the probe has
/// already consumed it and serde ignores it — the same probe-then-typed split `presets.json` uses.
#[derive(Deserialize)]
struct ManifestRead {
    document: Document,
}

/// Reads the version and nothing else. A typed probe rather than the `serde_json::Value` probe
/// `cutplan::presets` can afford: a manifest carries the whole document, and serde skips the
/// payload's tokens without building it, so the file is not parsed into memory twice.
#[derive(Deserialize)]
struct VersionProbe {
    #[serde(default)]
    version: Option<u32>,
}

pub(crate) fn write_manifest(doc: &Document) -> String {
    serde_json::to_string(&ManifestWire { version: MANIFEST_VERSION, document: doc })
        .expect("a Document that serializes for snapshot_json serializes inside the envelope")
}

/// Ceiling on a manifest's *decompressed* size, mirroring `trace::MAX_INPUT_FILE_BYTES` — the
/// same quarter gigabyte, for the same reason: a bound on what reading an untrusted file may
/// allocate. A manifest is JSON text, and a design with a hundred thousand nodes writes tens of
/// megabytes, so this clears every real project while refusing a member that inflates without
/// bound. It matters most on the *save* path, where the archive being inspected is one the
/// operator merely aimed at rather than opened.
pub(crate) const MAX_MANIFEST_BYTES: u64 = 256 * 1024 * 1024;

/// Reads a manifest member under `limit` bytes. `None` means it exceeded it and nothing more is
/// known about the member.
///
/// One byte past the limit is read on purpose: a member's declared uncompressed size is a header
/// field a crafted archive can put anything in, so the read itself has to be the bound rather
/// than a check made before it. `limit` is a parameter so the boundary is testable without moving
/// a quarter gigabyte through the suite; every caller passes `MAX_MANIFEST_BYTES`.
pub(crate) fn read_capped(member: &mut impl std::io::Read, limit: u64)
    -> std::io::Result<Option<Vec<u8>>>
{
    use std::io::Read;
    let mut bytes = Vec::new();
    member.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

/// The message a caller reports when `read_capped` returns `None`.
pub(crate) fn too_large() -> String {
    format!("manifest.json is larger than {} MiB", MAX_MANIFEST_BYTES / (1024 * 1024))
}

/// The version a manifest declares, read before any of its document is deserialized. An absent
/// `version` key is what identifies a pre-envelope manifest, and it is unambiguous: `Document`'s
/// fields are `nodes`/`root`/`ids`/`artboard`/`machine`, so no such manifest can carry one.
///
/// An explicit `"version": null` reads as absent too, which `Option` gives for free and which is
/// the same call `NodeWire` makes for a null attribute: nothing here writes one, a null cannot
/// name a schema, and "unversioned" is the only non-arbitrary reading left. It costs nothing
/// either way — an envelope whose version is null still fails the bare-`Document` parse that
/// version 1 implies.
pub(crate) fn probe_version(json: &str) -> Result<u32, IoError> {
    let probe: VersionProbe =
        serde_json::from_str(json).map_err(|e| IoError::Parse(e.to_string()))?;
    Ok(probe.version.unwrap_or(LEGACY_UNVERSIONED))
}

pub(crate) fn read_manifest(json: &str) -> Result<Document, IoError> {
    let version = probe_version(json)?;
    if version > MANIFEST_VERSION {
        return Err(IoError::UnsupportedProjectVersion {
            found: version,
            supported: MANIFEST_VERSION,
        });
    }
    if version < LEGACY_UNVERSIONED {
        // A positive claim to a version nothing ever wrote: malformed rather than "from the
        // future", and refused here so the `STEPS` slice below cannot underflow.
        return Err(IoError::Parse(format!("manifest version {version} was never written")));
    }
    let mut doc = if version == LEGACY_UNVERSIONED {
        serde_json::from_str::<Document>(json).map_err(|e| IoError::Parse(e.to_string()))?
    } else {
        serde_json::from_str::<ManifestRead>(json)
            .map_err(|e| IoError::Parse(e.to_string()))?
            .document
    };
    for step in &STEPS[(version - LEGACY_UNVERSIONED) as usize..] {
        step(&mut doc);
    }
    Ok(doc)
}

/// Version 1 → 2. `builtin_profiles` has written only the canonical ids (`cameo5`, `puma`) since
/// they were renamed, and `set_machine` resolves from that list, so a manifest still carrying
/// `cameo5_alpha`/`puma_iv` predates the envelope by definition. Moved here verbatim from
/// `load_project`, which used to run it on every file regardless of vintage.
fn legacy_machine_ids(doc: &mut Document) {
    if let Some(m) = doc.machine.as_mut() {
        m.id = match m.id.as_str() {
            "cameo5_alpha" => "cameo5".into(),
            "puma_iv" => "puma".into(),
            _ => std::mem::take(&mut m.id),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The boundary, exercised at a limit small enough to keep the suite honest about what it
    /// moves. `read_capped`'s own limit is the parameter; `MAX_MANIFEST_BYTES` is what callers
    /// pass, and `saving_over_an_archive_whose_manifest_inflates_without_bound_is_refused` in
    /// `project.rs` covers the wiring against a real compression bomb.
    #[test]
    fn a_capped_read_stops_one_byte_past_its_limit() {
        use std::io::Read;
        let mut at_limit = std::io::repeat(b'x').take(8);
        assert_eq!(read_capped(&mut at_limit, 8).unwrap(), Some(vec![b'x'; 8]),
            "a member exactly at the limit is returned whole");
        let mut over = std::io::repeat(b'x').take(9);
        assert_eq!(read_capped(&mut over, 8).unwrap(), None, "one byte past it is refused");
        let mut empty = std::io::empty();
        assert_eq!(read_capped(&mut empty, 8).unwrap(), Some(Vec::new()));
    }

    /// The table test that makes "adding a migration requires an explicit version step"
    /// mechanical: a bump with no step, or a step with no bump, fails here first.
    #[test]
    fn every_version_has_a_migration_step() {
        assert_eq!(MANIFEST_VERSION, LEGACY_UNVERSIONED + STEPS.len() as u32);
    }

    #[test]
    fn a_saved_manifest_declares_the_current_version_and_nothing_else() {
        let json = write_manifest(&Document::new());
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["version"].as_u64().unwrap(), MANIFEST_VERSION as u64);
        let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["document", "version"]);
    }

    /// The fail-closed proof: `Document`'s fields carry no serde defaults, so a reader that
    /// predates versioning errors on the missing `nodes` instead of loading an empty document
    /// and later saving it over the real one. If a future change gives `Document` field
    /// defaults, this test fails first.
    #[test]
    fn an_enveloped_manifest_cannot_be_read_as_a_bare_document() {
        assert!(serde_json::from_str::<Document>(&write_manifest(&Document::new())).is_err());
    }

    #[test]
    fn a_manifest_with_no_version_is_the_legacy_schema() {
        assert_eq!(probe_version(&Document::new().snapshot_json()).unwrap(), LEGACY_UNVERSIONED);
    }

    /// The payload is syntactically valid JSON that no `Document` can be built from, so this
    /// can only pass if the version is checked before the document is deserialized.
    #[test]
    fn a_newer_version_is_refused_before_the_document_is_parsed() {
        let future = MANIFEST_VERSION + 97;
        let json = format!(r#"{{"version":{future},"document":{{"nodes":"not a document"}}}}"#);
        match read_manifest(&json) {
            Err(IoError::UnsupportedProjectVersion { found, supported }) => {
                assert_eq!(found, future);
                assert_eq!(supported, MANIFEST_VERSION);
            }
            other => panic!("expected an unsupported-version refusal, got {other:?}"),
        }
    }

    #[test]
    fn malformed_bytes_at_the_current_version_are_a_parse_error() {
        let json =
            format!(r#"{{"version":{MANIFEST_VERSION},"document":{{"nodes":"not a document"}}}}"#);
        assert!(matches!(read_manifest(&json), Err(IoError::Parse(_))));
    }

    #[test]
    fn version_zero_is_malformed_not_legacy() {
        assert!(matches!(
            read_manifest(r#"{"version":0,"document":{}}"#),
            Err(IoError::Parse(_))
        ));
    }

    /// Pins that the legacy detector cannot manufacture a document out of anything that merely
    /// lacks a `version` key.
    #[test]
    fn an_unrelated_json_object_is_a_parse_error_not_an_empty_legacy_document() {
        assert!(matches!(read_manifest(r#"{"unrelated":true}"#), Err(IoError::Parse(_))));
    }

    #[test]
    fn the_legacy_step_renames_both_machine_ids() {
        for (legacy, canonical) in [("cameo5_alpha", "cameo5"), ("puma_iv", "puma")] {
            let mut doc = Document::new();
            doc.machine = Some(document::MachineProfile {
                id: legacy.into(),
                name: "legacy".into(),
                width_mm: 600.0,
                height_mm: 5000.0,
            });
            let back = read_manifest(&doc.snapshot_json()).unwrap();
            assert_eq!(back.machine.unwrap().id, canonical);
        }
    }

    /// Proves the steps are version-gated rather than run unconditionally; nothing this build
    /// writes can produce that id.
    #[test]
    fn a_current_version_manifest_skips_the_legacy_step() {
        let mut doc = Document::new();
        doc.machine = Some(document::MachineProfile {
            id: "puma_iv".into(),
            name: "GCC Puma IV".into(),
            width_mm: 600.0,
            height_mm: 5000.0,
        });
        let back = read_manifest(&write_manifest(&doc)).unwrap();
        assert_eq!(back.machine.unwrap().id, "puma_iv");
    }
}
