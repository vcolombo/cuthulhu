// SPDX-License-Identifier: GPL-3.0-or-later
use crate::delta::Document;

impl Document {
    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Editor;
    use crate::node::ShapeKind;

    #[test]
    fn snapshot_round_trips_through_json() {
        let mut ed = Editor::new();
        let root = ed.doc.root;
        let d = crate::commands::add_primitive(&mut ed.doc.ids, root,
            ShapeKind::Rect { w: 2.0, h: 2.0 }).unwrap();
        ed.commit(d);
        let json = ed.doc.snapshot_json();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ed.doc);
    }
}
