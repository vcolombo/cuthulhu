// SPDX-License-Identifier: GPL-3.0-or-later
//! What a pass is called, and the one string form that name takes everywhere.

use serde::{Deserialize, Serialize};

/// What a `DocumentPass` is keyed on — the answer to "which pass is this?" for whichever
/// `Grouping` produced it.
///
/// `All` is deliberately not `Color(None)`. Before #148 the single pass a `Grouping::Single`
/// plan holds and the pass of shapes with no visible paint were both keyed `None`, so a
/// refusal could only say the evasive "no planned pass without a color".
///
/// The `Option`s sit inside their variants rather than a shared `Unassigned`, because absence
/// is a property of that mode's key: `Color(None)` is a shape with no visible paint *in the
/// mode's terms*, and `Preset(None)` is a shape resolving to no material — the ordinary
/// state, not an error.
///
/// Serialized as its canonical string (`Display`/`FromStr`) rather than as a tagged enum: the
/// CLI needs a human grammar, the cut dialog needs a stable row key, and the e2e fake has to
/// produce byte-identical values. One representation cannot drift from itself.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum PassKey {
    All,
    Color(Option<u32>),
    Preset(Option<String>),
}

impl std::fmt::Display for PassKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PassKey::All => write!(f, "all"),
            // Lowercase, always: `FromStr` accepts either case, so writing one of them is
            // what makes the round trip a fixed point.
            PassKey::Color(Some(c)) => write!(f, "color:{c:08x}"),
            PassKey::Preset(Some(id)) => write!(f, "preset:{id}"),
            // Absence is its own token, never a reserved value inside a mode's namespace: a
            // preset id is an unrestricted operator string, so `preset:none` would collide
            // with a preset actually called `none`. `no-color` follows the same rule for one
            // grammar rather than two, even though 8 hex digits could not have collided.
            PassKey::Color(None) => write!(f, "no-color"),
            PassKey::Preset(None) => write!(f, "no-preset"),
        }
    }
}

impl std::str::FromStr for PassKey {
    type Err = String;
    fn from_str(s: &str) -> Result<PassKey, String> {
        let unknown = || format!("'{s}' is not a pass key (all, color:RRGGBBAA, no-color, preset:<id>, no-preset)");
        // First separator only: a preset id may contain a colon, so the grammar must not
        // constrain ids further than the application does.
        let Some((mode, value)) = s.split_once(':') else {
            return match s {
                "all" => Ok(PassKey::All),
                "no-color" => Ok(PassKey::Color(None)),
                "no-preset" => Ok(PassKey::Preset(None)),
                _ => Err(unknown()),
            };
        };
        match (mode, value) {
            // Eight digits exactly: a 6-digit RRGGBB would parse as 0x00RRGGBB — a colour no
            // shape carries — and match nothing while looking like it should.
            ("color", hex) if hex.len() == 8 => u32::from_str_radix(hex, 16)
                .map(|c| PassKey::Color(Some(c)))
                .map_err(|_| format!("'{s}' has a colour that is not 8 hex digits (RRGGBBAA)")),
            ("color", _) => Err(format!("'{s}' has a colour that is not 8 hex digits (RRGGBBAA)")),
            // An empty id is accepted, because `Display` can write one and a grammar whose own
            // output it refuses is not a round trip: `PassKey::Preset(Some(String::new()))` is a
            // constructible state (nothing validates a `MaterialPreset::id`), and serde would
            // emit `preset:` for it. Refusing it here was inherited from the first spelling,
            // where absence *was* `preset:none` and an empty tail really was ambiguous; with
            // absence spelled `no-preset`, `preset:` means one thing only. Whether an empty id
            // should be storable at all is a preset-file question, not this grammar's — #153.
            ("preset", id) => Ok(PassKey::Preset(Some(id.to_string()))),
            _ => Err(unknown()),
        }
    }
}

