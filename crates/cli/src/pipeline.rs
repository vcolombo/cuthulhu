// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{close_pass, open_pass, DeviceBackendFactory, DeviceInfo, Driver, Job, Settings};
use driver_registry::{device_at_port, machine_ids, takes_a_named_port, HardwareBackendFactory};

/// The driver for `--device`, or the message to print when there is none. The
/// registry is the only list of machines this build knows, so the CLI resolves
/// against it rather than keeping a second list that can disagree.
pub fn driver_for(machine_id: &str) -> Result<Box<dyn Driver>, String> {
    HardwareBackendFactory
        .driver_for(machine_id)
        .map(|d| d as Box<dyn Driver>)
        .ok_or_else(|| format!("unknown device '{machine_id}' (try: {})", machine_ids().join(", ")))
}

/// Which of the `attached` devices a `--device` cut goes to.
///
/// Enumeration wins when it finds one: a USB device is discriminated by
/// VID/PID, so the one that enumerated is the one meant. A serial port could be
/// any machine — the registry marks those `candidate` and this never picks one
/// for the operator, so `--port` is how a serial machine gets named at all.
/// Whether a machine *can* be named that way is the registry's answer, not this
/// function's: pointing `--port` at a USB machine would pair its dialect with a
/// wire nothing on it can read.
///
/// `attached` is passed in rather than enumerated here so that the choice is a
/// function of its inputs — otherwise every case below the first is reachable
/// only on a machine with the right hardware absent.
pub fn resolve_device_info(
    machine_id: &str,
    attached: &[DeviceInfo],
    port: Option<&str>,
    baud: u32,
) -> Result<DeviceInfo, String> {
    let found = attached.iter().find(|d| d.machine_id == machine_id && !d.candidate);
    if let Some(info) = found {
        return Ok(info.clone());
    }
    // `--port` is only offered to the machines it can help. Suggesting it for a
    // USB machine spends the operator's next attempt on a second refusal.
    let Some(path) = port else {
        return Err(if takes_a_named_port(machine_id) {
            format!("no {machine_id} device found — plug it in, or name its serial port with --port")
        } else {
            format!("no {machine_id} device found — plug it in")
        });
    };
    device_at_port(machine_id, path, baud)
        .ok_or_else(|| format!("no {machine_id} device found, and --port cannot name one: {machine_id} does not connect over a serial port"))
}

/// The whole of Pass `i` of `total` on the wire, for `--dry-run`: what
/// `DeviceManager` writes before it waits for the machine, then what it writes
/// after. Both halves come from `driver-core`, so this cannot say something a cut
/// would not.
pub fn dry_run_pass_bytes(d: &dyn Driver, job: &Job, i: usize, total: usize) -> Result<Vec<u8>, String> {
    let mut bytes = open_pass(d, job, i).map_err(|e| format!("encode: {e:?}"))?;
    bytes.extend(close_pass(d, i, total));
    Ok(bytes)
}

/// Import `svg` into a fresh `Document` — the CLI has no editing model, so a
/// cut is planned against a document that exists only for this command.
pub fn doc_from_svg(svg: &[u8]) -> Result<document::Document, String> {
    let mut doc = document::Document::new();
    // `to_string`, not `{e:?}`: `IoError` has written a sentence since #261, and the
    // desktop's three `IoError` commands forward it verbatim, so this was the one place
    // the same failure still printed a struct literal (#281). No prefix in front of it
    // either, and not because `usvg`'s payload names the format — two of its five
    // variants say only "provided data", so it does not. It is that `cuthulhu cut` reads
    // exactly one file, the one named on the command line beside it, so "the file" has
    // one possible referent and a verb in front of the sentence only says it twice.
    let (delta, _skipped) =
        fileio::import_svg(svg, &mut doc.ids, doc.root).map_err(|e| e.to_string())?;
    doc.apply(delta);
    Ok(doc)
}

