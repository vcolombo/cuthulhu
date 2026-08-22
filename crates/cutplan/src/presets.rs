// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::Settings;
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

/// The only on-disk format this build reads or writes. Named once so the check in
/// `load_presets`, the header `save_user_presets` writes and the sentence `UnknownVersion`
/// reads cannot drift apart.
const PRESETS_VERSION: u32 = 1;

/// Every way the presets file refuses to be read or written.
///
/// The desktop is the only caller — no CLI or Cut Host path reads or writes presets — and it
/// turns each of these straight into an `IpcError`, so the sentences and codes below are what
/// an operator reads and what a dialog can branch on (#278).
///
/// Reading and writing are separate variants rather than one `Io` because they are separate
/// facts with separate answers: an unreadable file means no material is available to cut with,
/// an unwritable one means an edit did not land while everything already saved is untouched.
/// One variant could only have said "read or written", which is vaguer at both of the places
/// the desktop shows it.
#[derive(Debug, PartialEq)]
pub enum PresetError {
    /// The file's bytes arrived and did not make sense. `Display` forwards the payload
    /// verbatim, so every site that builds one owes a finished sentence: the three that exist
    /// say three different things — not JSON at all, no version stated, no usable presets list
    /// — and no single wrapper could say all three.
    Corrupt(String),
    /// A version other than `PRESETS_VERSION`. Carries what was found; what this build reads is
    /// a build constant, so `Display` takes it from there rather than from the payload.
    ///
    /// `u64`, the width the probe reads, and not `u32`: narrowing it made the sentence
    /// contradict itself, because `4294967297 as u32` is `1` and the refusal then read
    /// "presets version 1; this build reads 1" (CodeRabbit, Copilot and Greptile on PR #280).
    UnknownVersion(u64),
    /// `load_presets` could not get the file's bytes. The payload is the OS diagnostic and
    /// `Display` wraps it: one site raises this, so one wrapper covers it.
    Unreadable(String),
    /// `save_user_presets` did not land the file. The payload is the OS or serde diagnostic and
    /// `Display` wraps it: five sites raise this — the directory, the temp file, the encode,
    /// the write and the rename — and every one of them leaves the file unwritten.
    Unwritable(String),
}

/// Every refusal in the words an operator reads.
///
/// Two of the four wrap their payload here and two do not, and which is which follows from the
/// sites. `Unreadable` and `Unwritable` state one fact at every site that raises them, so one
/// wrapper says it once and the payload stays the raw diagnostic it is. `Corrupt` states a
/// different fact at each of its three, so the sentence is written there and forwarded — a
/// wrapper in front of one would read twice, the reason #90 dropped `"preflight: "`.
impl std::fmt::Display for PresetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PresetError::Corrupt(message) => write!(f, "{message}"),
            // Not "written by a newer Cuthulhu", which would be false of the other direction:
            // the check below is `!= PRESETS_VERSION`, so a hand-edited `"version": 0` lands
            // here too. Naming both numbers says which way it went without claiming it.
            PresetError::UnknownVersion(found) => write!(
                f,
                "this presets file is in a format this build does not read \
                 (presets version {found}; this build reads {PRESETS_VERSION})"
            ),
            PresetError::Unreadable(message) =>
                write!(f, "the presets file could not be read ({message})"),
            PresetError::Unwritable(message) =>
                write!(f, "the presets file could not be written ({message})"),
        }
    }
}
impl std::error::Error for PresetError {}

impl PresetError {
    /// Stable identifier for a caller that must branch on the *kind* of refusal instead of
    /// matching the text of one. The desktop turns it straight into an `IpcError` code, and an
    /// `IpcError` code survives to the frontend, so offering an update check on a version this
    /// build cannot read and a path to the file on a corrupt one is a UI choice rather than a
    /// backend change. Same shape as `PreflightError::code` and `DeviceError::code`.
    ///
    /// Plural, unlike the desktop's own `unknown_preset` and `invalid_preset`: these name the
    /// presets file's refusals, not one material's.
    pub fn code(&self) -> &'static str {
        match self {
            PresetError::Corrupt(_) => "presets_corrupt",
            PresetError::UnknownVersion(_) => "presets_unknown_version",
            PresetError::Unreadable(_) => "presets_unreadable",
            PresetError::Unwritable(_) => "presets_unwritable",
        }
    }
}

/// Per-pass settings a caller wants applied on top of whatever preset is
/// selected. Every field is optional: `None` means "defer to the preset".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsOverride {
    pub speed: Option<u32>,
    pub force: Option<u32>,
    pub repeat_count: Option<u32>,
}