// The pair serde's `into`/`try_from` needs. `into` clones, which is why `PassKey` derives
// `Clone` — it holds a `String` and would need it anyway.
impl From<PassKey> for String {
    fn from(key: PassKey) -> String { key.to_string() }
}
impl TryFrom<String> for PassKey {
    type Error = String;
    fn try_from(s: String) -> Result<PassKey, String> { s.parse() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Every variant, both directions, plus the JSON form. The same table appears in
    /// TypeScript (`apps/desktop/ui/src/cut/viewmodel.test.ts`) because the string is the
    /// only thing that crosses the boundary — if the two tables disagree, the dialog and the
    /// planner disagree about what a pass is called.
    #[test]
    fn every_key_round_trips_through_its_canonical_string() {
        let table = [
            (PassKey::All, "all"),
            (PassKey::Color(Some(0xFF0000FF)), "color:ff0000ff"),
            (PassKey::Color(Some(0x0000FFFF)), "color:0000ffff"),
            (PassKey::Color(None), "no-color"),
            (PassKey::Preset(Some("cameo5-htv".into())), "preset:cameo5-htv"),
            (PassKey::Preset(None), "no-preset"),
        ];
        for (key, text) in table {
            assert_eq!(key.to_string(), text);
            assert_eq!(text.parse::<PassKey>().unwrap(), key);
            assert_eq!(serde_json::to_string(&key).unwrap(), format!("\"{text}\""));
            assert_eq!(serde_json::from_str::<PassKey>(&format!("\"{text}\"")).unwrap(), key);
        }
    }

    /// The property the first spelling lacked. A preset id is an unrestricted operator
    /// string (`presets.rs:9-15`), so `preset:none` for "no preset" collided with a preset
    /// whose id is literally `none` — two passes writing one string, which is duplicate
    /// React keys and a `plan_mismatch` no operator can clear. Absence is its own token.
    #[test]
    fn no_two_keys_write_the_same_string() {
        let keys = [
            PassKey::All,
            PassKey::Color(Some(0xFF0000FF)),
            PassKey::Color(None),
            PassKey::Preset(None),
            PassKey::Preset(Some("none".into())),
            PassKey::Preset(Some("no-preset".into())),
            PassKey::Preset(Some("all".into())),
            PassKey::Preset(Some("vinyl:thin".into())),
            PassKey::Preset(Some("with,comma".into())),
            // Constructible, so it must round-trip: `Display` writes `preset:` for it, and a
            // grammar that refuses its own output is not one.
            PassKey::Preset(Some(String::new())),
        ];
        let mut seen: HashMap<String, PassKey> = HashMap::new();
        for key in &keys {
            let text = key.to_string();
            assert_eq!(text.parse::<PassKey>().unwrap(), *key, "round trip {text}");
            if let Some(prev) = seen.insert(text.clone(), key.clone()) {
                panic!("{prev:?} and {key:?} both write {text}");
            }
        }
    }

    /// A colour is written lowercase and read either way, so the round trip is a fixed
    /// point: two spellings of one key would otherwise both appear in a pass list, and
    /// `plan_cut` matches keys by equality.
    #[test]
    fn a_colour_is_read_in_any_case_and_written_in_one() {
        assert_eq!("color:FF0000ff".parse::<PassKey>().unwrap(), PassKey::Color(Some(0xFF0000FF)));
        assert_eq!(PassKey::Color(Some(0xFF0000FF)).to_string(), "color:ff0000ff");
    }

    /// A preset id may contain the separator, because ids are the operator's and nothing
    /// validates them. Splitting on the first `:` is what allows that.
    #[test]
    fn a_preset_id_may_contain_the_separator() {
        let key: PassKey = "preset:vinyl:thin".parse().unwrap();
        assert_eq!(key, PassKey::Preset(Some("vinyl:thin".into())));
        assert_eq!(key.to_string(), "preset:vinyl:thin");
    }

    /// Refused rather than coerced, and quoting the input: these arrive from a person typing
    /// `--skip-pass`. `preset:` is refused because an empty tail is the one id that would be
    /// indistinguishable from a truncated key; `color:none` and `line-type:cut` are refused
    /// because they are the retired spellings, and accepting them quietly would resurrect the
    /// collision and a mode that no longer exists.
    #[test]
    fn a_malformed_key_is_refused_by_name() {
        for bad in ["", "all:1", "color:", "color:zz", "color:ff0000",
                    "color:none", "line-type:cut", "no-material", "colour:ff0000ff"] {
            let err = bad.parse::<PassKey>().expect_err("must not parse");
            assert!(err.contains(bad), "{err} should quote the input");
        }
    }
}
