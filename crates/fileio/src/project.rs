// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::Path;
use std::io::{Write, Read};
use document::Document;
use crate::IoError;

/// Write `manifest.json` (the source of truth: a `{ version, document }` envelope, see
/// `crate::manifest`) + `design.svg` (interchange copy) into a zip container at `path`,
/// atomically: build in a temp file in the same directory, then rename over the destination.
///
/// Refuses up front if `path` already holds a project this build cannot read.
pub fn save_project(path: &Path, doc: &Document) -> Result<(), IoError> {
    refuse_overwriting_a_newer_project(path)?;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| IoError::Io(e.to_string()))?;
    let mut zip = zip::ZipWriter::new(tmp.reopen().map_err(|e| IoError::Io(e.to_string()))?);
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", opts).map_err(|e| IoError::Io(e.to_string()))?;
    zip.write_all(crate::manifest::write_manifest(doc).as_bytes())
        .map_err(|e| IoError::Io(e.to_string()))?;
    zip.start_file("design.svg", opts).map_err(|e| IoError::Io(e.to_string()))?;
    zip.write_all(crate::doc_to_svg(doc).as_bytes()).map_err(|e| IoError::Io(e.to_string()))?;
    zip.finish().map_err(|e| IoError::Io(e.to_string()))?;
    tmp.persist(path).map_err(|e| IoError::Io(e.to_string()))?;   // atomic rename
    Ok(())
}

pub fn load_project(path: &Path) -> Result<Document, IoError> {
    let file = std::fs::File::open(path).map_err(|e| IoError::Io(e.to_string()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| IoError::Parse(e.to_string()))?;
    let mut s = String::new();
    zip.by_name("manifest.json").map_err(|e| IoError::Parse(e.to_string()))?
        .read_to_string(&mut s).map_err(|e| IoError::Io(e.to_string()))?;
    crate::manifest::read_manifest(&s)
}

/// The one way a project this build refused to open can still be destroyed is a Save As aimed
/// back at it: `AppState` keeps no path for a load that failed, but the file picker will happily
/// offer the file again. So the refusal lives at the write itself rather than in a caller's
/// bookkeeping, which also covers any future non-desktop writer.
///
/// It fails *open* on anything it can positively identify as not a project — an absent file,
/// bytes that are not a zip, an archive with no `manifest.json`, a manifest that is not text.
/// The guard exists to protect a project this build admits it cannot read, not to make saving
/// over an arbitrary file impossible.
///
/// A destination that *exists and cannot be inspected* is neither: an I/O failure is not
/// evidence that the file is not a project, so it fails **closed**. Reading a file the operator
/// cannot read is unusual; replacing one on the strength of a check that never ran is worse.
fn refuse_overwriting_a_newer_project(path: &Path) -> Result<(), IoError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(IoError::Io(e.to_string())),
    };
    let mut zip = match zip::ZipArchive::new(file) {
        Ok(zip) => zip,
        Err(zip::result::ZipError::Io(e)) => return Err(IoError::Io(e.to_string())),
        Err(_) => return Ok(()),
    };
    let mut s = String::new();
    match zip.by_name("manifest.json") {
        Ok(mut member) => match member.read_to_string(&mut s) {
            Ok(_) => {}
            // Bytes that are not UTF-8 are not a manifest this build ever wrote.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => return Ok(()),
            Err(e) => return Err(IoError::Io(e.to_string())),
        },
        Err(zip::result::ZipError::Io(e)) => return Err(IoError::Io(e.to_string())),
        Err(_) => return Ok(()),
    }
    match crate::manifest::probe_version(&s) {
        Ok(found) if found > crate::manifest::MANIFEST_VERSION => {
            Err(IoError::UnsupportedProjectVersion {
                found,
                supported: crate::manifest::MANIFEST_VERSION,
            })
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let mut doc = load_project(&path).unwrap();
        assert_eq!(doc.machine.as_ref().unwrap().id, "puma", "the legacy step ran");
        let root = doc.get(doc.root).unwrap();
        assert_eq!(root.kind, document::NodeKind::Layer);
        assert_eq!(root.children.len(), 1);
        let group = doc.get(root.children[0]).unwrap();
        assert_eq!(group.kind, document::NodeKind::Group);
        assert_eq!(group.transform.apply(0.0, 0.0), (3.0, 4.0));
        let rect = doc.get(group.children[0]).unwrap();
        assert_eq!(rect.kind,
            document::NodeKind::Shape(document::ShapeKind::Rect { w: 10.0, h: 20.0 }));
        assert_eq!(rect.style.stroke, Some(0xFF0000FF));
        assert_eq!(rect.cut_line_type, document::CutLineType::Cut);
        assert_eq!(rect.material_preset, document::PresetAssignment::Inherit);
        assert_eq!((doc.artboard.w, doc.artboard.h), (330.0, 3000.0));
        assert_eq!(doc.ids.next(), document::NodeId(4),
            "the id generator resumed where the file left it, so a new node cannot collide");
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
