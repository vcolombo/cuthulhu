// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::Path;
use std::io::Write;
use document::Document;
use crate::IoError;

/// Write `manifest.json` (the source of truth: a `{ version, document }` envelope, see
/// `crate::manifest`) + `design.svg` (interchange copy) into a zip container at `path`,
/// atomically: build in a temp file in the same directory, then rename over the destination.
///
/// Refuses up front if `path` holds a project written by a newer build, or one that still looks
/// like an archive this build could not inspect — the exact guarantee, with its residue, is on
/// `refuse_overwriting_a_newer_project`. A malformed manifest it *can* read is deliberately not
/// protected: that is a file to replace, not a project to keep.
pub fn save_project(path: &Path, doc: &Document) -> Result<(), IoError> {
    refuse_overwriting_a_newer_project(path)?;
    let manifest = crate::manifest::write_manifest(doc);
    refuse_writing_what_we_could_not_read("manifest", manifest.len() as u64,
        crate::manifest::MAX_MANIFEST_BYTES)?;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| IoError::Io(e.to_string()))?;
    let mut zip = zip::ZipWriter::new(tmp.reopen().map_err(|e| IoError::Io(e.to_string()))?);
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", opts).map_err(|e| IoError::Io(e.to_string()))?;
    zip.write_all(manifest.as_bytes()).map_err(|e| IoError::Io(e.to_string()))?;
    zip.start_file("design.svg", opts).map_err(|e| IoError::Io(e.to_string()))?;
    zip.write_all(crate::doc_to_svg(doc).as_bytes()).map_err(|e| IoError::Io(e.to_string()))?;
    zip.finish().map_err(|e| IoError::Io(e.to_string()))?;
    let written = tmp.as_file().metadata().map_err(|e| IoError::Io(e.to_string()))?.len();
    refuse_writing_what_we_could_not_read("archive", written, MAX_PROJECT_BYTES)?;
    tmp.persist(path).map_err(|e| IoError::Io(e.to_string()))?;   // atomic rename
    Ok(())
}

/// Applies a *read* ceiling to what this build is about to write, so that a project it saves is
/// always a project it can reopen.
///
/// Without this the two ends disagree: the reader refuses an oversized archive to bound what `zip`
/// allocates from it, and a save with no matching limit would produce exactly such a file — which
/// this build would then refuse to open, and refuse to save over, leaving the operator's work in a
/// file only some other program can read. Refusing before the temp file is renamed means the
/// destination is untouched and the operator is told while the document is still in front of them.
fn refuse_writing_what_we_could_not_read(what: &str, len: u64, limit: u64)
    -> Result<(), IoError>
{
    if len > limit {
        return Err(IoError::Io(format!(
            "this project is too large to save: its {what} is over {} MiB, which this build \
             would refuse to reopen",
            limit / (1024 * 1024)
        )));
    }
    Ok(())
}

/// Ceiling on a `.cut` container's own *logical* size, checked before `zip` parses any of it.
///
/// `zip 2.4.2` sizes two allocations from header fields whose only bound is a distance inside the
/// file: the ZIP64 end-of-central-directory record's extensible data sector, a
/// `vec![0u8; record_size - 44]` bounded only by the gap to its locator (`spec.rs`
/// `Zip64CentralDirectoryEnd::parse`), and the central directory's entry-count preallocation,
/// bounded only by `directory_start` (`read.rs` `read_central_header`). A **sparse** file can
/// advertise gigabytes of such distance while occupying almost no storage, so the logical length is
/// what has to be bounded, and it has to be bounded before the archive is opened rather than after.
///
/// **32 MiB, chosen from the amplification rather than from taste.** `zip` reserves capacity for
/// the entry count the trailer claims before validating a single entry, and it only requires that
/// those entries' 46-byte on-disk headers would fit — so the claim a container of size `C` can
/// carry is `C / 46`.
///
/// What that costs has to be measured as **peak live** bytes, not as the largest single allocation:
/// `zip` builds an `IndexMap` with capacity from the entry count *before* consuming its
/// `Vec<ZipFileData>`, so both arrays are resident at once. Measured with a counting allocator
/// against real archives, the peak is **508 bytes per entry** (48.3 MiB for 100 000, 96.9 MiB for
/// 200 000 — the largest single allocation is only 232 bytes per entry, which is the number that
/// misled an earlier revision of this constant). That is an 11× amplification of the container:
///
/// | ceiling | claimable entries | peak while opening |
/// |---|---|---|
/// | 16 MiB | 364 722 | 177 MiB |
/// | 32 MiB | 729 444 | 353 MiB |
/// | 48 MiB | 1 094 166 | 530 MiB |
/// | 64 MiB | 1 458 888 | 707 MiB |
///
/// A reservation this size is not a slow save; allocation failure **aborts** the process, which on
/// a small machine costs the operator the document they had open — the exact loss the rest of this
/// module exists to prevent. 32 MiB is the largest ceiling whose peak (353 MiB) stays clear of
/// `MAX_DECODE_ALLOC` (512 MiB), the most this codebase already tolerates; 48 MiB reaches 530 MiB
/// and exceeds it.
///
/// It costs nothing real: a 100 000-node design measures 0.9 MiB on disk (its manifest is 20.9 MiB
/// of JSON, compressing ~23×), so this is ~35× the largest project anyone plausibly has. Revisit it
/// when the `assets/` member the design anticipates actually lands, since embedded images are the
/// one thing that could approach it.
///
/// The amplification is *bounded* here, not removed — a true fix needs a bound on allocation, and
/// `zip 2.4.2` exposes none (`read::Config` carries only `archive_offset`). That, and the fact that
/// this is a one-time `fstat` a concurrent writer could race, are recorded on #262.
const MAX_PROJECT_BYTES: u64 = 32 * 1024 * 1024;

