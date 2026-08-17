// SPDX-License-Identifier: GPL-3.0-or-later
//! The one way to turn a document's passes into passes a machine can cut.
//!
//! Everything that can refuse a cut lives behind `plan_cut`: the stale-plan
//! check, colour matching, and preflight. A caller cannot skip a step, because
//! there are no steps to skip — it hands over what `plan_passes` gave it plus a
//! selection, and gets either a `CutPlan` or the reason it was refused.

use driver_core::manager::CutPass;
use driver_core::{Job, MachineCaps, MachineProfile, Settings};

use crate::pass_key::PassKey;
use crate::passes::DocumentPasses;
use crate::preflight::{preflight, ConfiguredPass, PreflightError};

/// One pass the caller wants cut, named by the key `plan_passes` gave it. Order within
/// `PlanOptions::passes` is the order they are cut.
#[derive(Clone, Debug, PartialEq)]
pub struct PassSelection {
    pub key: PassKey,
    pub settings: Settings,
}

/// What to cut, and how strictly.
///
/// A pass the caller does not list is not cut — there is no `enabled` flag,
/// because preflight only ever validated enabled passes, so omitting one and
/// disabling it were already the same thing.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanOptions {
    pub passes: Vec<PassSelection>,
    /// When set, refuse if the document has changed since the caller planned
    /// against it. Callers that plan and cut in one breath pass `None`.
    pub expect_revision: Option<u64>,
    pub allow_out_of_bounds: bool,
}

/// One pass, ready to encode. Keeps its key attached to the geometry it belongs to so callers
/// can label passes without index-matching a second list.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedPass {
    pub key: PassKey,
    pub job: Job,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CutPlan {
    pub passes: Vec<PlannedPass>,
}

impl CutPlan {
    /// Hand `DeviceManager::cut` what it takes.
    pub fn cut_passes(&self) -> Vec<CutPass> {
        self.passes.iter().map(|p| CutPass { job: p.job.clone() }).collect()
    }
}

/// Every reason a cut can be refused.
#[derive(Debug, PartialEq)]
pub enum CutError {
    StalePlan { expected: u64, actual: u64 },
    UnknownPass(PassKey),
    /// One pass named twice in a selection. Distinct from `UnknownPass`, because the pass plainly
    /// exists — saying otherwise would send a caller looking for a key that is right there.
    DuplicatePass(PassKey),
    Preflight(PreflightError),
}

/// Every refusal in the words an operator reads. `Preflight` forwards rather than
/// prefixing: its variants are already whole sentences, and the CLI's old
/// `format!("preflight: {e:?}")` is exactly the thing this replaces.
impl std::fmt::Display for CutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // The revision numbers are for a bug report, not for an operator; `Debug` keeps them.
            CutError::StalePlan { .. } =>
                write!(f, "the document changed since this cut was planned"),
            // The key's own `Display`, not a re-spelling of it: a caller who typed
            // `--skip-pass preset:cameo5-htv` must read that string back verbatim.
            CutError::UnknownPass(key) => write!(f, "no planned pass is called {key}"),
            CutError::DuplicatePass(key) => write!(f, "the pass {key} is selected more than once"),
            CutError::Preflight(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for CutError {}

impl CutError {
    /// Stable identifier for a caller that must branch on the *kind* of refusal — see
    /// `PreflightError::code`, whose codes this passes through unchanged so the desktop
    /// keeps emitting one flat set across both types.
    pub fn code(&self) -> &'static str {
        match self {
            CutError::StalePlan { .. } => "stale_plan",
            CutError::UnknownPass(_) => "unknown_pass",
            CutError::DuplicatePass(_) => "duplicate_pass",
            CutError::Preflight(e) => e.code(),
        }
    }
}

