// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::fs;
use std::io::Write;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterialPreset {
    pub id: String,
    pub name: String,
    pub machine_id: String,
    pub settings: PresetSettings,
    pub builtin: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresetSettings {
    pub speed: Option<u32>,
    pub force: Option<u32>,
    pub repeat_count: u32,
}

#[derive(Debug, PartialEq)]
pub enum PresetError {
    Corrupt(String),
    UnknownVersion(u32),
    Io(String),
}

pub fn builtin_presets() -> Vec<MaterialPreset> {
    vec![
        // Cameo 5 presets
        MaterialPreset {
            id: "cameo5-cardstock-medium".into(),
            name: "Cardstock (Medium)".into(),
            machine_id: "cameo5".into(),
            settings: PresetSettings {
                speed: Some(5),
                force: Some(20),
                repeat_count: 1,
            },
            builtin: true,
        },
        MaterialPreset {
            id: "cameo5-vinyl-adhesive".into(),
            name: "Vinyl (Adhesive)".into(),
            machine_id: "cameo5".into(),
            settings: PresetSettings {
                speed: Some(8),
                force: Some(10),
                repeat_count: 1,
            },
            builtin: true,
        },
        MaterialPreset {
            id: "cameo5-htv".into(),
            name: "HTV".into(),
            machine_id: "cameo5".into(),
            settings: PresetSettings {
                speed: Some(8),
                force: Some(12),
                repeat_count: 1,
            },
            builtin: true,
        },
        MaterialPreset {
            id: "cameo5-copy-paper".into(),
            name: "Copy Paper".into(),
            machine_id: "cameo5".into(),
            settings: PresetSettings {
                speed: Some(10),
                force: Some(8),
                repeat_count: 1,
            },
            builtin: true,
        },
        MaterialPreset {
            id: "cameo5-cardboard-thin".into(),
            name: "Cardboard (Thin)".into(),
            machine_id: "cameo5".into(),
            settings: PresetSettings {
                speed: Some(3),
                force: Some(30),
                repeat_count: 1,
            },
            builtin: true,
        },
        // Puma presets (panel-set: speed/force None)
        MaterialPreset {
            id: "puma-cardstock-medium".into(),
            name: "Cardstock (Medium)".into(),
            machine_id: "puma".into(),
            settings: PresetSettings {
                speed: None,
                force: None,
                repeat_count: 1,
            },
            builtin: true,
        },
        MaterialPreset {
            id: "puma-vinyl-adhesive".into(),
            name: "Vinyl (Adhesive)".into(),
            machine_id: "puma".into(),
            settings: PresetSettings {
                speed: None,
                force: None,
                repeat_count: 1,
            },
            builtin: true,
        },
        MaterialPreset {
            id: "puma-htv".into(),
            name: "HTV".into(),
            machine_id: "puma".into(),
            settings: PresetSettings {
                speed: None,
                force: None,
                repeat_count: 1,
            },
            builtin: true,
        },
        MaterialPreset {
            id: "puma-copy-paper".into(),
            name: "Copy Paper".into(),
            machine_id: "puma".into(),
            settings: PresetSettings {
                speed: None,
                force: None,
                repeat_count: 1,
            },
            builtin: true,
        },
        MaterialPreset {
            id: "puma-cardboard-thin".into(),
            name: "Cardboard (Thin)".into(),
            machine_id: "puma".into(),
            settings: PresetSettings {
                speed: None,
                force: None,
                repeat_count: 1,
            },
            builtin: true,
        },
    ]
}

pub fn load_presets(user_file: &Path) -> Result<Vec<MaterialPreset>, PresetError> {
    let mut all_presets = builtin_presets();

    // Try to load user presets if file exists
    if user_file.exists() {
        let content = fs::read_to_string(user_file)
            .map_err(|e| PresetError::Io(e.to_string()))?;

        // Check version FIRST before parsing full schema (allows future schema changes)
        let value: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| PresetError::Corrupt(e.to_string()))?;

        let version = value
            .get("version")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| PresetError::Corrupt("missing or invalid version field".into()))?;

        if version != 1 {
            return Err(PresetError::UnknownVersion(version as u32));
        }

        // Now parse full schema
        #[derive(Deserialize)]
        struct FileFormat {
            version: u32,
            presets: Vec<MaterialPreset>,
        }

        let file_data: FileFormat = serde_json::from_str(&content)
            .map_err(|e| PresetError::Corrupt(e.to_string()))?;

        // Force builtin: false on all user entries (on-disk contract is user-entries-only)
        let mut user_presets = file_data.presets;
        for preset in &mut user_presets {
            preset.builtin = false;
        }

        // Remove builtin presets that are shadowed by user presets
        let user_ids: std::collections::HashSet<_> =
            user_presets.iter().map(|p| &p.id).collect();
        all_presets.retain(|p| !user_ids.contains(&p.id));

        // Add user presets
        all_presets.extend(user_presets);
    }

    Ok(all_presets)
}

