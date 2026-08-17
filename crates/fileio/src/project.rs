// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::Path;
use std::io::{Write, Read};
use document::Document;
use crate::IoError;

/// Write `manifest.json` (the source of truth) + `design.svg` (interchange copy)
/// into a zip container at `path`, atomically: build in a temp file in the same
/// directory, then rename over the destination.
pub fn save_project(path: &Path, doc: &Document) -> Result<(), IoError> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| IoError::Io(e.to_string()))?;
    let mut zip = zip::ZipWriter::new(tmp.reopen().map_err(|e| IoError::Io(e.to_string()))?);
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", opts).map_err(|e| IoError::Io(e.to_string()))?;
    zip.write_all(doc.snapshot_json().as_bytes()).map_err(|e| IoError::Io(e.to_string()))?;
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
    let mut doc: Document = serde_json::from_str(&s).map_err(|e| IoError::Parse(e.to_string()))?;
    if let Some(m) = doc.machine.as_mut() {
        m.id = match m.id.as_str() {
            "cameo5_alpha" => "cameo5".into(),
            "puma_iv" => "puma".into(),
            _ => std::mem::take(&mut m.id),
        };
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_machine_ids_migrate_on_load() {
        let mut doc = document::Document::new();
        let legacy = document::MachineProfile { id: "puma_iv".into(), name: "GCC Puma IV".into(),
            width_mm: 600.0, height_mm: 5000.0 };
        doc.machine = Some(legacy);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.cut");
        save_project(&path, &doc).unwrap();
        let back = load_project(&path).unwrap();
        assert_eq!(back.machine.unwrap().id, "puma");
    }

    /// The migration through a real project file, which is the level it matters at:
    /// `legacy_machine_ids_migrate_on_load` above could plant a legacy *value* in a live
    /// `Document`, but an absent field cannot be planted that way — `save_project` always
    /// writes it. So the manifest is pruned rather than hand-written: everything except
    /// `cut_line_type` is exactly what `save_project` emits today, so the fixture cannot
    /// drift from `Document`'s real shape.
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
