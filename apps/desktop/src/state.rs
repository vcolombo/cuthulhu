// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::Path;
use document::{commands, CmdError, Delta, Editor, MachineProfile, NodeId, ShapeKind};
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
}
