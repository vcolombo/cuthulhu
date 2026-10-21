// SPDX-License-Identifier: GPL-3.0-or-later

//! The operator's application preferences, separate from a project and from presets.
//!
//! Versioned so a file this build does not understand is refused rather than rewritten: an
//! appearance change must not discard a units field a newer build stored, and a damaged file
//! must not be replaced with defaults that look like a choice the operator made (#243).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What `load`, the header `set_appearance` writes, and the sentence `UnknownVersion` names.
const PREFERENCES_VERSION: u64 = 1;

/// Light, dark, or follow the operating system. System is the absence of a choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Light,
    Dark,
    System,
}

impl Default for Appearance {
    fn default() -> Self {
        Self::System
    }
}

/// Lengths on screen. Storage and IPC stay in millimetres; this only chooses how they are shown.
///
/// Absence is not millimetres. A missing key means the operator has not chosen, and the UI
/// resolves the computer's region once and saves `in` or `mm`. A present key, including
/// millimetres, is that saved choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Units {
    #[serde(rename = "in")]
    In,
    #[serde(rename = "mm")]
    Mm,
}

/// The preferences this build reads. Further fields live in the file as raw JSON so a save of
/// one field does not drop them; they are not modelled until a caller has somewhere to put them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    pub appearance: Appearance,
    /// `None` when the file has no `units` key. Serialises as null so the UI can tell
    /// "never saved" from a saved millimetre.
    pub units: Option<Units>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            units: None,
        }
    }
}

/// Which native theme to ask the window for, and which solid colour to put behind the webview
/// while it is still hidden. `theme: None` leaves the window following the OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchChrome {
    pub theme: Option<ResolvedTheme>,
    pub background: ResolvedTheme,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedTheme {
    Light,
    Dark,
}

/// Explicit Light or Dark is that theme. System follows `system` when the OS could be asked,
/// and the light background otherwise — the same colour the hidden window is created with, so
/// a theme query that fails does not flip the first frame to the dark one.
pub fn launch_chrome(appearance: Appearance, system: Option<ResolvedTheme>) -> LaunchChrome {
    match appearance {
        Appearance::Light => LaunchChrome {
            theme: Some(ResolvedTheme::Light),
            background: ResolvedTheme::Light,
        },
        Appearance::Dark => LaunchChrome {
            theme: Some(ResolvedTheme::Dark),
            background: ResolvedTheme::Dark,
        },
        Appearance::System => LaunchChrome {
            theme: None,
            background: system.unwrap_or(ResolvedTheme::Light),
        },
    }
}

#[derive(Debug, PartialEq)]
pub enum PreferencesError {
    /// A finished sentence. Each site states a different fact, so the payload is the sentence.
    Corrupt(String),
    /// A version other than [`PREFERENCES_VERSION`]. `u64` so the sentence can name the number
    /// that was read; narrowing to `u32` made a too-large version print as the one this build
    /// accepts (the same trap `PresetError::UnknownVersion` documents).
    UnknownVersion(u64),
    Unreadable(String),
    Unwritable(String),
}

impl std::fmt::Display for PreferencesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreferencesError::Corrupt(message) => write!(f, "{message}"),
            PreferencesError::UnknownVersion(found) => write!(
                f,
                "this preferences file is in a format this build does not read \
                 (preferences version {found}; this build reads {PREFERENCES_VERSION})"
            ),
            PreferencesError::Unreadable(message) => {
                write!(f, "the preferences file could not be read ({message})")
            }
            PreferencesError::Unwritable(message) => {
                write!(f, "the preferences file could not be written ({message})")
            }
        }
    }
}

impl std::error::Error for PreferencesError {}

impl PreferencesError {
    pub fn code(&self) -> &'static str {
        match self {
            PreferencesError::Corrupt(_) => "preferences_corrupt",
            PreferencesError::UnknownVersion(_) => "preferences_unknown_version",
            PreferencesError::Unreadable(_) => "preferences_unreadable",
            PreferencesError::Unwritable(_) => "preferences_unwritable",
        }
    }
}

/// Beside `presets.json` and `hosts.json`.
pub fn default_preferences_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("cuthulhu").join("preferences.json"))
}

/// The saved preferences, or the defaults when no file has been written yet.
///
/// A missing file is not created here. Stamping defaults on first read would turn "never
/// saved" into a file a newer build then has to migrate, and it would hide a later failure
/// to write. Corrupt and unknown-version files are errors so a caller can refuse to overwrite
/// them; they are not reported as the defaults.
pub fn load(path: &Path) -> Result<Preferences, PreferencesError> {
    let Some(value) = read_value(path)? else {
        return Ok(Preferences::default());
    };
    preferences_of(&value)
}