pub fn save_user_presets(user_file: &Path, user: &[MaterialPreset]) -> Result<(), PresetError> {
    let dir = user_file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| PresetError::Io(e.to_string()))?;

    #[derive(Serialize)]
    struct FileFormat {
        version: u32,
        presets: Vec<MaterialPreset>,
    }

    let file_data = FileFormat {
        version: 1,
        presets: user.to_vec(),
    };

    let json = serde_json::to_string_pretty(&file_data)
        .map_err(|e| PresetError::Io(e.to_string()))?;

    let mut file = tmp.reopen().map_err(|e| PresetError::Io(e.to_string()))?;
    file.write_all(json.as_bytes())
        .map_err(|e| PresetError::Io(e.to_string()))?;

    tmp.persist(user_file)
        .map_err(|e| PresetError::Io(e.to_string()))?;

    Ok(())
}

pub fn default_presets_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("cuthulhu").join("presets.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_entry_shadows_builtin_and_delete_reveals_it() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");

        // Save a user preset with same ID as a builtin
        let user_presets = vec![MaterialPreset {
            id: "cameo5-cardstock-medium".into(),
            name: "Cardstock (Heavy)".into(), // Custom name
            machine_id: "cameo5".into(),
            settings: PresetSettings {
                speed: Some(3),
                force: Some(25),
                repeat_count: 2,
            },
            builtin: false,
        }];

        save_user_presets(&user_file, &user_presets).unwrap();

        // Load and verify user preset shadows builtin
        let loaded = load_presets(&user_file).unwrap();
        let cardstock = loaded
            .iter()
            .find(|p| p.id == "cameo5-cardstock-medium")
            .unwrap();

        assert_eq!(cardstock.name, "Cardstock (Heavy)");
        assert_eq!(cardstock.settings.speed, Some(3));
        assert_eq!(cardstock.settings.force, Some(25));
        assert_eq!(cardstock.settings.repeat_count, 2);
        assert!(!cardstock.builtin);

        // Count how many cameo5-cardstock-medium are in loaded (should be 1)
        let count = loaded
            .iter()
            .filter(|p| p.id == "cameo5-cardstock-medium")
            .count();
        assert_eq!(count, 1);

        // Delete user preset by saving empty list
        save_user_presets(&user_file, &[]).unwrap();

        // Load and verify builtin is revealed
        let loaded_after = load_presets(&user_file).unwrap();
        let builtin_cardstock = loaded_after
            .iter()
            .find(|p| p.id == "cameo5-cardstock-medium")
            .unwrap();

        assert_eq!(builtin_cardstock.name, "Cardstock (Medium)"); // builtin name
        assert_eq!(builtin_cardstock.settings.speed, Some(5)); // builtin values
        assert_eq!(builtin_cardstock.settings.force, Some(20));
        assert!(builtin_cardstock.builtin);
    }

    #[test]
    fn corrupt_and_unknown_version_files_error_without_clobbering() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");

        // Test 1: Write garbage and verify it errors with Corrupt
        fs::write(&user_file, "not valid json").unwrap();
        let original_content = fs::read_to_string(&user_file).unwrap();

        let result = load_presets(&user_file);
        assert!(matches!(result, Err(PresetError::Corrupt(_))));

        // Verify file was not clobbered
        let content_after = fs::read_to_string(&user_file).unwrap();
        assert_eq!(content_after, original_content);

        // Test 2: Write unknown version (no presets field) and verify it errors as UnknownVersion
        // This tests that version check runs FIRST, before parsing full schema
        fs::write(&user_file, r#"{"version": 99}"#).unwrap();
        let original_content = fs::read_to_string(&user_file).unwrap();

        let result = load_presets(&user_file);
        assert_eq!(result, Err(PresetError::UnknownVersion(99)));

        // Verify file was not clobbered
        let content_after = fs::read_to_string(&user_file).unwrap();
        assert_eq!(content_after, original_content);

        // Test 3: Unknown version with presets field present
        fs::write(&user_file, r#"{"version": 99, "presets": []}"#).unwrap();
        let original_content = fs::read_to_string(&user_file).unwrap();

        let result = load_presets(&user_file);
        assert_eq!(result, Err(PresetError::UnknownVersion(99)));

        // Verify file was not clobbered
        let content_after = fs::read_to_string(&user_file).unwrap();
        assert_eq!(content_after, original_content);
    }

    #[test]
    fn save_is_atomic_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");

        let user_presets = vec![MaterialPreset {
            id: "custom-material".into(),
            name: "Custom".into(),
            machine_id: "cameo5".into(),
            settings: PresetSettings {
                speed: Some(7),
                force: Some(15),
                repeat_count: 3,
            },
            builtin: false,
        }];

        // Save user presets
        save_user_presets(&user_file, &user_presets).unwrap();

        // Load and verify merge (builtin + user)
        let loaded = load_presets(&user_file).unwrap();

        // Should have all builtins plus the custom one
        let num_builtins = builtin_presets().len();
        assert_eq!(loaded.len(), num_builtins + 1);

        // Find and verify the custom preset
        let custom = loaded
            .iter()
            .find(|p| p.id == "custom-material")
            .unwrap();
        assert_eq!(custom.name, "Custom");
        assert_eq!(custom.settings.speed, Some(7));
        assert_eq!(custom.settings.force, Some(15));
        assert_eq!(custom.settings.repeat_count, 3);
        assert!(!custom.builtin);

        // Verify at least one builtin is present
        let has_builtin = loaded.iter().any(|p| p.builtin);
        assert!(has_builtin);
    }

    #[test]
    fn builtins_cover_both_machines_with_valid_ranges() {
        let builtins = builtin_presets();

        // Collect by machine
        let cameo5_presets: Vec<_> =
            builtins.iter().filter(|p| p.machine_id == "cameo5").collect();
        let puma_presets: Vec<_> =
            builtins.iter().filter(|p| p.machine_id == "puma").collect();

        // Both machines should have at least 4 presets
        assert!(cameo5_presets.len() >= 4, "cameo5 has < 4 presets");
        assert!(puma_presets.len() >= 4, "puma has < 4 presets");

        // All should have machine_id in {cameo5, puma}
        for preset in &builtins {
            assert!(
                preset.machine_id == "cameo5" || preset.machine_id == "puma",
                "invalid machine_id: {}",
                preset.machine_id
            );
            assert!(preset.builtin, "builtin preset marked as non-builtin");
        }

        // All should have repeat_count in 1..=10
        for preset in &builtins {
            assert!(
                preset.settings.repeat_count >= 1 && preset.settings.repeat_count <= 10,
                "repeat_count out of range: {}",
                preset.settings.repeat_count
            );
        }

        // Cameo5 presets must have speed and force (not None)
        for preset in &cameo5_presets {
            assert!(
                preset.settings.speed.is_some(),
                "cameo5 preset {} missing speed",
                preset.id
            );
            assert!(
                preset.settings.force.is_some(),
                "cameo5 preset {} missing force",
                preset.id
            );
        }

        // Puma presets must have speed and force set to None (panel-set)
        for preset in &puma_presets {
            assert_eq!(
                preset.settings.speed, None,
                "puma preset {} should have speed=None",
                preset.id
            );
            assert_eq!(
                preset.settings.force, None,
                "puma preset {} should have force=None",
                preset.id
            );
        }
    }

    #[test]
    fn default_presets_path_ends_with_cuthulhu_presets_json() {
        if let Some(path) = default_presets_path() {
            assert!(path.ends_with("cuthulhu/presets.json"));
        }
    }
}