/// The passes to cut, in cut order: apply `--order` (named passes to the front, in the order
/// given; the rest keep their planned order) and then `--skip-pass`.
///
/// A key that names no planned pass is refused, for either flag. `--order` used to drop one
/// silently and `--skip-color` still did, which made a typo indistinguishable from a colour
/// the document did not contain — and a silently ignored skip means cutting a pass the
/// operator believed they had excluded.
pub fn pass_order(
    planned: &[cutplan::DocumentPass],
    skip_passes: &[String],
    order: &[String],
) -> Result<Vec<cutplan::PassKey>, String> {
    let mut keys: Vec<cutplan::PassKey> = planned.iter().map(|p| p.key.clone()).collect();
    let parse = |s: &String| s.trim().parse::<cutplan::PassKey>();

    let mut front = vec![];
    for key in order.iter().map(parse).collect::<Result<Vec<_>, _>>()? {
        let Some(i) = keys.iter().position(|k| *k == key) else {
            // A key already moved to the front is a repeat, not an unknown pass — the same
            // distinction `travel_for_order` draws, and for the same reason: "not a pass this
            // file plans" is a lie about a pass that plainly is.
            return Err(if front.contains(&key) {
                format!("--order names {key} twice; each pass can only be ordered once")
            } else {
                format!("--order names {key}, which is not a pass this file plans")
            });
        };
        front.push(keys.remove(i));
    }
    front.extend(keys);
    keys = front;

    for key in skip_passes.iter().map(parse).collect::<Result<Vec<_>, _>>()? {
        let Some(i) = keys.iter().position(|k| *k == key) else {
            return Err(format!("--skip-pass names {key}, which is not a pass this file plans"));
        };
        keys.remove(i);
    }
    Ok(keys)
}

/// Plan a cut from an SVG: import, group, order, select, and validate through
/// `cutplan::plan_cut` — the same entry point the desktop uses, so the CLI gets preflight
/// rather than sending unchecked geometry at the machine.
///
/// One entry point for every mode. `Grouping::Single` used to have its own function because
/// the plain path did its own planning; with the mode named explicitly there is nothing left
/// for a second function to say.
pub fn plan_cut_from_svg(
    svg: &[u8],
    driver: &dyn Driver,
    settings: &Settings,
    grouping: cutplan::Grouping,
    skip_passes: &[String],
    order: &[String],
    allow_out_of_bounds: bool,
) -> Result<cutplan::CutPlan, String> {
    let doc = doc_from_svg(svg)?;
    // Planned once: the flags name passes, so the keys have to be known before a selection
    // can be built, and `plan_cut` cuts the very passes handed to it here.
    let planned = cutplan::plan_passes_with(&doc, grouping).map_err(|e| e.to_string())?;
    // Two different empty cuts, told apart here because only this caller knows an SVG was
    // imported and what the operator asked to skip. Left to `plan_cut`, both would arrive as
    // an unmatched selection or `NothingToCut`, and one sentence would have to cover both.
    if planned.passes.is_empty() {
        return Err("no cuttable paths in SVG".into());
    }
    let keys = pass_order(&planned.passes, skip_passes, order)?;
    if keys.is_empty() {
        return Err("every pass in this file was skipped; nothing is left to cut".into());
    }

    // ponytail: one `--speed`/`--force` pair applies to every pass; the CLI has no per-pass
    // settings and no presets. Per-pass settings need a flag that names a pass key.
    let passes = keys
        .into_iter()
        .map(|key| cutplan::PassSelection { key, settings: settings.clone() })
        .collect();

    // No revision to be stale against: the document was imported a few lines ago.
    let opts = cutplan::PlanOptions { passes, expect_revision: None, allow_out_of_bounds };
    cutplan::plan_cut(&planned, driver.profile(), &driver.caps(), &opts).map_err(describe_cut_error)
}

/// `CutError` as something to print at a terminal. Two arms outlive the shared
/// `Display`: `NothingToCut`, because only this caller knows an SVG was imported
/// and that none of its paths were cut; and out-of-bounds, because naming
/// `--allow-out-of-bounds` is the CLI's to do — the desktop hardcodes
/// `allow_out_of_bounds: false` and offers the operator no such control.
fn describe_cut_error(e: cutplan::CutError) -> String {
    use cutplan::preflight::PreflightError as P;
    match e {
        cutplan::CutError::Preflight(P::NothingToCut) => "no cuttable paths in SVG".into(),
        cutplan::CutError::Preflight(P::OutOfBounds { .. }) =>
            format!("{e} — pass --allow-out-of-bounds to send it anyway"),
        e => e.to_string(),
    }
}