/// Record `appearance` and leave every other key in place.
///
/// Refuses, without writing, when the file cannot be read, is not a preferences file this
/// build understands, or states a version this build does not. A missing file is the one
/// case that creates one.
pub fn set_appearance(
    path: &Path,
    appearance: Appearance,
) -> Result<Preferences, PreferencesError> {
    update(path, Some(appearance), None)
}

/// Record whichever of appearance and units the caller named. `None` leaves that key as it
/// stands, including a key this build stores and one it does not.
///
/// Both `None` reads and does not write. A missing file is created only when something is set.
pub fn update(
    path: &Path,
    appearance: Option<Appearance>,
    units: Option<Units>,
) -> Result<Preferences, PreferencesError> {
    if appearance.is_none() && units.is_none() {
        return load(path);
    }
    let mut value = match read_value(path)? {
        Some(value) => value,
        None => serde_json::json!({ "version": PREFERENCES_VERSION }),
    };
    // Probe before mutating, so a refusal returns the file untouched. `preferences_of` is that
    // probe: an unknown version and a damaged field both land here, and a missing field is the
    // default rather than damage.
    let _ = preferences_of(&value)?;
    let object = value.as_object_mut().ok_or_else(|| {
        PreferencesError::Corrupt("the preferences file is not a JSON object".into())
    })?;
    object.insert("version".into(), Value::from(PREFERENCES_VERSION));
    if let Some(appearance) = appearance {
        object.insert(
            "appearance".into(),
            serde_json::to_value(appearance)
                .map_err(|e| PreferencesError::Unwritable(e.to_string()))?,
        );
    }
    if let Some(units) = units {
        object.insert(
            "units".into(),
            serde_json::to_value(units).map_err(|e| PreferencesError::Unwritable(e.to_string()))?,
        );
    }
    write_value(path, &value)?;
    preferences_of(&value)
}

/// What startup uses. A missing directory, or a file this build will not read, must not keep
/// the window from opening on the system theme — and must not be rewritten as that theme.
pub fn load_or_warn(path: Option<&Path>, on_error: impl FnOnce(&PreferencesError)) -> Preferences {
    let Some(path) = path else {
        return Preferences::default();
    };
    match load(path) {
        Ok(prefs) => prefs,
        Err(e) => {
            on_error(&e);
            Preferences::default()
        }
    }
}

fn read_value(path: &Path) -> Result<Option<Value>, PreferencesError> {
    // `read`, not `exists` then read: a directory the process cannot search answers `exists`
    // as false, and the operator's choice would come back as "never set".
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        // Windows reports a file standing in the path as NotFound. That is not a
        // preferences file that was never saved: creating the parent would try to
        // replace the file. Unix fails this read with ENOTDIR, which is the other arm.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && an_ancestor_is_a_file(path) => {
            return Err(PreferencesError::Unreadable(e.to_string()));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(PreferencesError::Unreadable(e.to_string())),
    };
    let content = String::from_utf8(bytes)
        .map_err(|_| PreferencesError::Corrupt("the preferences file is not UTF-8 text".into()))?;
    let value: Value = serde_json::from_str(&content).map_err(|e| {
        PreferencesError::Corrupt(format!(
            "the preferences file could not be understood as JSON ({e})"
        ))
    })?;
    Ok(Some(value))
}

fn preferences_of(value: &Value) -> Result<Preferences, PreferencesError> {
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            PreferencesError::Corrupt(
                "the preferences file does not state a usable whole-number version, so this build \
             cannot tell what format it is in"
                    .into(),
            )
        })?;
    if version != PREFERENCES_VERSION {
        return Err(PreferencesError::UnknownVersion(version));
    }
    let appearance = match value.get("appearance") {
        None => Appearance::System,
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| {
            PreferencesError::Corrupt(
                "the preferences file's appearance is not light, dark, or system".into(),
            )
        })?,
    };
    let units = match value.get("units") {
        None | Some(Value::Null) => None,
        Some(v) => Some(serde_json::from_value(v.clone()).map_err(|_| {
            PreferencesError::Corrupt("the preferences file's units are not in or mm".into())
        })?),
    };
    Ok(Preferences { appearance, units })
}

