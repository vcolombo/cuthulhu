// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::Path;
use document::{CmdError, CutLineType, Delta, Editor, MachineProfile, NodeId, PresetAssignment, ShapeKind, commands};
use fileio::IoError;
use geometry::{Affine, BoolOp};

/// Wraps the document `Editor` with thin methods, one per IPC command. Each method
/// carries the actual logic (or delegates straight to `document`/`fileio`); `ipc.rs`
/// just maps typed errors to `String` for the Tauri boundary.
pub struct AppState {
    pub editor: Editor,
}

impl AppState {
    pub fn new() -> Self {
        AppState { editor: Editor::new() }
    }

    /// Test/IPC helper: add a rect under the document root, committed as one step.
    pub fn add_rect(&mut self, w: f64, h: f64) -> NodeId {
        let d = commands::add_primitive(&mut self.editor.doc.ids, self.editor.doc.root,
            ShapeKind::Rect { w, h }).unwrap();
        let id = if let document::NodeOp::Add { node, .. } = &d.0[0] { node.id } else { unreachable!() };
        self.editor.commit(d);
        id
    }

    /// Discards the current document (and its undo history) and starts a fresh one.
    pub fn new_doc(&mut self) -> String {
        self.editor = Editor::new();
        self.snapshot()
    }

    pub fn snapshot(&self) -> String {
        self.editor.doc.snapshot_json()
    }

    pub fn commit_transform(&mut self, ids: Vec<NodeId>, m: Affine) -> Result<Delta, CmdError> {
        let d = commands::transform_nodes(&self.editor.doc, &ids, m)?;
        Ok(self.editor.commit(d))
    }

    pub fn add_primitive(&mut self, parent: NodeId, kind: ShapeKind) -> Result<Delta, CmdError> {
        let d = commands::add_primitive(&mut self.editor.doc.ids, parent, kind)?;
        Ok(self.editor.commit(d))
    }

    pub fn boolean_op(&mut self, ids: Vec<NodeId>, op: BoolOp) -> Result<Delta, CmdError> {
        self.editor.boolean(&ids, op)
    }

    pub fn add_text(&mut self, parent: NodeId, family: String, size_mm: f64, text: String) -> Result<Delta, CmdError> {
        self.editor.add_text(parent, &family, size_mm, &text)
    }

    pub fn delete(&mut self, ids: Vec<NodeId>) -> Result<Delta, CmdError> {
        let d = commands::delete_nodes(&self.editor.doc, &ids)?;
        Ok(self.editor.commit(d))
    }

    pub fn reorder(&mut self, id: NodeId, new_index: usize) -> Result<Delta, CmdError> {
        let d = commands::reorder(&self.editor.doc, id, new_index)?;
        Ok(self.editor.commit(d))
    }

    pub fn set_cut_line_type(&mut self, ids: Vec<NodeId>, value: CutLineType)
        -> Result<Delta, CmdError> {
        let d = commands::set_cut_line_type(&self.editor.doc, &ids, value)?;
        // An empty delta is a no-op the operator asked for; committing it would clear the
        // redo stack and add an undo step that does nothing.
        if d.0.is_empty() { return Ok(d); }
        Ok(self.editor.commit(d))
    }

    pub fn set_material_preset(&mut self, ids: Vec<NodeId>, value: PresetAssignment)
        -> Result<Delta, CmdError> {
        let d = commands::set_material_preset(&self.editor.doc, &ids, value)?;
        // Same rule as `set_cut_line_type`: an empty delta is a no-op the operator asked for,
        // and committing it would clear the redo stack and add an undo step that does nothing.
        if d.0.is_empty() { return Ok(d); }
        Ok(self.editor.commit(d))
    }

    pub fn undo(&mut self) -> Option<Delta> {
        self.editor.undo()
    }

    pub fn redo(&mut self) -> Option<Delta> {
        self.editor.redo()
    }

    /// Imports SVG paths under `parent`, committed as one undoable step. Returns the
    /// committed delta plus any elements the importer had to skip (unsupported nodes).
    pub fn import_svg(&mut self, bytes: Vec<u8>, parent: NodeId) -> Result<(Delta, Vec<String>), IoError> {
        let (d, skipped) = fileio::import_svg(&bytes, &mut self.editor.doc.ids, parent)?;
        Ok((self.editor.commit(d), skipped))
    }

    pub fn save_project(&self, path: &Path) -> Result<(), IoError> {
        fileio::save_project(path, &self.editor.doc)
    }

    /// Loads a project from disk, replacing the current document and undo history.
    pub fn load_project(&mut self, path: &Path) -> Result<String, IoError> {
        let doc = fileio::load_project(path)?;
        self.editor = Editor::new();
        self.editor.doc = doc;
        Ok(self.snapshot())
    }

    pub fn set_machine(&mut self, machine_id: &str) -> Result<(), CmdError> {
        let profile = document::builtin_profiles().into_iter().find(|p| p.id == machine_id)
            .ok_or(CmdError::NotFound)?;
        self.editor.set_machine(profile);
        Ok(())
    }

    pub fn list_machines(&self) -> Vec<MachineProfile> {
        document::builtin_profiles()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_state_commit_transform_moves_node() {
        let mut app = AppState::new();
        let id = app.add_rect(10.0, 10.0);
        app.commit_transform(vec![id], geometry::Affine::translate(3.0, 0.0)).unwrap();
        assert_eq!(app.editor.doc.get(id).unwrap().transform.apply(0.0, 0.0), (3.0, 0.0));
    }

    #[test]
    fn app_state_undo_reverts_last_commit() {
        let mut app = AppState::new();
        let id = app.add_rect(5.0, 5.0);
        assert!(app.editor.doc.get(id).is_some());
        app.undo();
        assert!(app.editor.doc.get(id).is_none());
        app.redo();
        assert!(app.editor.doc.get(id).is_some());
    }

    #[test]
    fn app_state_import_svg_commits_paths_under_parent() {
        let mut app = AppState::new();
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/></svg>"#;
        let root = app.editor.doc.root;
        let (_, skipped) = app.import_svg(svg.to_vec(), root).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(app.editor.doc.get(root).unwrap().children.len(), 1);
    }

    #[test]
    fn app_state_set_machine_rejects_unknown_id() {
        let mut app = AppState::new();
        assert!(app.set_machine("not-a-real-machine").is_err());
    }

    #[test]
    fn app_state_new_doc_clears_history() {
        let mut app = AppState::new();
        app.add_rect(1.0, 1.0);
        app.new_doc();
        assert!(app.undo().is_none());
    }

    /// The panel dispatches on every click, including the one that re-picks the value the
    /// selection already carries. Committing that empty delta would clear the redo stack and
    /// leave an undo step that undoes nothing, so the method must decline to commit it.
    #[test]
    fn app_state_set_cut_line_type_no_op_keeps_redo_stack() {
        let mut app = AppState::new();
        let id = app.add_rect(1.0, 1.0);
        app.add_rect(2.0, 2.0);
        app.undo();

        let d = app.set_cut_line_type(vec![id], CutLineType::Cut).unwrap();
        assert!(d.0.is_empty(), "premise: the rect already cuts");
        assert!(app.redo().is_some(), "a no-op must not throw away redoable work");
    }
}