fn project_too_large() -> String {
    format!("the file is larger than {} MiB", MAX_PROJECT_BYTES / (1024 * 1024))
}

/// Reads the manifest under `manifest::MAX_MANIFEST_BYTES`, out of a container under
/// `MAX_PROJECT_BYTES`: a `.cut` is a zip, and neither a member's decompressed size nor the
/// metadata `zip` allocates from is bounded by anything the file has to actually contain.
///
/// The archive is scoped to the read, not to the function, so its central-directory index — up to
/// ~192 MiB of it, at what `MAX_PROJECT_BYTES` admits — is freed before the document is
/// deserialized. Deserializing is itself the largest allocation on this path (a `Document` plus the
/// JSON it came from), and holding the index across it would stack the two peaks for no reason.
pub fn load_project(path: &Path) -> Result<Document, IoError> {
    let file = std::fs::File::open(path).map_err(|e| IoError::Io(e.to_string()))?;
    if file.metadata().map_err(|e| IoError::Io(e.to_string()))?.len() > MAX_PROJECT_BYTES {
        return Err(IoError::Io(project_too_large()));
    }
    let text = {
        let mut zip = zip::ZipArchive::new(file).map_err(|e| IoError::Parse(e.to_string()))?;
        let mut member = zip.by_name("manifest.json").map_err(|e| IoError::Parse(e.to_string()))?;
        let bytes = crate::manifest::read_capped(&mut member, crate::manifest::MAX_MANIFEST_BYTES)
            .map_err(|e| IoError::Io(e.to_string()))?
            .ok_or_else(|| IoError::Io(crate::manifest::too_large()))?;
        String::from_utf8(bytes).map_err(|e| IoError::Parse(e.to_string()))?
    };
    crate::manifest::read_manifest(&text)
}

/// The one way a project this build refused to open can still be destroyed is a Save As aimed
/// back at it: `AppState` keeps no path for a load that failed, but the file picker will happily
/// offer the file again. So the refusal lives at the write itself rather than in a caller's
/// bookkeeping, which also covers any future non-desktop writer.
///
/// What it guarantees: a destination that **opens as an archive** and declares a version above
/// this build's is refused, whatever else is wrong with the file and whatever is prepended to
/// it; and a destination that opens, or that still *looks* like an archive, is never replaced on
/// the strength of an inspection that failed. So an unreadable file, a damaged central directory,
/// clobbered leading bytes, an archive this build of `zip` cannot handle, a member it cannot
/// decompress and a manifest whose CRC does not check out all fail **closed** — each may be a
/// present manifest this build cannot read, which is the case the guard exists for.
///
/// It fails *open* only where it can name the reason the destination is **not** such a project:
/// no file there, bytes that neither open as an archive nor look like one, an archive with no
/// `manifest.json`, a manifest that is not UTF-8, a manifest whose version does not parse. The
/// last two are deliberate: a manifest this build can read and cannot make sense of is a file to
/// replace, not a project to keep, or a corrupt `.cut` would become a path that can never be
/// saved to again.
///
/// The residue is an archive that neither opens **nor** shows its shape to either bounded check:
/// its leading marker gone — buried behind prepended data, or damaged outright — *and* no
/// end-of-central-directory signature within the last 64 KiB, whether because that record was
/// destroyed or because non-conforming trailing data pushed it out of the window. Seeing those
/// takes a scan of the whole file, which starts refusing ordinary binaries that merely contain
/// four such bytes by chance. Recorded on #262 with the rest of the crafted-archive family.
///
/// An archive naming `manifest.json` twice is *not* part of that residue, though it reads like it
/// should be: `zip` keeps the last such member and discards the other before any caller sees it, so
/// the version this guard reads is the only version anything in this workspace can read. That
/// reading is pinned by
/// `tests::a_duplicate_manifest_member_reads_as_the_last_one_and_hides_the_other`, because a crate
/// that stopped collapsing duplicates would turn the ambiguity into something a caller could refuse
/// — which is the condition #262 records for revisiting it.
fn refuse_overwriting_a_newer_project(path: &Path) -> Result<(), IoError> {
    use zip::result::ZipError;
    let uninspectable = |e: &dyn std::fmt::Display| {
        Err(IoError::Io(format!("the file being replaced could not be inspected ({e})")))
    };
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return uninspectable(&e),
    };
    // Before `zip` parses anything: the metadata it sizes allocations from is bounded only by
    // distances inside the file, and a sparse destination can advertise gigabytes of those. An
    // fstat that fails is not a fast path here but the bound itself, so it fails closed too.
    match file.metadata() {
        Ok(m) if m.len() > MAX_PROJECT_BYTES => return uninspectable(&project_too_large()),
        Ok(_) => {}
        Err(e) => return uninspectable(&e),
    }
    let mut zip = match zip::ZipArchive::new(file) {
        Ok(zip) => zip,
        // It did not open, and `ZipError::InvalidArchive` cannot settle why: `zip` returns it
        // both for bytes that are not an archive and for a real one whose bookkeeping is damaged,
        // whose `manifest.json` may still be sitting there intact. So the question becomes
        // whether the file still looks like an archive at all.
        Err(e) => return match looks_like_an_archive(path) {
            Ok(true) => uninspectable(&e),
            Ok(false) => Ok(()),
            // Could not even re-read it to decide. Fail closed for the same reason as above.
            Err(io) => uninspectable(&io),
        },
    };
    let bytes = match zip.by_name("manifest.json") {
        // Read to bytes rather than to a `String`: `read_to_string` reports a failed CRC check
        // and a non-UTF-8 member with the same `InvalidData` kind, and those are opposite
        // answers — one is an archive this build could not read, the other is not a manifest.
        //
        // Capped, because this is the one path that inflates a member of a file the operator
        // only *aimed* at: a crafted archive whose manifest decompresses without bound would
        // otherwise exhaust memory before the guard could refuse anything.
        Ok(mut member) => {
            match crate::manifest::read_capped(&mut member, crate::manifest::MAX_MANIFEST_BYTES) {
                Ok(Some(bytes)) => bytes,
                // Too large to inspect is still not evidence that there is nothing to protect.
                Ok(None) => return uninspectable(&crate::manifest::too_large()),
                Err(e) => return uninspectable(&e),
            }
        }
        // Only `FileNotFound` proves the member is absent; anything else means it is there and
        // could not be reached.
        Err(ZipError::FileNotFound) => return Ok(()),
        Err(e) => return uninspectable(&e),
    };
    // The archive index is not needed past this point, and the probe below allocates too: `zip`'s
    // is freed first, for the same reason `load_project` scopes it to its read.
    drop(zip);
    let Ok(text) = String::from_utf8(bytes) else { return Ok(()) };
    match crate::manifest::probe_version(&text) {
        Ok(found) if found > crate::manifest::MANIFEST_VERSION => {
            Err(IoError::UnsupportedProjectVersion {
                found,
                supported: crate::manifest::MANIFEST_VERSION,
            })
        }
        _ => Ok(()),
    }
}