/// Override fields win over the preset's; with neither, `Settings::default()`
/// (repeat_count 1, no speed or force).
pub fn resolve_settings(preset: Option<&MaterialPreset>, override_: &SettingsOverride) -> Settings {
    Settings {
        speed: override_.speed.or_else(|| preset.and_then(|p| p.settings.speed)),
        force: override_.force.or_else(|| preset.and_then(|p| p.settings.force)),
        repeat_count: override_
            .repeat_count
            .or_else(|| preset.map(|p| p.settings.repeat_count))
            .unwrap_or(1),
    }
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

    // `read`, not `exists()` and then a read: `Path::exists` answers false when the file cannot
    // be stat'd at all — a config directory the process may not search, say — so the operator's
    // whole saved list vanished and every caller carried on as if they had never saved one. Only
    // a file that is genuinely absent is the first-run case (Codex on PR #280).
    let found = match fs::read(user_file) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(PresetError::Unreadable(e.to_string())),
    };

    if let Some(bytes) = found {
        // Bytes that are not text are damage, not a read that failed: `read_to_string` folded
        // the two together, so a `presets.json` an editor saved in Latin-1 reported a disk fault
        // and sent the operator to check permissions (Codex on PR #280).
        let content = String::from_utf8(bytes)
            .map_err(|_| PresetError::Corrupt("the presets file is not UTF-8 text".into()))?;

        // Check version FIRST before parsing full schema (allows future schema changes)
        let value: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| PresetError::Corrupt(
                format!("the presets file is not valid JSON ({e})")))?;

        // Absent, not a number, not whole, negative, or past what a `u64` holds — all of them
        // leave this build with nothing to check the format against, which is what the sentence
        // has to say. "Usable" rather than plain "whole-number" because the last of those *is* a
        // whole number, just not one this can read (Codex on PR #280).
        let version = value
            .get("version")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| PresetError::Corrupt(
                "the presets file does not state a usable whole-number version, so this build \
                 cannot tell what format it is in".into()))?;

        if version != PRESETS_VERSION as u64 {
            return Err(PresetError::UnknownVersion(version));
        }

        // Now parse full schema. Version was already validated from the Value probe above, so
        // the full parse only needs the presets — which is also why this refusal can say the
        // presets are the part that did not read, rather than repeating "not valid JSON".
        #[derive(Deserialize)]
        struct FileFormat {
            presets: Vec<MaterialPreset>,
        }

        let file_data: FileFormat = serde_json::from_str(&content)
            .map_err(|e| PresetError::Corrupt(
                format!("the presets file does not hold a usable list of material presets ({e})")))?;

        // Force builtin: false on all user entries (on-disk contract is user-entries-only)
        let mut user_presets = file_data.presets;
        for preset in &mut user_presets {
            preset.builtin = false;
        }
        // An entry with no id names no material: nothing can select it deliberately, and letting
        // one in means a pass keyed `preset:` resolves to real speed and force by accident. The
        // file is hand-editable, so this is the boundary that keeps the state out — dropped
        // rather than refused, because one malformed entry should not cost an operator the rest
        // of their presets.
        user_presets.retain(|p| !p.id.is_empty());

        // A preset is machine-scoped: its speed and force mean nothing on another cutter, so a
        // user entry replaces a builtin only when both fields match. Keyed on the id alone, an
        // entry named with another machine's builtin id deleted that builtin from the loaded set
        // — and a user id is the operator's own string, so `my-vinyl` legitimately exists for a
        // Cameo and a Puma (#153).
        let user_keys: std::collections::HashSet<(&str, &str)> =
            user_presets.iter().map(|p| (p.machine_id.as_str(), p.id.as_str())).collect();
        all_presets.retain(|p| !user_keys.contains(&(p.machine_id.as_str(), p.id.as_str())));

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

    // First-run: the default path lives under config_dir()/cuthulhu/, which doesn't
    // exist on a fresh install, and NamedTempFile::new_in requires it to.
    std::fs::create_dir_all(dir).map_err(|e| PresetError::Unwritable(e.to_string()))?;

    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| PresetError::Unwritable(e.to_string()))?;

    #[derive(Serialize)]
    struct FileFormat {
        version: u32,
        presets: Vec<MaterialPreset>,
    }

    let file_data = FileFormat {
        version: PRESETS_VERSION,
        presets: user.to_vec(),
    };

    // Serializing `String`s and numbers cannot fail, so this arm is unreachable in practice —
    // but if it ever fired the file would still be the thing that did not get written, which is
    // what `Unwritable` says.
    let json = serde_json::to_string_pretty(&file_data)
        .map_err(|e| PresetError::Unwritable(e.to_string()))?;

    // Write through the temp file's own handle: a reopen()'d second handle held
    // across persist() can make the atomic rename fail on Windows.
    tmp.as_file_mut()
        .write_all(json.as_bytes())
        .map_err(|e| PresetError::Unwritable(e.to_string()))?;

    // `PersistError`'s own `Display` prefixes the OS string with temp-file plumbing the operator
    // has no use for, so the payload is the io error it wraps.
    tmp.persist(user_file)
        .map_err(|e| PresetError::Unwritable(e.error.to_string()))?;

    Ok(())
}

