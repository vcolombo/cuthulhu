// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Serialize, Deserialize};
use geometry::Rect;
use crate::history::Editor;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct MachineProfile {
    pub id: String,
    pub name: String,
    pub width_mm: f64,
    pub height_mm: f64,
}

pub fn builtin_profiles() -> Vec<MachineProfile> {
    vec![
        MachineProfile {
            id: "cameo5".into(),
            name: "Silhouette Cameo 5 Alpha".into(),
            width_mm: 330.0,
            height_mm: 3000.0,
        },
        MachineProfile {
            id: "puma".into(),
            name: "GCC Puma IV".into(),
            width_mm: 600.0,
            height_mm: 5000.0,
        },
    ]
}

impl Editor {
    pub fn set_machine(&mut self, p: MachineProfile) {
        self.doc.artboard = Rect {
            x: 0.0,
            y: 0.0,
            w: p.width_mm,
            h: p.height_mm,
        };
        self.doc.machine = Some(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_ids_are_canonical() {
        let ids: Vec<String> = builtin_profiles().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec!["cameo5", "puma"]);
    }

    #[test]
    fn set_machine_resizes_artboard() {
        let mut ed = Editor::new();
        let puma = builtin_profiles()
            .into_iter()
            .find(|p| p.id == "puma")
            .unwrap();
        ed.set_machine(puma);
        assert!(ed.doc.artboard.w > 300.0); // Puma is wide-format
    }
}