/// `file/preferences.json` when `file` is a regular file. `read` calls that NotFound on
/// Windows and ENOTDIR on Unix, so the NotFound arm is the only one that has to ask.
fn an_ancestor_is_a_file(path: &Path) -> bool {
    let mut cursor = path.parent();
    while let Some(dir) = cursor {
        if dir.as_os_str().is_empty() {
            return false;
        }
        match std::fs::metadata(dir) {
            Ok(meta) => return meta.is_file(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => cursor = dir.parent(),
            // `file/missing` when `file` is a file. Unix says NotADirectory; Windows
            // says NotFound and the walk reaches the file.
            Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => return true,
            Err(_) => return false,
        }
    }
    false
}

fn write_value(path: &Path, value: &Value) -> Result<(), PreferencesError> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| PreferencesError::Unwritable(e.to_string()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| PreferencesError::Unwritable(e.to_string()))?;
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| PreferencesError::Unwritable(e.to_string()))?;
    tmp.write_all(json.as_bytes())
        .map_err(|e| PreferencesError::Unwritable(e.to_string()))?;
    // The io error, not `PersistError`'s temp-file plumbing.
    tmp.persist(path)
        .map_err(|e| PreferencesError::Unwritable(e.error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("preferences.json")
    }

    #[test]
    fn a_missing_file_is_system_and_is_not_created() {
        let dir = dir();
        let path = path(&dir);
        assert_eq!(load(&path).unwrap(), Preferences::default());
        assert!(!path.exists(), "reading defaults must not stamp a file");
    }

    #[test]
    fn appearance_round_trips_and_keeps_a_field_this_build_does_not_model() {
        let dir = dir();
        let path = path(&dir);
        std::fs::write(&path, r#"{"version":1,"units":"in","weed":true}"#).unwrap();
        assert_eq!(load(&path).unwrap().appearance, Appearance::System);
        assert_eq!(load(&path).unwrap().units, Some(Units::In));

        let saved = set_appearance(&path, Appearance::Dark).unwrap();
        assert_eq!(saved.appearance, Appearance::Dark);
        assert_eq!(saved.units, Some(Units::In));
        assert_eq!(load(&path).unwrap().appearance, Appearance::Dark);

        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            value["weed"], true,
            "a later field has to survive an appearance save"
        );
        assert_eq!(value["units"], "in", "saving appearance must not drop units");
        assert_eq!(value["version"], 1);
        assert_eq!(value["appearance"], "dark");

        set_appearance(&path, Appearance::Light).unwrap();
        assert_eq!(load(&path).unwrap().appearance, Appearance::Light);
        set_appearance(&path, Appearance::System).unwrap();
        assert_eq!(load(&path).unwrap().appearance, Appearance::System);
    }

    #[test]
    fn units_round_trip_and_do_not_drop_appearance() {
        let dir = dir();
        let path = path(&dir);
        set_appearance(&path, Appearance::Dark).unwrap();
        let saved = update(&path, None, Some(Units::In)).unwrap();
        assert_eq!(saved.units, Some(Units::In));
        assert_eq!(saved.appearance, Appearance::Dark);
        assert_eq!(load(&path).unwrap().units, Some(Units::In));
        // A missing units key is not a choice. The UI resolves the region and saves one.
        std::fs::write(&path, r#"{"version":1,"appearance":"light"}"#).unwrap();
        assert_eq!(load(&path).unwrap().units, None);
        std::fs::write(&path, r#"{"version":1,"appearance":"light","units":null}"#).unwrap();
        assert_eq!(load(&path).unwrap().units, None);
        // Nothing named is a read, and it must not create a file where there was none.
        let fresh = dir.path().join("fresh.json");
        assert_eq!(update(&fresh, None, None).unwrap(), Preferences::default());
        assert!(!fresh.exists());
    }

    #[test]
    fn units_this_build_does_not_know_are_corrupt_and_are_not_overwritten() {
        let dir = dir();
        let path = path(&dir);
        let original = r#"{"version":1,"units":"cm"}"#;
        std::fs::write(&path, original).unwrap();
        let err = update(&path, None, Some(Units::In)).unwrap_err();
        assert!(
            matches!(err, PreferencesError::Corrupt(ref m) if m.contains("not in or mm")),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn an_unknown_version_is_not_overwritten() {
        let dir = dir();
        let path = path(&dir);
        let original = r#"{"version":99,"appearance":"light","units":"in"}"#;
        std::fs::write(&path, original).unwrap();

        let err = set_appearance(&path, Appearance::Dark).unwrap_err();
        assert_eq!(err, PreferencesError::UnknownVersion(99));
        assert_eq!(err.code(), "preferences_unknown_version");
        assert_eq!(
            err.to_string(),
            "this preferences file is in a format this build does not read \
             (preferences version 99; this build reads 1)"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(
            load(&path).unwrap_err(),
            PreferencesError::UnknownVersion(99)
        );
    }

    #[test]
    fn a_version_past_u32_is_named_as_read() {
        let dir = dir();
        let path = path(&dir);
        std::fs::write(&path, r#"{"version":4294967297}"#).unwrap();
        let err = load(&path).unwrap_err();
        assert_eq!(err, PreferencesError::UnknownVersion(4_294_967_297));
        assert!(err.to_string().contains("4294967297"), "{err}");
    }

    #[test]
    fn a_corrupt_file_is_not_overwritten() {
        let dir = dir();
        let path = path(&dir);
        let original = "{ this is not json";
        std::fs::write(&path, original).unwrap();
        let err = set_appearance(&path, Appearance::Dark).unwrap_err();
        assert!(matches!(err, PreferencesError::Corrupt(_)), "{err:?}");
        assert_eq!(err.code(), "preferences_corrupt");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn bytes_that_are_not_text_are_corrupt_not_unreadable() {
        let dir = dir();
        let path = path(&dir);
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        let err = load(&path).unwrap_err();
        assert_eq!(
            err,
            PreferencesError::Corrupt("the preferences file is not UTF-8 text".into())
        );
    }

    #[test]
    fn an_unusable_version_names_why() {
        let dir = dir();
        let path = path(&dir);
        std::fs::write(&path, r#"{"appearance":"dark"}"#).unwrap();
        let err = load(&path).unwrap_err();
        assert!(
            matches!(err, PreferencesError::Corrupt(ref m) if m.contains("usable whole-number")),
            "{err}"
        );
    }

    #[test]
    fn an_appearance_this_build_does_not_know_is_corrupt() {
        let dir = dir();
        let path = path(&dir);
        let original = r#"{"version":1,"appearance":"sepia"}"#;
        std::fs::write(&path, original).unwrap();
        let err = set_appearance(&path, Appearance::Dark).unwrap_err();
        assert!(
            matches!(err, PreferencesError::Corrupt(ref m) if m.contains("not light, dark, or system")),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn a_first_save_creates_the_directory_and_the_file() {
        let dir = dir();
        let path = dir.path().join("cuthulhu").join("preferences.json");
        set_appearance(&path, Appearance::Light).unwrap();
        assert_eq!(load(&path).unwrap().appearance, Appearance::Light);
    }

    #[test]
    fn a_path_through_a_file_is_refused_and_the_file_is_left_alone() {
        let dir = dir();
        let blocker = dir.path().join("not-a-directory");
        std::fs::write(&blocker, "x").unwrap();
        // `file/preferences.json` is not a missing file. Unix says the parent is not a
        // directory; Windows says the path was not found. Either way it is a read failure.
        // Treating it as "never saved" would try to create a directory where a file stands.
        let path = blocker.join("preferences.json");
        let err = set_appearance(&path, Appearance::Dark).unwrap_err();
        assert!(matches!(err, PreferencesError::Unreadable(_)), "{err:?}");
        assert_eq!(err.code(), "preferences_unreadable");
        assert_eq!(std::fs::read_to_string(&blocker).unwrap(), "x");
        assert_eq!(
            PreferencesError::Unwritable("disk full".into()).code(),
            "preferences_unwritable",
        );
        // The Windows NotFound arm. Unix never reaches it: the read is already ENOTDIR.
        assert!(an_ancestor_is_a_file(&path));
        assert!(an_ancestor_is_a_file(&blocker.join("deeper").join("preferences.json")));
        assert!(!an_ancestor_is_a_file(&dir.path().join("preferences.json")));
    }

    #[test]
    fn load_or_warn_keeps_a_bad_file_and_falls_back_without_writing() {
        let dir = dir();
        let path = path(&dir);
        let original = r#"{"version":4}"#;
        std::fs::write(&path, original).unwrap();
        let mut seen = None;
        let prefs = load_or_warn(Some(&path), |e| seen = Some(e.to_string()));
        assert_eq!(prefs, Preferences::default());
        let message = seen.expect("the bad file was reported");
        assert!(message.contains("preferences version 4"), "{message}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn launch_chrome_follows_an_explicit_choice_and_the_os_only_for_system() {
        assert_eq!(
            launch_chrome(Appearance::Dark, Some(ResolvedTheme::Light)),
            LaunchChrome {
                theme: Some(ResolvedTheme::Dark),
                background: ResolvedTheme::Dark
            }
        );
        assert_eq!(
            launch_chrome(Appearance::Light, Some(ResolvedTheme::Dark)),
            LaunchChrome {
                theme: Some(ResolvedTheme::Light),
                background: ResolvedTheme::Light
            }
        );
        assert_eq!(
            launch_chrome(Appearance::System, Some(ResolvedTheme::Dark)),
            LaunchChrome {
                theme: None,
                background: ResolvedTheme::Dark
            }
        );
        // The window is created on the light background. A system theme that could not be read
        // stays on that colour rather than guessing dark.
        assert_eq!(
            launch_chrome(Appearance::System, None),
            LaunchChrome {
                theme: None,
                background: ResolvedTheme::Light
            }
        );
    }
}