pub fn default_presets_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("cuthulhu").join("presets.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table, in the shape of `passes.rs`'s `every_plan_refusal_has_a_sentence`. A new
    /// variant fails to compile `Display` and `code`; a reworded or re-coded one fails here.
    ///
    /// `Corrupt`'s sentence column is `None` on purpose: `Display` forwards that payload
    /// verbatim, so a row here would compare a literal with itself and pass however the three
    /// real sites are worded. Those three are pinned where they are built, by the
    /// construction-path tests below. The other three variants compose their sentence here, so
    /// a row is the whole contract for every site that raises them.
    #[test]
    fn every_preset_refusal_has_a_sentence_and_a_code() {
        let cases: Vec<(PresetError, &str, Option<&str>)> = vec![
            (PresetError::Corrupt("whatever the site wrote".into()), "presets_corrupt", None),
            (
                PresetError::UnknownVersion(3),
                "presets_unknown_version",
                Some("this presets file is in a format this build does not read \
                      (presets version 3; this build reads 1)"),
            ),
            (
                PresetError::Unreadable("Permission denied (os error 13)".into()),
                "presets_unreadable",
                Some("the presets file could not be read (Permission denied (os error 13))"),
            ),
            (
                PresetError::Unwritable("No space left on device (os error 28)".into()),
                "presets_unwritable",
                Some("the presets file could not be written (No space left on device (os error 28))"),
            ),
        ];
        for (error, code, sentence) in cases {
            assert_eq!(error.code(), code, "{error:?}");
            if let Some(sentence) = sentence {
                assert_eq!(error.to_string(), sentence, "{error:?}");
            }
        }
    }

    #[test]
    fn a_presets_file_that_is_not_json_is_refused_in_words() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");
        std::fs::write(&user_file, "half a file, truncated mid-").unwrap();

        let err = load_presets(&user_file).unwrap_err();
        assert_eq!(err.code(), "presets_corrupt");
        assert!(
            err.to_string().starts_with("the presets file is not valid JSON ("),
            "got {err}"
        );
    }

    /// The issue's own scenario: a hand-edited file with the header dropped. Reaches the
    /// `ok_or_else` in `load_presets` rather than asserting the literal against itself.
    #[test]
    fn a_presets_file_with_no_version_is_refused_in_words() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");
        std::fs::write(&user_file, r#"{"presets":[]}"#).unwrap();

        let err = load_presets(&user_file).unwrap_err();
        assert_eq!(err.code(), "presets_corrupt");
        assert_eq!(
            err.to_string(),
            "the presets file does not state a usable whole-number version, so this build \
             cannot tell what format it is in"
        );

        // Every other way the probe returns `None` takes the same branch, so the sentence has to
        // be true of all of them. The last is a whole number and still unusable, which is why it
        // does not say plain "whole-number".
        for unusable in [r#""one""#, "-1", "1.5", "18446744073709551616"] {
            std::fs::write(&user_file, format!(r#"{{"version":{unusable},"presets":[]}}"#)).unwrap();
            assert_eq!(load_presets(&user_file).unwrap_err(), err, "version {unusable}");
        }
    }

    /// Valid JSON, a version this build reads, and a `presets` field that is not a list of
    /// presets — the third `Corrupt` site, which says something the other two do not.
    #[test]
    fn a_presets_file_with_no_usable_list_is_refused_in_words() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");
        std::fs::write(&user_file, r#"{"version":1,"presets":"not a list"}"#).unwrap();

        let err = load_presets(&user_file).unwrap_err();
        assert_eq!(err.code(), "presets_corrupt");
        assert!(
            err.to_string()
                .starts_with("the presets file does not hold a usable list of material presets ("),
            "got {err}"
        );
    }

    /// A version refusal must beat the schema parse below it: a file this build cannot read the
    /// format of has not been shown to be damaged, and telling the operator to repair it would
    /// send them to edit a file a newer build wrote correctly.
    ///
    /// `corrupt_and_unknown_version_files_error_without_clobbering` reaches the same branch, but
    /// with a `presets` field that is absent or empty — parseable either way, so nothing there
    /// would notice the two checks swapping. This one puts a payload under the version that the
    /// parse below would reject.
    #[test]
    fn a_version_this_build_does_not_read_is_refused_before_its_contents_are_judged() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");
        std::fs::write(&user_file, r#"{"version":3,"presets":"not a list either"}"#).unwrap();

        let err = load_presets(&user_file).unwrap_err();
        assert_eq!(err, PresetError::UnknownVersion(3));
        assert_eq!(err.code(), "presets_unknown_version");
        assert_eq!(
            err.to_string(),
            "this presets file is in a format this build does not read \
             (presets version 3; this build reads 1)"
        );
    }

    /// The version the refusal names is the version on disk, whatever its width. Carried as a
    /// `u32` it was truncated, so a file saying `4294967297` was refused with "presets version
    /// 1; this build reads 1" — a sentence contradicting itself while naming the one version
    /// this build does read (CodeRabbit, Copilot and Greptile on PR #280).
    #[test]
    fn a_version_too_large_for_a_u32_is_named_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");
        let truncates_to_one = u32::MAX as u64 + 2;
        std::fs::write(&user_file, format!(r#"{{"version":{truncates_to_one},"presets":[]}}"#))
            .unwrap();

        let err = load_presets(&user_file).unwrap_err();
        assert_eq!(err, PresetError::UnknownVersion(truncates_to_one));
        assert_eq!(
            err.to_string(),
            "this presets file is in a format this build does not read \
             (presets version 4294967297; this build reads 1)"
        );
    }

    /// The `Unreadable` site, reached for real. A directory at the file's path fails the read —
    /// which no permission bit can be relied on to do, since a test running as root reads a
    /// 0o000 file anyway.
    ///
    /// The second half is what `Path::exists` used to swallow: a read that fails without the
    /// file being absent must refuse, not answer with the builtins as though the operator had
    /// never saved anything. `exists()` is false for both, which is why it could not tell them
    /// apart (Codex on PR #280).
    #[test]
    fn a_presets_file_whose_bytes_cannot_be_read_is_refused_in_words() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");
        std::fs::create_dir(&user_file).unwrap();

        let err = load_presets(&user_file).unwrap_err();
        assert_eq!(err.code(), "presets_unreadable");
        assert!(
            err.to_string().starts_with("the presets file could not be read ("),
            "got {err}"
        );

        // The parent is an ordinary file, so the read fails and the path does not exist.
        let blocker = dir.path().join("cuthulhu");
        std::fs::write(&blocker, "not a directory").unwrap();
        let behind_a_file = load_presets(&blocker.join("presets.json")).unwrap_err();
        assert_eq!(behind_a_file.code(), "presets_unreadable", "got {behind_a_file}");

        // And a file that is genuinely absent is still the first run, not a refusal.
        let fresh = load_presets(&dir.path().join("never-saved.json")).expect("first run loads");
        assert_eq!(fresh, builtin_presets());
    }

    /// Bytes that are not text are damage, not a read that failed. `read_to_string` reported a
    /// `presets.json` an editor had saved in Latin-1 as a disk fault, sending the operator to
    /// check permissions on a file whose problem was its contents (Codex on PR #280).
    #[test]
    fn a_presets_file_that_is_not_text_is_refused_as_damaged() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");
        // `Caf\xe9` in Latin-1, inside an otherwise well-formed file.
        let mut bytes = br#"{"version":1,"presets":[{"name":"Caf"#.to_vec();
        bytes.extend_from_slice(&[0xE9]);
        bytes.extend_from_slice(br#""}]}"#);
        std::fs::write(&user_file, bytes).unwrap();

        let err = load_presets(&user_file).unwrap_err();
        assert_eq!(err.code(), "presets_corrupt", "got {err}");
        assert_eq!(err.to_string(), "the presets file is not UTF-8 text");
    }

    /// `Unwritable` through `create_dir_all`, the first of the five sites the save path can fail
    /// at: the parent is an ordinary file, so the directory cannot be made. The other four
    /// (the temp file, the encode, the write, the rename) need a full disk or a mid-flight
    /// unmount to force, and do not need forcing here — `Display` wraps this variant rather
    /// than forwarding it, so one sentence covers all five and the table above pins it.
    #[test]
    fn a_presets_file_that_cannot_be_written_is_refused_in_words() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("cuthulhu");
        std::fs::write(&blocker, "not a directory").unwrap();

        let err = save_user_presets(&blocker.join("presets.json"), &[]).unwrap_err();
        assert_eq!(err.code(), "presets_unwritable");
        assert!(
            err.to_string().starts_with("the presets file could not be written ("),
            "got {err}"
        );
    }

    #[test]
    fn an_override_field_beats_the_preset_and_a_missing_preset_falls_back_to_default() {
        let preset = MaterialPreset {
            id: "p1".into(),
            name: "Test".into(),
            machine_id: "cameo5".into(),
            settings: PresetSettings { speed: Some(5), force: Some(20), repeat_count: 3 },
            builtin: false,
        };

        let partial = SettingsOverride { speed: None, force: Some(25), repeat_count: None };
        let resolved = resolve_settings(Some(&preset), &partial);
        assert_eq!(resolved.force, Some(25), "override wins");
        assert_eq!(resolved.speed, Some(5), "preset fills the gap");
        assert_eq!(resolved.repeat_count, 3, "preset fills the gap");

        let empty = SettingsOverride { speed: None, force: None, repeat_count: None };
        assert_eq!(resolve_settings(None, &empty), Settings::default());
    }

    #[test]
    fn first_run_save_creates_missing_parent_directory() {
        // Mirrors a fresh install: default_presets_path()'s cuthulhu/ directory
        // doesn't exist yet, and save must create it rather than error.
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("cuthulhu").join("presets.json");

        save_user_presets(&user_file, &[]).unwrap();
        assert!(user_file.exists());
    }

    /// An entry with no id names no material. Codex's third gate on PR #152: with one in the
    /// file, a pass keyed `preset:` resolved to that entry's real speed and force by accident,
    /// while the dialog showed defaults — the machine and the screen disagreeing. The file is
    /// hand-editable, so this is where the state is kept out.
    #[test]
    fn a_user_preset_with_no_id_is_dropped_rather_than_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");
        std::fs::write(&user_file, r#"{"version":1,"presets":[
            {"id":"","name":"Nameless","machine_id":"cameo5",
             "settings":{"speed":1,"force":1,"repeat_count":1},"builtin":false},
            {"id":"mine","name":"Mine","machine_id":"cameo5",
             "settings":{"speed":2,"force":2,"repeat_count":1},"builtin":false}
        ]}"#).unwrap();

        let loaded = load_presets(&user_file).unwrap();
        assert!(!loaded.iter().any(|p| p.id.is_empty()), "an id-less entry must not load");
        assert!(loaded.iter().any(|p| p.id == "mine"),
            "and one malformed entry must not cost the operator the rest of the file");
    }

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

    /// Shadowing is keyed on the machine as well as the id. A Puma entry named with a Cameo
    /// builtin's id used to delete that builtin from the loaded set, and two machines' entries
    /// sharing an operator-chosen id could not coexist at all (#153).
    #[test]
    fn a_user_entry_shadows_a_builtin_only_for_its_own_machine() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("presets.json");
        let entry = |machine: &str, id: &str, speed: u32| MaterialPreset {
            id: id.into(),
            name: format!("{machine} {id}"),
            machine_id: machine.into(),
            settings: PresetSettings { speed: Some(speed), force: Some(10), repeat_count: 1 },
            builtin: false,
        };

        save_user_presets(&user_file, &[
            // A Puma entry whose id is a Cameo builtin's.
            entry("puma", "cameo5-cardstock-medium", 1),
            // And the operator's own id, on both machines.
            entry("cameo5", "my-vinyl", 2),
            entry("puma", "my-vinyl", 3),
        ]).unwrap();

        let loaded = load_presets(&user_file).unwrap();
        let one = |machine: &str, id: &str| {
            let found: Vec<_> = loaded.iter()
                .filter(|p| p.machine_id == machine && p.id == id).collect();
            assert_eq!(found.len(), 1, "exactly one {machine}/{id}, got {found:#?}");
            found[0]
        };

        assert!(one("cameo5", "cameo5-cardstock-medium").builtin,
            "the Cameo's builtin survives a Puma entry that happens to share its id");
        assert!(!one("puma", "cameo5-cardstock-medium").builtin, "and the Puma entry loads too");
        assert_eq!(one("cameo5", "my-vinyl").settings.speed, Some(2));
        assert_eq!(one("puma", "my-vinyl").settings.speed, Some(3),
            "each machine keeps its own settings under the shared id");
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