/// Whether a file `zip` could not open still carries the shape of an archive, asked only in that
/// failure arm and only to decide whether replacing it would destroy something.
///
/// Two pieces of evidence, both bounded, because the alternative — searching the whole file for a
/// signature — has a real collision rate on large files (a 100 MB binary hits any given four
/// bytes by chance about one time in fifty) and would turn ordinary Save As targets into
/// refusals:
///
/// - The **leading signature**, which every archive with nothing prepended begins with: a local
///   file header (`0x04034b50`), a bare end-of-central-directory record for an empty archive
///   (`0x06054b50`), the spanning signature that starts a split archive's first segment
///   (`0x08074b50`), or the temporary spanning marker that replaces it when the split turned out
///   to need only one segment (`0x30304b50`, the bytes `PK00`).
/// - The **end-of-central-directory record**, which the format puts within its own 22 bytes plus
///   a `.ZIP file comment` of at most 64 KiB of the end. That is the *format's* bound, not the
///   pinned `zip 2.4.2`'s, which scans the whole file for the record; a conforming window is the
///   narrower question on purpose. It is what still identifies an archive whose front has been
///   clobbered — four damaged bytes there stop `ZipArchive::new` from finding anything (it
///   reports "Could not find EOCD") while leaving `manifest.json` fully recoverable.
///
/// `[doc: PKWARE .ZIP File Format Specification 6.3.10,
/// https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT, §4.3.7 local file header,
/// §4.3.16 end of central directory record, §8.5.3 spanning signature, §8.5.4 temporary
/// spanning marker]`
fn looks_like_an_archive(path: &Path) -> Result<bool, std::io::Error> {
    use std::io::{Read, Seek, SeekFrom};
    // The record itself, plus the largest comment its 2-byte length field can describe.
    const TRAILER: u64 = 22 + u16::MAX as u64;
    let mut file = std::fs::File::open(path)?;
    let mut signature = [0u8; 4];
    match file.read_exact(&mut signature) {
        Ok(()) if matches!(&signature,
            b"PK\x03\x04" | b"PK\x05\x06" | b"PK\x07\x08" | b"PK00") => {
            return Ok(true)
        }
        Ok(()) => {}
        // Too short to hold a signature, so too short to be an archive.
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
        Err(e) => return Err(e),
    }
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TRAILER)))?;
    let mut trailer = Vec::new();
    file.take(TRAILER).read_to_end(&mut trailer)?;
    Ok(trailer.windows(4).any(|w| w == b"PK\x05\x06"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// The write limits, at their boundary and at the limits the *read* paths use — the property
    /// Greptile found violated: a project this build saves must be one it can reopen. Exercised
    /// through the predicate rather than a real oversized project, because building a
    /// 32 MiB document would add no evidence; the two call sites pass the same constants
    /// `load_project` and the guard enforce.
    #[test]
    fn a_save_refuses_output_this_build_would_not_read_back() {
        for limit in [crate::manifest::MAX_MANIFEST_BYTES, MAX_PROJECT_BYTES] {
            assert!(refuse_writing_what_we_could_not_read("manifest", limit, limit).is_ok(),
                "exactly at the limit is writable, since the reader accepts it");
            match refuse_writing_what_we_could_not_read("archive", limit + 1, limit) {
                Err(IoError::Io(m)) => {
                    assert!(m.contains("too large to save"), "{m}");
                    assert!(m.contains(&format!("{} MiB", limit / (1024 * 1024))), "{m}");
                    assert!(m.contains("refuse to reopen"), "{m}");
                }
                other => panic!("expected a refusal naming the ceiling, got {other:?}"),
            }
        }
    }

    /// The container bound, exercised on the real constant the way `trace`'s file-size test is:
    /// the 32 MiB file is *extended* rather than written, so the suite moves no payload and — on a
    /// filesystem that leaves the range unallocated — stores none either. That is also exactly the
    /// shape of the attack: `zip` sizes allocations from distances inside a file, and a sparse file
    /// can advertise them for free.
    #[test]
    fn saving_over_a_destination_too_large_to_be_a_project_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sparse.cut");
        std::fs::File::create(&path).unwrap().set_len(MAX_PROJECT_BYTES + 1).unwrap();

        match save_project(&path, &document::Document::new()) {
            Err(IoError::Io(m)) => assert!(m.contains("larger than"), "{m}"),
            other => panic!("expected a refusal naming the size, got {other:?}"),
        }
        assert_eq!(std::fs::metadata(&path).unwrap().len(), MAX_PROJECT_BYTES + 1,
            "the destination is untouched");
    }

    /// The same bound on the way in. Opening is the operator's explicit act, so this is an error
    /// rather than a refusal to write, but the allocation it prevents is the same one.
    #[test]
    fn opening_a_file_too_large_to_be_a_project_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sparse.cut");
        std::fs::File::create(&path).unwrap().set_len(MAX_PROJECT_BYTES + 1).unwrap();
        assert!(matches!(load_project(&path), Err(IoError::Io(m)) if m.contains("larger than")));
    }
    /// The cap, end to end, against a member that inflates a thousandfold: 32 MiB of zeros deflates
    /// to a few dozen kilobytes, so an archive an operator merely aims at can ask this build to
    /// allocate without bound. Refused as uninspectable — too large to read is not evidence that
    /// there is nothing to protect — and the destination is left alone.
    #[test]
    fn saving_over_an_archive_whose_manifest_inflates_without_bound_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bomb.cut");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        let zeros = vec![0u8; 1024 * 1024];
        let mut written = 0u64;
        while written <= crate::manifest::MAX_MANIFEST_BYTES {
            zip.write_all(&zeros).unwrap();
            written += zeros.len() as u64;
        }
        zip.finish().unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(before.len() < 4 * 1024 * 1024, "premise: it is small on disk, huge inflated");

        match save_project(&path, &document::Document::new()) {
            Err(IoError::Io(m)) => assert!(m.contains("larger than"), "{m}"),
            other => panic!("expected a refusal naming the size, got {other:?}"),
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /// The legacy step at the archive level. It must be a hand-built pre-envelope manifest: a
    /// project this build saves declares the current version, whose steps correctly skip this
    /// rename, so a `save_project` round-trip can no longer produce the fixture.
    #[test]
    fn a_project_saved_before_versioning_migrates_its_machine_id() {
        let mut doc = document::Document::new();
        doc.machine = Some(document::MachineProfile { id: "puma_iv".into(),
            name: "GCC Puma IV".into(), width_mm: 600.0, height_mm: 5000.0 });
        let manifest: serde_json::Value = serde_json::from_str(&doc.snapshot_json()).unwrap();
        assert!(manifest.get("version").is_none(),
            "premise: a bare document snapshot is what a pre-envelope manifest was");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.cut");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.finish().unwrap();

        let back = load_project(&path).unwrap();
        assert_eq!(back.machine.unwrap().id, "puma");
    }

    /// The absent-field migration through a real project file, which is the level it matters
    /// at: a legacy *value* can be planted straight into a live `Document`, but an absent field
    /// cannot be — `save_project` always writes every field. So this fixture is a current
    /// snapshot with `cut_line_type` pruned back out of it, which is the only way to express
    /// "written before that field existed" without hand-maintaining a whole manifest.
    ///
    /// Being generated, it tracks `Document`'s current shape: a field added later appears here
    /// too, so this test says nothing about whether a *real* old file still loads.
    /// `a_frozen_version_one_project_still_loads_with_equivalent_state` is the one that does.
    #[test]
    fn a_project_saved_before_cuttability_derives_it_from_stroke() {
        let mut doc = document::Document::new();
        let mut stroked = document::Node::shape(doc.ids.next(),
            document::ShapeKind::Rect { w: 10.0, h: 10.0 });
        stroked.style = document::Style { stroke: Some(0xFF0000FF), fill: None };
        let stroked_id = stroked.id;
        let mut fill_only = document::Node::shape(doc.ids.next(),
            document::ShapeKind::Rect { w: 10.0, h: 10.0 });
        fill_only.style = document::Style { stroke: None, fill: Some(0x00FF00FF) };
        let fill_only_id = fill_only.id;
        doc.apply(document::Delta(vec![
            document::NodeOp::Add { parent: doc.root, node: stroked, index: usize::MAX },
            document::NodeOp::Add { parent: doc.root, node: fill_only, index: usize::MAX },
        ]));

        let mut manifest: serde_json::Value = serde_json::from_str(&doc.snapshot_json()).unwrap();
        for node in manifest["nodes"].as_object_mut().unwrap().values_mut() {
            assert!(node.as_object_mut().unwrap().remove("cut_line_type").is_some(),
                "premise: every node is written with the field, so pruning it makes an old file");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.cut");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.finish().unwrap();

        let back = load_project(&path).unwrap();
        assert_eq!(back.get(stroked_id).unwrap().cut_line_type, document::CutLineType::Cut);
        assert_eq!(back.get(fill_only_id).unwrap().cut_line_type, document::CutLineType::NoCut);
    }

    /// The same migration for the material assignment, which needs its own fixture because
    /// the two absent-field rules are deliberately different: a missing `cut_line_type`
    /// derives from the stroke, a missing `material_preset` is simply `Inherit`.
    #[test]
    fn a_project_saved_before_material_assignments_inherits() {
        let mut doc = document::Document::new();
        let shape = document::Node::shape(doc.ids.next(),
            document::ShapeKind::Rect { w: 10.0, h: 10.0 });
        let shape_id = shape.id;
        doc.apply(document::Delta(vec![
            document::NodeOp::Add { parent: doc.root, node: shape, index: usize::MAX },
        ]));

        let mut manifest: serde_json::Value = serde_json::from_str(&doc.snapshot_json()).unwrap();
        for node in manifest["nodes"].as_object_mut().unwrap().values_mut() {
            assert!(node.as_object_mut().unwrap().remove("material_preset").is_some(),
                "premise: every node is written with the field, so pruning it makes an old file");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.cut");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.finish().unwrap();

        let back = load_project(&path).unwrap();
        assert_eq!(back.get(shape_id).unwrap().material_preset, document::PresetAssignment::Inherit);
        assert_eq!(back.get(shape_id).unwrap().cut_line_type, document::CutLineType::Cut,
            "premise: the neighbouring migration still runs");
    }

    #[test]
    fn save_then_load_round_trips_document() {
        let mut doc = document::Document::new();
        let id = doc.ids.next();
        doc.apply(document::Delta(vec![document::NodeOp::Add {
            parent: doc.root, index: 0,
            node: document::Node::shape(id, document::ShapeKind::Rect { w: 5.0, h: 5.0 }) }]));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proj.cut");
        super::save_project(&path, &doc).unwrap();
        let back = super::load_project(&path).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn a_saved_archive_carries_a_versioned_manifest_and_an_unchanged_design_svg() {
        let doc = document::Document::new();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proj.cut");
        save_project(&path, &doc).unwrap();

        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut manifest = String::new();
        zip.by_name("manifest.json").unwrap().read_to_string(&mut manifest).unwrap();
        assert_eq!(crate::manifest::probe_version(&manifest).unwrap(),
            crate::manifest::MANIFEST_VERSION);
        let mut svg = String::new();
        zip.by_name("design.svg").unwrap().read_to_string(&mut svg).unwrap();
        assert_eq!(svg, crate::doc_to_svg(&doc));
    }

    /// A project from a newer build must be named, not mistaken for corruption.
    #[test]
    fn opening_a_project_from_a_newer_build_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.cut");
        write_future_project(&path);
        match load_project(&path) {
            Err(IoError::UnsupportedProjectVersion { found, supported }) => {
                assert_eq!(found, 99);
                assert_eq!(supported, crate::manifest::MANIFEST_VERSION);
            }
            other => panic!("expected an unsupported-version refusal, got {other:?}"),
        }
    }

    /// The data-destruction proof: the normal flow is `load_project` (refused) then a Save As
    /// aimed back at the same path, which must leave the file byte-identical.
    #[test]
    fn saving_over_a_project_from_a_newer_build_is_refused_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.cut");
        write_future_project(&path);
        let before = std::fs::read(&path).unwrap();

        let mut other = document::Document::new();
        let id = other.ids.next();
        other.apply(document::Delta(vec![document::NodeOp::Add {
            parent: other.root, index: 0,
            node: document::Node::shape(id, document::ShapeKind::Rect { w: 5.0, h: 5.0 }) }]));
        assert!(matches!(save_project(&path, &other),
            Err(IoError::UnsupportedProjectVersion { found: 99, .. })));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /// `zip` locates an archive by scanning back for its central directory, so a container with
    /// arbitrary bytes prepended — a self-extracting stub, or two files concatenated — opens
    /// normally. `load_project` therefore reads such a project and names its version, and the
    /// guard has to agree: a destination whose first bytes are not a zip signature is not
    /// automatically a destination with nothing to protect.
    #[test]
    fn saving_over_a_prefixed_archive_from_a_newer_build_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("future.cut");
        write_future_project(&plain);
        let path = dir.path().join("prefixed.cut");
        let mut prefixed = b"MZ this is a self-extracting stub".to_vec();
        prefixed.extend_from_slice(&std::fs::read(&plain).unwrap());
        std::fs::write(&path, &prefixed).unwrap();
        assert!(matches!(load_project(&path),
            Err(IoError::UnsupportedProjectVersion { found: 99, .. })),
            "premise: prepended data does not stop this build from reading the project");

        let before = std::fs::read(&path).unwrap();
        assert!(matches!(save_project(&path, &document::Document::new()),
            Err(IoError::UnsupportedProjectVersion { found: 99, .. })));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /// Pins the guard's deliberate fail-open: it protects projects this build cannot read, not
    /// every path that happens to exist.
    #[test]
    fn saving_over_bytes_that_are_not_a_project_still_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proj.cut");
        std::fs::write(&path, b"not a zip").unwrap();
        let doc = document::Document::new();
        save_project(&path, &doc).unwrap();
        assert_eq!(load_project(&path).unwrap(), doc);
    }

    /// Overwriting the local file header signature stops `zip` from finding anything — it reports
    /// "Could not find EOCD" even though the central directory and `manifest.json` are intact —
    /// so the leading signature alone cannot decide whether there is a project here. The
    /// end-of-central-directory record at the other end still says there is.
    ///
    /// `[doc: PKWARE .ZIP File Format Specification 6.3.10,
    /// https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT, §4.3.7 local file header,
    /// §4.3.16 end of central directory record]`
    #[test]
    fn saving_over_an_archive_whose_leading_bytes_are_damaged_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("head-damaged.cut");
        save_project(&path, &document::Document::new()).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[..4].copy_from_slice(b"XY\0\0");
        std::fs::write(&path, &bytes).unwrap();
        assert!(zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).is_err(),
            "premise: the archive no longer opens");
        let before = std::fs::read(&path).unwrap();

        assert!(matches!(save_project(&path, &document::Document::new()), Err(IoError::Io(_))));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /// The other bounded check, on its own: a split archive that needed only one segment begins
    /// with the temporary spanning marker instead of a local file header, so a file that starts
    /// `PK00` and has lost its trailer is still an archive — recognisable by nothing but that
    /// marker.
    ///
    /// `[doc: PKWARE .ZIP File Format Specification 6.3.10,
    /// https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT, §8.5.4 temporary spanning
    /// marker]`
    #[test]
    fn saving_over_a_split_marked_archive_with_no_trailer_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pk00.cut");
        save_project(&path, &document::Document::new()).unwrap();
        let mut bytes = b"PK00".to_vec();
        bytes.extend_from_slice(&std::fs::read(&path).unwrap());
        let end = bytes.len();
        bytes.truncate(end - 24); // the end-of-central-directory record, gone
        std::fs::write(&path, &bytes).unwrap();
        assert!(zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).is_err(),
            "premise: the archive no longer opens");
        assert!(!std::fs::read(&path).unwrap().windows(4).rev().take(65557)
            .any(|w| w == b"PK\x05\x06"),
            "premise: no trailer is left, so only the leading marker can identify it");
        let before = std::fs::read(&path).unwrap();

        assert!(matches!(save_project(&path, &document::Document::new()), Err(IoError::Io(_))));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /// The counterpart to the guard's fail-open, and the reason that policy is a judgement
    /// rather than a shortcut: a destination that exists but cannot be read is not evidence of
    /// anything, so the save is refused instead of proceeding on an inspection that never
    /// happened. Unix-only because there is no portable way to make a file unreadable while
    /// leaving its directory writable, which is the shape that makes the overwrite possible.
    #[cfg(unix)]
    #[test]
    fn saving_over_a_destination_that_cannot_be_inspected_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unreadable.cut");
        save_project(&path, &document::Document::new()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&path).is_ok() {
            // Running as a user who ignores the mode bits (root); the premise cannot be set up.
            return;
        }
        assert!(matches!(save_project(&path, &document::Document::new()), Err(IoError::Io(_))));
    }

    /// The case no error *kind* can decide: `zip` reports a failed CRC check and a non-UTF-8
    /// member with the same `InvalidData`, and those are opposite answers — one is a present
    /// manifest this build could not read, the other is not a manifest. The member is **stored**
    /// rather than deflated and corrupted in the middle, so the failure is specifically the
    /// checksum: a deflated member could fail to inflate instead, which is a different error and
    /// would let this test pass without covering the ambiguous one. The premise is asserted, not
    /// assumed.
    #[test]
    fn saving_over_an_archive_whose_manifest_does_not_check_out_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let stored_current_project = |path: &std::path::Path| {
            let manifest = crate::manifest::write_manifest(&document::Document::new());
            let stored = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
            zip.start_file("manifest.json", stored).unwrap();
            zip.write_all(manifest.as_bytes()).unwrap();
            zip.finish().unwrap();
        };

        // The same fixture twice: one copy proves it is replaceable while intact, so the refusal
        // below can only come from the corruption. Proving that on the corrupted copy's own path
        // would replace the fixture before it could be corrupted.
        let intact = dir.path().join("intact.cut");
        stored_current_project(&intact);
        assert!(save_project(&intact, &document::Document::new()).is_ok(),
            "premise: intact, this destination is a current-version project and replaceable");

        let path = dir.path().join("corrupt.cut");
        stored_current_project(&path);
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let at = archive.by_name("manifest.json").unwrap().data_start() as usize;
        drop(archive);
        let mut bytes = std::fs::read(&path).unwrap();
        // Inside the stored JSON, so the member still reads back byte-for-byte — just not the
        // bytes its CRC was computed over.
        bytes[at + 8] ^= 0x20;
        std::fs::write(&path, &bytes).unwrap();

        let mut probe = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let read = probe.by_name("manifest.json").unwrap().read_to_end(&mut Vec::new());
        assert_eq!(read.unwrap_err().kind(), std::io::ErrorKind::InvalidData,
            "premise: the corruption surfaces as the same kind a non-UTF-8 member does");
        drop(probe);

        let before = std::fs::read(&path).unwrap();
        match save_project(&path, &document::Document::new()) {
            Err(IoError::Io(_)) => {}
            other => panic!("expected a refusal to replace an archive it could not inspect, \
                got {other:?}"),
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /// A zip whose bookkeeping is broken but whose `manifest.json` may still be intact behind
    /// it. `ZipError::InvalidArchive` covers this as well as bytes that are not a zip, which is
    /// why the signature — not that error — is what the guard reads to tell the two apart.
    #[test]
    fn saving_over_a_zip_whose_directory_is_damaged_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("damaged.cut");
        save_project(&path, &document::Document::new()).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let end = bytes.len();
        bytes.truncate(end - 8); // the end-of-central-directory record, gone
        std::fs::write(&path, &bytes).unwrap();
        assert!(zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).is_err(),
            "premise: the archive no longer opens");
        assert!(bytes.starts_with(b"PK\x03\x04"), "premise: it still says it is a zip");
        let before = std::fs::read(&path).unwrap();

        assert!(matches!(save_project(&path, &document::Document::new()), Err(IoError::Io(_))));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /// The positive counterpart: a manifest member whose bytes are intact but not text. `zip`
    /// gives the same `InvalidData` kind as the corrupt archive above, and the answer is the
    /// opposite — nothing this build ever wrote is non-UTF-8, so there is no project to protect.
    #[test]
    fn saving_over_an_archive_whose_manifest_is_not_text_still_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("binary-manifest.cut");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&[0xFF, 0xFE, 0xFD, 0xFC]).unwrap();
        zip.finish().unwrap();

        let doc = document::Document::new();
        save_project(&path, &doc).unwrap();
        assert_eq!(load_project(&path).unwrap(), doc);
    }

    /// A byte-frozen version-1 manifest: exactly what the last pre-envelope build wrote — a bare
    /// `Document` snapshot with no `version` key, a rect inside a translated group, and the
    /// retired `puma_iv` machine id.
    ///
    /// Frozen as a literal rather than regenerated from `Document::snapshot_json()`, which is
    /// what the two absent-field fixtures above must do to prune a field. A generated fixture
    /// silently acquires every field a later feature adds, so it stops being an old file at the
    /// exact moment the legacy path could break: a real version-1 file lacks that field, and a
    /// generated one does not. This is the fixture that keeps failing when that happens.
    const FROZEN_V1_MANIFEST: &str = r#"{"nodes":{"2":{"id":2,"kind":{"Shape":{"Rect":{"w":10.0,"h":20.0}}},"transform":[1.0,0.0,0.0,1.0,0.0,0.0],"style":{"stroke":4278190335,"fill":null},"cut_line_type":"Cut","material_preset":{"state":"inherit"},"children":[]},"1":{"id":1,"kind":"Layer","transform":[1.0,0.0,0.0,1.0,0.0,0.0],"style":{"stroke":255,"fill":null},"cut_line_type":"Cut","material_preset":{"state":"inherit"},"children":[3]},"3":{"id":3,"kind":"Group","transform":[1.0,0.0,0.0,1.0,3.0,4.0],"style":{"stroke":255,"fill":null},"cut_line_type":"Cut","material_preset":{"state":"inherit"},"children":[2]}},"root":1,"ids":3,"artboard":{"x":0.0,"y":0.0,"w":330.0,"h":3000.0},"machine":{"id":"puma_iv","name":"GCC Puma IV","width_mm":600.0,"height_mm":5000.0}}"#;

    #[test]
    fn a_frozen_version_one_project_still_loads_with_equivalent_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v1.cut");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(FROZEN_V1_MANIFEST.as_bytes()).unwrap();
        zip.finish().unwrap();

        // The whole document, not a sample of it: re-serialize what loaded and compare against
        // the frozen bytes with the one intended difference applied. `serde_json::Value`
        // compares structurally, so map order does not enter into it. A future version-specific
        // wire conversion that dropped any field — machine dimensions, artboard origin, a node
        // id, a fill — fails here, which a hand-picked list of assertions would not.
        let doc = load_project(&path).unwrap();
        let expected: serde_json::Value =
            serde_json::from_str(&FROZEN_V1_MANIFEST.replace("puma_iv", "puma")).unwrap();
        let actual: serde_json::Value = serde_json::from_str(&doc.snapshot_json()).unwrap();
        assert_eq!(actual, expected);
        assert!(FROZEN_V1_MANIFEST.contains("puma_iv"),
            "premise: the frozen file carries the retired id, so the comparison above proves \
             the legacy step rewrote it");
    }

    /// An archive whose manifest declares a version no build has ever written.
    fn write_future_project(path: &std::path::Path) {
        let manifest = format!(r#"{{"version":99,"document":{}}}"#,
            document::Document::new().snapshot_json());
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    /// How many times a member name appears in an archive's bytes. Twice per member: the format
    /// stores it in the member's own local file header and again in its central directory entry, and
    /// both are plain bytes with an explicit length field
    /// `[doc: PKWARE .ZIP File Format Specification 6.3.10,
    /// https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT, §4.3.7 local file header,
    /// §4.3.12 central directory structure]`.
    fn times_named(bytes: &[u8], name: &[u8]) -> usize {
        bytes.windows(name.len()).filter(|window| *window == name).count()
    }

    /// Renames every occurrence of a member name in an archive's bytes, which is how the fixture
    /// below gets two *physical* members called `manifest.json` — two local file headers and two
    /// central directory entries carrying the same name, which is the ambiguity the reader then
    /// collapses. Both names are 13 bytes, so nothing has to move: the name's length is recorded in
    /// each of those two records, and no offset, size or CRC in the format depends on which name a
    /// member carries (same citation as above).
    fn rename_every_member(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
        assert_eq!(from.len(), to.len(), "a rename that moves bytes would invalidate every offset");
        let mut out = bytes.to_vec();
        let mut at = 0;
        while at + from.len() <= out.len() {
            if &out[at..at + from.len()] == from {
                out[at..at + from.len()].copy_from_slice(to);
                at += from.len();
            } else {
                at += 1;
            }
        }
        out
    }

    /// How `zip 2.4.2` reads an archive that names `manifest.json` twice, pinned because it is the
    /// whole reason #262 is a recorded constraint rather than a fix: the crate collapses duplicate
    /// names before any caller sees them, keeping the **last**. `len()` is 1, `by_name` answers
    /// once, and `by_index` stops at 1 — so the shadowed member is unaddressable by every reader in
    /// this workspace, including any future Cuthulhu build, since they all read through this crate.
    /// Under the only reading available, such an archive *is* whatever its last member says, which
    /// is why replacing it is the ordinary open/edit/save flow rather than a violated invariant:
    /// the guarantee — a project this build refuses to open is not replaced by it — holds in both
    /// orders, which is what the second half of this test states.
    ///
    /// Nothing Cuthulhu can run writes such a file: `ZipWriter` refuses the second member outright,
    /// asserted here because it is what forced the hand-patched fixture. A crate upgrade that
    /// errors on duplicate names, or that exposes an entry count disagreeing with the name map,
    /// fails this test — and that is the evidence #262 asks for before the decision is revisited,
    /// since load could then refuse the ambiguous archive as malformed and the guard would follow.
    #[test]
    fn a_duplicate_manifest_member_reads_as_the_last_one_and_hides_the_other() {
        /// 13 bytes, exactly like `manifest.json`, and nothing a real archive would carry.
        const PLACEHOLDER: &str = "second_member";
        let options = zip::write::SimpleFileOptions::default();

        let mut refuses = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        refuses.start_file("manifest.json", options).unwrap();
        // The variant, not its wording: `zip` may reword "Duplicate filename" without changing what
        // it refuses, and a test that failed on that would be reporting nothing (Copilot on PR #273).
        // The refusal is unconditional — the writer keys its own entries by name and rejects a
        // second insert of one it already holds
        // `[src: zip 2.4.2 src/write.rs, ZipWriter::insert_file_data (MIT)]`.
        let refusal = refuses.start_file("manifest.json", options).unwrap_err();
        assert!(matches!(refusal, zip::result::ZipError::InvalidArchive(_)),
            "premise: no Cuthulhu build can write this fixture, so it is patched instead: {refusal}");

        let future = format!(r#"{{"version":99,"document":{}}}"#,
            document::Document::new().snapshot_json());
        let current = crate::manifest::write_manifest(&document::Document::new());
        let two_manifests = |path: &std::path::Path, first: &str, last: &str| {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
            zip.start_file("manifest.json", options).unwrap();
            zip.write_all(first.as_bytes()).unwrap();
            zip.start_file(PLACEHOLDER, options).unwrap();
            zip.write_all(last.as_bytes()).unwrap();
            zip.finish().unwrap();
            let raw = std::fs::read(path).unwrap();
            assert_eq!(times_named(&raw, PLACEHOLDER.as_bytes()), 2,
                "premise: the placeholder is written once in the local header and once in the \
                 central directory, and the rename below reaches both");
            std::fs::write(path,
                rename_every_member(&raw, PLACEHOLDER.as_bytes(), b"manifest.json")).unwrap();
            let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
            assert_eq!(archive.len(), 1, "the reader collapses the duplicate before any caller \
                sees it, so nothing in this workspace can tell the archive is ambiguous");
            let mut read_back = String::new();
            archive.by_name("manifest.json").unwrap().read_to_string(&mut read_back).unwrap();
            assert_eq!(read_back, last, "the member kept is the last one named");
            // The indexed view is the deduplicated one, so it is not a way around `by_name`: the one
            // index carries the surviving member and there is no second one to visit. Every parsed
            // central-directory entry is inserted into an `IndexMap` keyed on its own name, so a
            // repeated name replaces the earlier entry's *value* while keeping its position — which
            // is why the count drops and the last member is what remains
            // `[src: zip 2.4.2 src/read.rs, shared::SharedBuilder::build (MIT)]`, reported upstream
            // as zip-rs/zip2#841.
            let mut first_index = String::new();
            archive.by_index(0).unwrap().read_to_string(&mut first_index).unwrap();
            assert_eq!(first_index, last, "the one index holds the surviving member");
            assert!(archive.by_index(1).is_err(), "there is no shadowed member to reach");
        };

        let dir = tempfile::tempdir().unwrap();

        // Future first: the archive reads as the current-version project its last member is, so it
        // opens, and the guard allows the write that destroys the shadowed member with the rest of
        // the file. That is the accepted consequence, not an oversight — there is no reading under
        // which this build ever saw a project from the future here.
        let hidden_future = dir.path().join("future-then-current.cut");
        two_manifests(&hidden_future, &future, &current);
        assert_eq!(load_project(&hidden_future).unwrap(), document::Document::new());
        save_project(&hidden_future, &document::Document::new()).unwrap();
        // Not a member count: the design anticipates an `assets/` member, and this test is about the
        // duplicate rather than the format's shape (Copilot on PR #273). What it states is that the
        // ambiguity is gone and the file is an ordinary project.
        let mut replaced = zip::ZipArchive::new(std::fs::File::open(&hidden_future).unwrap()).unwrap();
        for member in ["manifest.json", "design.svg"] {
            assert!(replaced.by_name(member).is_ok(), "an ordinary save writes {member}");
        }
        drop(replaced);
        assert_eq!(times_named(&std::fs::read(&hidden_future).unwrap(), b"manifest.json"), 2,
            "one member, not two: the name is written once in the local header and once in the \
             central directory");

        // Current first, so the version claim is the one that survives: it is refused on open, and
        // the guard refuses to replace it by name and leaves the bytes alone. The invariant holds
        // in the order that can express it.
        let visible_future = dir.path().join("current-then-future.cut");
        two_manifests(&visible_future, &current, &future);
        assert!(matches!(load_project(&visible_future),
            Err(IoError::UnsupportedProjectVersion { found: 99, .. })));
        let before = std::fs::read(&visible_future).unwrap();
        assert!(matches!(save_project(&visible_future, &document::Document::new()),
            Err(IoError::UnsupportedProjectVersion { found: 99, .. })));
        assert_eq!(std::fs::read(&visible_future).unwrap(), before,
            "a refused save leaves the destination byte-identical");
    }
}

#[cfg(test)]
mod overwrite_tests {
    use super::*;
    #[test]
    fn save_twice_to_same_path_overwrites() {
        let doc = document::Document::new();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proj.cut");
        save_project(&path, &doc).unwrap();
        save_project(&path, &doc).unwrap(); // second save must overwrite, not error
        assert!(load_project(&path).is_ok());
    }
}