/// `--skip-pass` and `--order` name passes, which only a grouped cut has more than one of. A
/// single-pass cut puts every cut shape in one pass, so these flags cannot do anything there
/// and are refused rather than ignored.
pub fn check_pass_flag_scope(
    skip_passes: &[String],
    order: &[String],
    grouping: cutplan::Grouping,
) -> Result<(), String> {
    if grouping != cutplan::Grouping::Single {
        return Ok(());
    }
    if !skip_passes.is_empty() {
        return Err("--skip-pass applies to a grouped cut; --group-by single is one pass over every shape".into());
    }
    if !order.is_empty() {
        return Err("--order applies to a grouped cut; --group-by single is one pass over every shape".into());
    }
    Ok(())
}

/// More than one pass needs a human at the keyboard between passes; a plan with one pass
/// never pauses, so it is allowed even without a TTY.
pub fn check_interactive(is_tty: bool, pass_count: usize) -> Result<(), String> {
    if !is_tty && pass_count > 1 {
        return Err("a cut with more than one pass requires an interactive terminal".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cutplan::Grouping;

    fn two_color_svg() -> &'static [u8] {
        br##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect width="5" height="5" stroke="#ff0000" fill="none"/>
            <circle cx="10" cy="10" r="3" stroke="#0000ff" fill="none"/>
        </svg>"##
    }

    fn cut_settings() -> Settings {
        Settings { speed: None, force: None, repeat_count: 1 }
    }

    fn cameo5() -> Box<dyn Driver> {
        driver_for("cameo5").expect("the registry knows the Cameo")
    }


    #[test]
    fn out_of_bounds_geometry_is_refused_unless_allowed() {
        // 1512px @96dpi = 400mm wide, past the Cameo's 330mm bed.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect width="1512" height="10" stroke="#ff0000" fill="none"/>
        </svg>"##;

        let err = plan_cut_from_svg(svg, cameo5().as_ref(), &cut_settings(), Grouping::Color, &[], &[], false).unwrap_err();
        assert!(err.contains("outside"), "expected an out-of-bounds refusal, got: {err}");

        assert!(
            plan_cut_from_svg(svg, cameo5().as_ref(), &cut_settings(), Grouping::Color, &[], &[], true).is_ok(),
            "--allow-out-of-bounds must let it through",
        );
    }

    #[test]
    fn settings_out_of_range_are_refused_before_reaching_the_machine() {
        let bad = Settings { speed: Some(99), force: None, repeat_count: 1 };
        let err = plan_cut_from_svg(two_color_svg(), cameo5().as_ref(), &bad, Grouping::Color, &[], &[], false).unwrap_err();
        assert!(err.contains("speed"), "expected a settings-range refusal, got: {err}");
    }

    /// A fill-only SVG used to be refused by name here, because `plan_passes` cut only stroked
    /// shapes. Since #144 a colour grouping plans it, keyed on the fill, and the refusal it used to
    /// produce belongs to an SVG with no geometry at all — which
    /// `by_color_cut_of_an_svg_with_no_geometry_is_refused_by_name` covers.
    #[test]
    fn a_fill_only_svg_is_planned_by_color_on_its_fill() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect width="5" height="5" fill="#ff0000"/>
        </svg>"##;
        let plan = plan_cut_from_svg(svg, cameo5().as_ref(), &cut_settings(), Grouping::Color, &[], &[], false).unwrap();
        assert_eq!(plan.passes.len(), 1);
        assert_eq!(plan.passes[0].key, cutplan::PassKey::Color(Some(0xFF0000FF)));
    }

    /// The grouped route to "no cuttable paths in SVG": no geometry means no passes, so the
    /// early check reports the file rather than letting an unmatched selection describe the
    /// request. Since #148 both modes take that same check, so this and its `Single`
    /// counterpart pin one sentence rather than two implementations of it.
    #[test]
    fn by_color_cut_of_an_svg_with_no_geometry_is_refused_by_name() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"></svg>"##;
        let err = plan_cut_from_svg(svg, cameo5().as_ref(), &cut_settings(), Grouping::Color, &[], &[], false)
            .expect_err("an SVG with no geometry has nothing to cut");
        assert_eq!(err, "no cuttable paths in SVG");
    }

    /// A single-pass cut means everything in the file in one pass, and since #148 it says so
    /// with a grouping mode rather than a separate planning function. The colour half pins
    /// that a fill keys a pass at all, through the caller the CLI actually uses.
    #[test]
    fn plain_cut_plans_one_pass_and_by_color_still_sees_both_fills() {
        let two_fills = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <rect width="5" height="5" fill="#ff0000"/><rect x="6" width="5" height="5" fill="#00ff00"/></svg>"##;

        let plain = plan_cut_from_svg(two_fills, cameo5().as_ref(), &Settings::default(), Grouping::Single, &[], &[], false).unwrap();
        assert_eq!(plain.passes.len(), 1);
        assert_eq!(plain.passes[0].key, cutplan::PassKey::All, "one pass by request, named for that");

        let by_color =
            plan_cut_from_svg(two_fills, cameo5().as_ref(), &cut_settings(), Grouping::Color, &[], &[], false).unwrap();
        assert_eq!(by_color.passes.len(), 2, "the fixture's two fills survived the import");
        assert!(by_color.passes.iter().all(|p| p.key != cutplan::PassKey::Color(Some(0x000000FF))),
            "keyed on the fills, not on the black stroke a plain cut used to stamp");
    }

    /// The fill-only-clipart case, stated as behaviour rather than as a consequence: paint that
    /// nobody can see still cuts, because cuttability is the attribute and import defaults it to
    /// `Cut`. Before #144 this SVG planned nothing at all.
    #[test]
    fn plain_cut_plans_invisible_paint() {
        let transparent_fill = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <rect width="5" height="5" fill="#00ff00" fill-opacity="0"/></svg>"##;

        let plan = plan_cut_from_svg(transparent_fill, cameo5().as_ref(), &Settings::default(), Grouping::Single, &[], &[], false).unwrap();
        assert_eq!(plan.passes.len(), 1);
        assert_eq!(plan.passes[0].key, cutplan::PassKey::All);
    }

    /// The whole point of the change: the plain path is preflighted. A shape past the
    /// bed's edge was silently sent to the machine before.
    #[test]
    fn plain_cut_refuses_out_of_bounds_geometry() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10000mm" height="10mm">
            <rect x="9000" width="500" height="5" fill="#000000"/></svg>"##;
        let err = plan_cut_from_svg(svg, cameo5().as_ref(), &Settings::default(), Grouping::Single, &[], &[], false)
            .expect_err("out of bounds must be refused");
        assert!(err.contains("outside"), "unexpected message: {err}");
        // ...and the escape hatch works, now that there is a check to overrule.
        assert!(plan_cut_from_svg(svg, cameo5().as_ref(), &Settings::default(), Grouping::Single, &[], &[], true).is_ok());
    }

    /// With no paths at all there are no passes, so the early check reports the file rather
    /// than letting an unmatched selection read as an internal error.
    #[test]
    fn plain_cut_of_an_empty_svg_says_nothing_to_cut() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"></svg>"##;
        let err = plan_cut_from_svg(svg, cameo5().as_ref(), &Settings::default(), Grouping::Single, &[], &[], false).expect_err("empty");
        assert_eq!(err, "no cuttable paths in SVG");
    }

    fn planned_two_colours() -> cutplan::DocumentPasses {
        cutplan::plan_passes(&doc_from_svg(two_color_svg()).unwrap()).unwrap()
    }

    /// `--order` puts named passes first in the order given, then everything else in planned
    /// order; `--skip-pass` removes. Keys, not colours, so a preset-grouped cut can be
    /// sequenced exactly as a colour-grouped one always could.
    #[test]
    fn pass_order_sequences_and_skips_by_key() {
        let planned = planned_two_colours();
        let blue_first = pass_order(&planned.passes, &[], &["color:0000ffff".into()]).unwrap();
        assert_eq!(blue_first,
            vec![cutplan::PassKey::Color(Some(0x0000FFFF)), cutplan::PassKey::Color(Some(0xFF0000FF))]);

        let without_red = pass_order(&planned.passes, &["color:ff0000ff".into()], &[]).unwrap();
        assert_eq!(without_red, vec![cutplan::PassKey::Color(Some(0x0000FFFF))]);

        // Order is applied before the skip filter, as it always was.
        let both = pass_order(&planned.passes, &["color:ff0000ff".into()],
            &["color:0000ffff".into(), "color:ff0000ff".into()]).unwrap();
        assert_eq!(both, vec![cutplan::PassKey::Color(Some(0x0000FFFF))]);
    }

    /// `--order` is repeatable rather than comma-separated, because a preset id may contain a
    /// comma and a split list would make such a pass unnameable — an operator's own string
    /// deciding whether a flag can address a pass.
    #[test]
    fn order_is_repeatable_and_keeps_the_order_given() {
        let planned = planned_two_colours();
        let keys = pass_order(&planned.passes, &[],
            &["color:0000ffff".into(), "color:ff0000ff".into()]).unwrap();
        assert_eq!(keys,
            vec![cutplan::PassKey::Color(Some(0x0000FFFF)), cutplan::PassKey::Color(Some(0xFF0000FF))]);
    }

    /// Both flags refuse a key that names no planned pass. `--order` used to drop one
    /// silently and `--skip-color` still did: with four spellings of a key a typo is likelier
    /// than it was, and a skipped pass that was never there means cutting a colour the
    /// operator believed they had excluded. A key from another mode needs no rule of its own.
    #[test]
    fn a_key_that_names_no_planned_pass_is_refused() {
        let planned = planned_two_colours();
        let err = pass_order(&planned.passes, &[], &["no-preset".into()]).unwrap_err();
        assert!(err.contains("no-preset"), "{err}");

        let err = pass_order(&planned.passes, &["preset:cameo5-htv".into()], &[]).unwrap_err();
        assert!(err.contains("preset:cameo5-htv"), "{err}");
    }

    /// Naming one pass twice is a repeat, not an unknown pass. Copilot's point on PR #152:
    /// "not a pass this file plans" is a lie about a pass that plainly is, and the operator
    /// cannot tell a typo from a duplicate if both say the same thing.
    #[test]
    fn ordering_the_same_pass_twice_says_so() {
        let planned = planned_two_colours();
        let err = pass_order(&planned.passes, &[],
            &["color:ff0000ff".into(), "color:ff0000ff".into()]).unwrap_err();
        assert!(err.contains("twice"), "{err}");
        assert!(err.contains("color:ff0000ff"), "{err}");
    }

    /// A malformed key is `PassKey`'s own error, surfaced unchanged: one grammar means one
    /// message, and the CLI is where a person types it.
    #[test]
    fn a_malformed_pass_key_is_refused_with_the_grammar() {
        let planned = planned_two_colours();
        let err = pass_order(&planned.passes, &["ff0000ff".into()], &[]).unwrap_err();
        assert!(err.contains("is not a pass key"), "{err}");
    }

    /// A single-pass cut has one pass whose name nobody needs, so these flags cannot do
    /// anything and are refused rather than ignored.
    #[test]
    fn pass_flags_are_refused_for_a_single_pass_cut() {
        assert_eq!(
            check_pass_flag_scope(&["color:ff0000ff".into()], &[], Grouping::Single),
            Err("--skip-pass applies to a grouped cut; --group-by single is one pass over every shape".into())
        );
        assert_eq!(
            check_pass_flag_scope(&[], &["color:ff0000ff".into()], Grouping::Single),
            Err("--order applies to a grouped cut; --group-by single is one pass over every shape".into())
        );
        for g in [Grouping::Color, Grouping::Stroke, Grouping::Fill, Grouping::Preset] {
            assert!(check_pass_flag_scope(&["color:ff0000ff".into()], &["color:ff0000ff".into()], g).is_ok());
        }
    }

    /// Two different empty cuts, two different sentences. "no cuttable paths in SVG" used to
    /// cover both, which told an operator their file was empty when in fact their own
    /// `--skip-pass` had emptied the selection.
    #[test]
    fn an_empty_file_and_an_emptied_selection_read_differently() {
        let err = plan_cut_from_svg(two_color_svg(), cameo5().as_ref(), &cut_settings(),
            Grouping::Color, &["color:ff0000ff".into(), "color:0000ffff".into()], &[], false)
            .unwrap_err();
        assert_eq!(err, "every pass in this file was skipped; nothing is left to cut");
    }

    /// The TTY rule is about passes, not about a flag name: one pass never pauses, so it is
    /// allowed unattended whichever mode produced it.
    #[test]
    fn an_unattended_multi_pass_cut_is_refused() {
        assert_eq!(
            check_interactive(false, 2),
            Err("a cut with more than one pass requires an interactive terminal".into())
        );
        assert!(check_interactive(false, 1).is_ok());
        assert!(check_interactive(true, 2).is_ok());
    }

    /// `--device` resolves through the registry rather than a list the CLI
    /// keeps: every machine the registry knows is accepted, and an unknown one
    /// is refused with that same list rather than a hardcoded suggestion.
    #[test]
    fn device_ids_come_from_the_registry() {
        for id in machine_ids() {
            assert_eq!(driver_for(id).expect("registry id").profile().id, id);
        }
        // `.err()` rather than `expect_err`: `Box<dyn Driver>` has no `Debug`.
        let err = driver_for("cameo6").err().expect("unknown device must be refused");
        for id in machine_ids() {
            assert!(err.contains(id), "{err} should name {id} as a choice");
        }
    }

    /// A serial port that enumerated is a `candidate` — something is on it, but
    /// nothing says what. Resolution never picks one, so a `--port`-less serial
    /// cut asks instead of guessing.
    #[test]
    fn a_serial_device_needs_port_and_is_taken_at_the_operators_word() {
        let enumerated_port = DeviceInfo {
            instance_id: "serial:/dev/ttyS9".into(),
            machine_id: "puma".into(),
            transport: driver_core::TransportKind::Serial { path: "/dev/ttyS9".into(), baud: 9600 },
            candidate: true,
            host: None,
        };
        let err = resolve_device_info("puma", std::slice::from_ref(&enumerated_port), None, 9600)
            .expect_err("must not guess which serial port is the Puma");
        assert!(err.contains("--port"), "unexpected message: {err}");

        let info = resolve_device_info("puma", &[enumerated_port], Some("/dev/ttyUSB0"), 19200).expect("named port");
        assert_eq!(info.machine_id, "puma");
        assert_eq!(
            info.transport,
            driver_core::TransportKind::Serial { path: "/dev/ttyUSB0".into(), baud: 19200 }
        );
        assert!(info.candidate, "an operator-named port is still unverified hardware");
    }

    /// `--port` used to be ignored for a USB machine, which at least sent it
    /// nowhere. Honouring it for every machine would be worse: with no Cameo
    /// attached it would write GPGL to whatever sits on that port. The registry
    /// says which machines a port can name, and this refuses the rest.
    #[test]
    fn a_usb_machine_cannot_be_pointed_at_a_serial_port() {
        let err = resolve_device_info("cameo5", &[], Some("/dev/ttyUSB0"), 9600)
            .expect_err("a Cameo does not speak serial");
        assert!(err.contains("does not connect over a serial port"), "unexpected message: {err}");

        // ...and the missing-device message does not send the operator to a flag
        // whose only effect on this machine is the refusal above.
        let err = resolve_device_info("cameo5", &[], None, 9600).expect_err("nothing attached");
        assert!(!err.contains("--port"), "a USB machine must not be offered --port: {err}");
    }

    /// An enumerated device is the one meant, and `--port` does not override it:
    /// the Cameo announced itself over USB, so a port is not what it is on.
    #[test]
    fn an_enumerated_device_wins_over_a_named_port() {
        let attached = [DeviceInfo {
            instance_id: "usb:1:4".into(),
            machine_id: "cameo5".into(),
            transport: driver_core::TransportKind::Usb { locator: "1:4".into() },
            candidate: false,
            host: None,
        }];
        let info = resolve_device_info("cameo5", &attached, Some("/dev/ttyUSB0"), 9600).expect("attached Cameo");
        assert_eq!(info.transport, driver_core::TransportKind::Usb { locator: "1:4".into() });
    }

    /// The leak this change exists to close: four preflight refusals used to fall
    /// through to `format!("preflight: {e:?}")`, so a document built for a Puma sent
    /// to a Cameo printed a struct literal. Tested against `describe_cut_error`
    /// directly because an SVG import never sets a machine id, so `plan_cut_from_svg`
    /// cannot reach `MachineMismatch`.
    #[test]
    fn a_machine_mismatch_reads_as_a_sentence() {
        let err = describe_cut_error(cutplan::CutError::Preflight(
            cutplan::preflight::PreflightError::MachineMismatch {
                document: "puma".into(),
                device: "cameo5".into(),
            },
        ));
        assert_eq!(err, "this document is set up for `puma`, but the connected machine is `cameo5`");
    }

    /// Out-of-bounds is the one refusal an operator may reasonably want to overrule,
    /// and only the CLI has a flag for it — the desktop hardcodes `allow_out_of_bounds:
    /// false`. So the shared sentence states the fact and this caller adds the escape.
    #[test]
    fn out_of_bounds_names_the_flag_that_overrules_it() {
        let err = describe_cut_error(cutplan::CutError::Preflight(
            cutplan::preflight::PreflightError::OutOfBounds {
                node: document::NodeId(3),
                bounds: (0.0, 0.0, 304.8, 304.8),
            },
        ));
        assert_eq!(
            err,
            "shape #3 lies outside the 304.8 x 304.8 mm cutting area — pass --allow-out-of-bounds to send it anyway",
        );
    }

    /// Bytes `usvg` declines used to reach the operator as `SVG parse: Parse("…")` — a
    /// struct literal wrapped around the sentence `IoError` has written since #261, and
    /// which the desktop's own `import_svg` command has forwarded since that same change
    /// (#281). `import_svg` can fail in exactly one place — the `usvg::Tree::from_data`
    /// call inside `svg_to_paths` — so one sentence is true of every input below; the
    /// four are the shapes an operator hands a cutter by accident, not four branches.
    ///
    /// Stated as forwarding, because forwarding is the contract: whatever `IoError`
    /// writes for these bytes is what the operator reads, character for character. A
    /// test that pinned `usvg`'s own wording instead would break on a parser upgrade,
    /// and — the gap Codex found in the first version of this — would still pass if the
    /// parenthesised half were dropped on the way out, which is the whole of what the
    /// operator can act on.
    #[test]
    fn an_svg_that_cannot_be_parsed_reads_as_a_sentence() {
        let refused: [(&str, &[u8]); 4] = [
            ("not markup at all", b"this is not an SVG"),
            ("truncated mid-element", br#"<svg xmlns="http://www.w3.org/2000/svg"><rect"#),
            ("well-formed XML that is not SVG", br#"<html><body/></html>"#),
            ("nothing at all", b""),
        ];

        for (what, bytes) in refused {
            let mut doc = document::Document::new();
            let sentence = fileio::import_svg(bytes, &mut doc.ids, doc.root)
                .err()
                .unwrap_or_else(|| panic!("{what} is not an importable SVG"))
                .to_string();
            let err = doc_from_svg(bytes).expect_err(what);
            assert_eq!(err, sentence, "{what}: the CLI reworded what `IoError` wrote");

            let detail = err
                .strip_prefix("the file could not be understood (")
                .and_then(|rest| rest.strip_suffix(')'))
                .unwrap_or_else(|| panic!("{what}: not the sentence `IoError` writes: {err}"));
            assert!(!detail.is_empty(), "{what}: the parser's own account was dropped");
            assert!(!detail.contains("Parse("), "{what}: a `Debug` rendering came back: {err}");
        }
    }
}