/// Select, validate and flatten `planned` into passes for `profile`.
///
/// Takes what `plan_passes` produced rather than the `Document` it came from:
/// planning is the expensive step (traversal, world transforms, flattening) and
/// callers that build a selection already have to know the document's colours,
/// so re-planning here would walk every document twice. It also means the
/// geometry preflight validates is, by construction, the geometry that gets
/// cut — there is no second document to disagree with.
///
/// The machine id checked against `profile` rides along on `planned`, so a
/// caller cannot forget to pass it — passes from a document with no machine set
/// (the CLI imports one straight from SVG) simply skip that rule.
pub fn plan_cut(
    planned: &DocumentPasses,
    profile: &MachineProfile,
    caps: &MachineCaps,
    opts: &PlanOptions,
) -> Result<CutPlan, CutError> {
    if let Some(expected) = opts.expect_revision {
        if planned.doc_revision != expected {
            return Err(CutError::StalePlan { expected, actual: planned.doc_revision });
        }
    }

    // Every selected pass is enabled by construction — omitting a pass is the
    // only way to not cut it, which is why `PassSelection` has no such flag.
    //
    // A key named twice is refused rather than cut twice. Two selections resolve the same
    // `DocumentPass`, so the identical geometry would be flattened into two Jobs and the worker
    // would run both — a second pass over material the blade has already been through, which on
    // vinyl means cutting the backing. `travel_for_order` has always enforced exactly-once for
    // the *preview*; this is the cut path finally agreeing with it. No UI can produce a
    // duplicate (the dialog's rows are unique by key), so this guards the IPC boundary itself.
    let mut configured: Vec<ConfiguredPass> = Vec::with_capacity(opts.passes.len());
    let mut seen: Vec<&PassKey> = Vec::with_capacity(opts.passes.len());
    for sel in &opts.passes {
        if seen.contains(&&sel.key) {
            return Err(CutError::DuplicatePass(sel.key.clone()));
        }
        seen.push(&sel.key);
        let pass = planned
            .passes
            .iter()
            .find(|p| p.key == sel.key)
            .ok_or_else(|| CutError::UnknownPass(sel.key.clone()))?;
        configured.push(ConfiguredPass { pass, settings: sel.settings.clone(), enabled: true });
    }

    preflight(&configured, profile, caps, planned.machine_id.as_deref(), opts.allow_out_of_bounds)
        .map_err(CutError::Preflight)?;

    Ok(CutPlan {
        passes: configured
            .iter()
            .map(|c| PlannedPass {
                key: c.pass.key.clone(),
                job: Job {
                    polylines: c.pass.shapes.iter().flat_map(|s| s.polylines.iter().cloned()).collect(),
                    settings: c.settings.clone(),
                },
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passes::plan_passes;
    use document::history::Editor;
    use document::{Delta, Node, NodeOp, ShapeKind, Style};
    use geometry::Affine;

    const RED: u32 = 0xFF0000FF;
    const BLUE: u32 = 0x0000FFFF;

    /// The passes of a document of 5x5 stroked rects, each
    /// `(stroke, translate_x, translate_y)`.
    fn passes(rects: &[(u32, f64, f64)]) -> DocumentPasses {
        let mut ed = Editor::new();
        let root = ed.doc.root;
        for &(stroke, tx, ty) in rects {
            let id = ed.doc.ids.next();
            let mut node = Node::shape(id, ShapeKind::Rect { w: 5.0, h: 5.0 });
            node.style = Style { stroke: Some(stroke), fill: None };
            node.transform = Affine::translate(tx, ty);
            ed.commit(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));
        }
        plan_passes(&ed.doc).unwrap()
    }

    fn profile(width_mm: f64, height_mm: f64) -> MachineProfile {
        MachineProfile { id: "cameo5".into(), name: "Test".into(), width_mm, height_mm }
    }

    fn caps() -> MachineCaps {
        MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: false }
    }

    /// Colours are the only keys these cases need, so the helper still takes them — it names
    /// them as keys now, which is what `plan_cut` matches on.
    fn select(colors: &[u32]) -> Vec<PassSelection> {
        colors.iter()
            .map(|&c| PassSelection { key: PassKey::Color(Some(c)), settings: Settings::default() })
            .collect()
    }

    fn opts(passes: Vec<PassSelection>) -> PlanOptions {
        PlanOptions { passes, expect_revision: None, allow_out_of_bounds: false }
    }

    #[test]
    fn stale_revision_is_refused() {
        let planned_against = passes(&[(RED, 0.0, 0.0)]).doc_revision;
        // the document moves on after the caller planned against it
        let planned = passes(&[(RED, 0.0, 0.0), (BLUE, 10.0, 10.0)]);

        let mut o = opts(select(&[RED]));
        o.expect_revision = Some(planned_against);
        let err = plan_cut(&planned, &profile(500.0, 500.0), &caps(), &o).unwrap_err();

        assert!(
            matches!(err, CutError::StalePlan { expected, .. } if expected == planned_against),
            "expected StalePlan, got {err:?}"
        );
    }

    /// A selection naming a pass that was not planned is refused, never quietly dropped:
    /// cutting three of four passes because one name was wrong is a ruined sheet.
    #[test]
    fn an_unknown_pass_is_refused_not_dropped() {
        let planned = passes(&[(RED, 0.0, 0.0)]);
        let err = plan_cut(&planned, &profile(500.0, 500.0), &caps(), &opts(select(&[0xDEADBEEF]))).unwrap_err();
        assert_eq!(err, CutError::UnknownPass(PassKey::Color(Some(0xDEADBEEF))));
    }

    /// Codex's blocking finding on PR #152: two selections naming one pass resolve the same
    /// `DocumentPass`, so the identical geometry was flattened into two Jobs and the worker ran
    /// both — a second pass over material the blade had already been through. Predates #148 (the
    /// same hole existed keyed on colour) and is refused here rather than cut.
    #[test]
    fn one_pass_selected_twice_is_refused_not_cut_twice() {
        let planned = passes(&[(RED, 0.0, 0.0)]);
        let twice = select(&[RED, RED]);
        let err = plan_cut(&planned, &profile(500.0, 500.0), &caps(), &opts(twice)).unwrap_err();
        assert_eq!(err, CutError::DuplicatePass(PassKey::Color(Some(RED))));
    }

    #[test]
    fn selection_order_is_cut_order() {
        // plan_passes groups first-seen: red then blue. The selection reverses it.
        let planned = passes(&[(RED, 0.0, 0.0), (BLUE, 10.0, 10.0)]);
        let plan = plan_cut(&planned, &profile(500.0, 500.0), &caps(), &opts(select(&[BLUE, RED]))).unwrap();
        let order: Vec<PassKey> = plan.passes.iter().map(|p| p.key.clone()).collect();
        assert_eq!(order, vec![PassKey::Color(Some(BLUE)), PassKey::Color(Some(RED))]);
    }

    #[test]
    fn a_pass_absent_from_the_selection_is_not_cut() {
        let planned = passes(&[(RED, 0.0, 0.0), (BLUE, 10.0, 10.0)]);
        let plan = plan_cut(&planned, &profile(500.0, 500.0), &caps(), &opts(select(&[RED]))).unwrap();
        assert_eq!(plan.passes.len(), 1);
        assert_eq!(plan.passes[0].key, PassKey::Color(Some(RED)));
    }

    #[test]
    fn an_empty_selection_is_nothing_to_cut() {
        let planned = passes(&[(RED, 0.0, 0.0)]);
        let err = plan_cut(&planned, &profile(500.0, 500.0), &caps(), &opts(vec![])).unwrap_err();
        assert_eq!(err, CutError::Preflight(PreflightError::NothingToCut));
    }

    #[test]
    fn out_of_bounds_is_refused_unless_allowed() {
        // 5x5 rect translated to x=200 sits outside a 100mm-wide bed
        let planned = passes(&[(RED, 200.0, 0.0)]);
        let bed = profile(100.0, 100.0);

        let err = plan_cut(&planned, &bed, &caps(), &opts(select(&[RED]))).unwrap_err();
        assert!(
            matches!(err, CutError::Preflight(PreflightError::OutOfBounds { .. })),
            "expected OutOfBounds, got {err:?}"
        );

        let mut allowed = opts(select(&[RED]));
        allowed.allow_out_of_bounds = true;
        assert!(plan_cut(&planned, &bed, &caps(), &allowed).is_ok(), "the escape hatch must let it through");
    }

    #[test]
    fn the_selections_settings_are_what_preflight_validates() {
        let planned = passes(&[(RED, 0.0, 0.0)]);
        let too_fast = vec![PassSelection {
            key: PassKey::Color(Some(RED)),
            settings: Settings { speed: Some(99), force: None, repeat_count: 1 },
        }];
        let err = plan_cut(&planned, &profile(500.0, 500.0), &caps(), &opts(too_fast)).unwrap_err();
        assert!(
            matches!(err, CutError::Preflight(PreflightError::SettingsOutOfRange(_))),
            "expected SettingsOutOfRange, got {err:?}"
        );
    }

    #[test]
    fn each_job_carries_its_own_pass_geometry_and_settings() {
        let planned = passes(&[(RED, 0.0, 0.0), (RED, 10.0, 0.0), (BLUE, 20.0, 0.0)]);
        let settings = Settings { speed: Some(5), force: Some(20), repeat_count: 2 };
        let sel = vec![PassSelection { key: PassKey::Color(Some(RED)), settings: settings.clone() }];

        let plan = plan_cut(&planned, &profile(500.0, 500.0), &caps(), &opts(sel)).unwrap();

        let job = &plan.passes[0].job;
        assert_eq!(job.settings, settings);
        assert_eq!(job.polylines.len(), 2, "both red rects flatten into the one job");
        assert_eq!(plan.cut_passes().len(), 1);
    }

    /// The three top-level refusals. Preflight's own table is pinned in preflight.rs;
    /// what matters here is that the wrapped variant adds no prefix of its own — a
    /// caller printing "preflight: ..." in front of a finished sentence reads twice.
    #[test]
    fn every_refusal_has_a_sentence_and_a_code() {
        let stale = CutError::StalePlan { expected: 7, actual: 9 };
        assert_eq!(stale.code(), "stale_plan");
        assert_eq!(stale.to_string(), "the document changed since this cut was planned");

        // One arm per key kind, because the sentence names the key: "no planned pass is
        // called preset:cameo5-htv" is a different fact from a colour that is not there.
        for (key, sentence) in [
            (PassKey::Color(Some(0xFF0000FF)), "no planned pass is called color:ff0000ff"),
            (PassKey::Color(None), "no planned pass is called no-color"),
            (PassKey::All, "no planned pass is called all"),
            (PassKey::Preset(Some("cameo5-htv".into())), "no planned pass is called preset:cameo5-htv"),
            (PassKey::Preset(None), "no planned pass is called no-preset"),
        ] {
            let err = CutError::UnknownPass(key);
            assert_eq!(err.code(), "unknown_pass");
            assert_eq!(err.to_string(), sentence);
        }

        let duplicate = CutError::DuplicatePass(PassKey::Color(Some(0xFF0000FF)));
        assert_eq!(duplicate.code(), "duplicate_pass");
        assert_eq!(duplicate.to_string(), "the pass color:ff0000ff is selected more than once");

        let wrapped = CutError::Preflight(PreflightError::NothingToCut);
        assert_eq!(wrapped.code(), "nothing_to_cut");
        assert_eq!(wrapped.to_string(), "no pass selected for this cut has any geometry");
    }
}
