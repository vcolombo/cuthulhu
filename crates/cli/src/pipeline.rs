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
    let (delta, _skipped) = fileio::import_svg(svg, &mut doc.ids, doc.root)
        .map_err(|e| format!("SVG parse: {e:?}"))?;
    doc.apply(delta);
    Ok(doc)
}

/// The stroke a plain cut gives every imported path. Opaque black, matching
/// `document::Style::default()`.
pub const CUT_STROKE: u32 = 0x000000FF;

/// Import `svg` for a plain (non-`--by-color`) cut: every path gets the same
/// stroke, so `plan_passes` finds all of it and groups it into exactly one
/// `ColorPass`.
///
/// This is what the plain path has always meant — cut everything in the file,
/// in one pass — stated explicitly so the cut can go through `plan_cut` and be
/// preflighted. It deliberately does not touch `plan_passes`' stroke rule; see
/// issue #68 for whether that rule should change at all.
pub fn doc_from_svg_all_cuttable(svg: &[u8]) -> Result<document::Document, String> {
    let mut doc = document::Document::new();
    let (mut delta, _skipped) = fileio::import_svg(svg, &mut doc.ids, doc.root)
        .map_err(|e| format!("SVG parse: {e:?}"))?;
    for op in delta.0.iter_mut() {
        if let document::NodeOp::Add { node, .. } = op {
            node.style.stroke = Some(CUT_STROKE);
        }
    }
    doc.apply(delta);
    Ok(doc)
}

/// The colours to cut, in cut order: apply `--order` (listed colours to the
/// front, in listed sequence; the rest keep their relative order) and then
/// `--skip-color`, in that order per the brief.
pub fn pass_order(
    planned: &[cutplan::ColorPass],
    skip_colors: &[String],
    order: Option<String>,
) -> Result<Vec<Option<u32>>, String> {
    let mut colors: Vec<Option<u32>> = planned.iter().map(|p| p.color).collect();

    if let Some(order) = order {
        let wanted: Vec<u32> = order.split(',').map(|s| parse_hex_color(s.trim())).collect::<Result<_, _>>()?;
        let mut front = vec![];
        for color in wanted {
            if let Some(i) = colors.iter().position(|c| *c == Some(color)) {
                front.push(colors.remove(i));
            }
        }
        front.extend(colors);
        colors = front;
    }

    let skip: Vec<u32> = skip_colors.iter().map(|s| parse_hex_color(s)).collect::<Result<_, _>>()?;
    colors.retain(|c| !c.is_some_and(|c| skip.contains(&c)));
    Ok(colors)
}

/// Plan a `--by-color` cut from an SVG: import, order, select, and validate
/// through `cutplan::plan_cut` — the same entry point the desktop uses, so the
/// CLI gets preflight rather than sending unchecked geometry at the machine.
pub fn plan_cut_from_svg(
    svg: &[u8],
    driver: &dyn Driver,
    settings: &Settings,
    skip_colors: &[String],
    order: Option<String>,
    allow_out_of_bounds: bool,
) -> Result<cutplan::CutPlan, String> {
    let doc = doc_from_svg(svg)?;
    // Planned once: --order and --skip-color name colours, so the colours have
    // to be known before a selection can be built, and plan_cut cuts the very
    // passes handed to it here.
    let planned = cutplan::plan_passes(&doc).map_err(|e| e.to_string())?;
    let colors = pass_order(&planned.passes, skip_colors, order)?;

    // One `--speed`/`--force` pair applies to every pass; the CLI has no
    // per-pass settings and no presets.
    let passes = colors
        .into_iter()
        .map(|color| cutplan::PassSelection { color, settings: settings.clone() })
        .collect();

    // No revision to be stale against: the document was imported a few lines ago.
    let opts = cutplan::PlanOptions { passes, expect_revision: None, allow_out_of_bounds };
    cutplan::plan_cut(&planned, driver.profile(), &driver.caps(), &opts).map_err(describe_cut_error)
}

/// Plan a plain cut: all geometry, one pass, validated through `plan_cut` — the
/// same entry point the desktop and `--by-color` use.
pub fn plan_plain_cut(
    svg: &[u8],
    driver: &dyn Driver,
    settings: &Settings,
    allow_out_of_bounds: bool,
) -> Result<cutplan::CutPlan, String> {
    let doc = doc_from_svg_all_cuttable(svg)?;
    let planned = cutplan::plan_passes(&doc).map_err(|e| e.to_string())?;
    // Checked here rather than left to `plan_cut`: with no passes at all, asking for
    // CUT_STROKE is an unmatched colour, and "no pass matches color" describes the
    // request instead of the file.
    if planned.passes.is_empty() {
        return Err("no cuttable paths in SVG".into());
    }
    let passes = vec![cutplan::PassSelection { color: Some(CUT_STROKE), settings: settings.clone() }];
    let opts = cutplan::PlanOptions { passes, expect_revision: None, allow_out_of_bounds };
    cutplan::plan_cut(&planned, driver.profile(), &driver.caps(), &opts).map_err(describe_cut_error)
}

/// `CutError` as something to print at a terminal. Two arms outlive the shared
/// `Display`: `NothingToCut`, because only this caller knows an SVG was imported
/// and that none of its paths were stroked; and out-of-bounds, because naming
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

/// Parse an 8-hex-digit `RRGGBBAA` string into a `0xRRGGBBAA` color.
/// Parses an 8-digit `RRGGBBAA` hex color. The length check is required: without
/// it a 6-digit `RRGGBB` parses as `0x00RRGGBB` and silently matches nothing.
pub fn parse_hex_color(s: &str) -> Result<u32, String> {
    if s.len() != 8 {
        return Err(format!("bad color '{s}': expected 8 hex digits (RRGGBBAA)"));
    }
    u32::from_str_radix(s, 16).map_err(|e| format!("bad color '{s}': {e}"))
}

/// `--skip-color` and `--order` select and sequence colours, which only a
/// `--by-color` cut has. A plain cut puts every colour in one pass, so these
/// flags cannot do anything there and are refused rather than ignored.
pub fn check_color_flag_scope(
    skip_colors: &[String],
    order: &Option<String>,
    by_color: bool,
) -> Result<(), String> {
    if by_color {
        return Ok(());
    }
    if !skip_colors.is_empty() {
        return Err("--skip-color applies to --by-color cuts; a plain cut is one pass over every colour".into());
    }
    if order.is_some() {
        return Err("--order applies to --by-color cuts; a plain cut is one pass over every colour".into());
    }
    Ok(())
}

/// `--by-color` needs a human at the keyboard between passes; a plan with
/// only one pass never pauses, so it's allowed even without a TTY.
pub fn check_interactive(is_tty: bool, pass_count: usize) -> Result<(), String> {
    if !is_tty && pass_count > 1 {
        return Err("--by-color requires an interactive terminal".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn by_color_plans_from_svg_respects_skip_and_order() {
        let doc = doc_from_svg(two_color_svg()).unwrap();
        let planned = cutplan::plan_passes(&doc).unwrap();
        let colors = pass_order(&planned.passes, &["ff0000ff".into()], Some("0000ffff,ff0000ff".into())).unwrap();
        assert_eq!(colors.len(), 1, "red skipped"); // order flag applied before skip filter
        assert_eq!(colors[0], Some(0x0000FFFF));
    }

    #[test]
    fn out_of_bounds_geometry_is_refused_unless_allowed() {
        // 1512px @96dpi = 400mm wide, past the Cameo's 330mm bed.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect width="1512" height="10" stroke="#ff0000" fill="none"/>
        </svg>"##;

        let err = plan_cut_from_svg(svg, cameo5().as_ref(), &cut_settings(), &[], None, false).unwrap_err();
        assert!(err.contains("outside"), "expected an out-of-bounds refusal, got: {err}");

        assert!(
            plan_cut_from_svg(svg, cameo5().as_ref(), &cut_settings(), &[], None, true).is_ok(),
            "--allow-out-of-bounds must let it through",
        );
    }

    #[test]
    fn settings_out_of_range_are_refused_before_reaching_the_machine() {
        let bad = Settings { speed: Some(99), force: None, repeat_count: 1 };
        let err = plan_cut_from_svg(two_color_svg(), cameo5().as_ref(), &bad, &[], None, false).unwrap_err();
        assert!(err.contains("speed"), "expected a settings-range refusal, got: {err}");
    }

    /// A fill-only SVG used to be refused by name here, because `plan_passes` cut only stroked
    /// shapes. Since #144 `--by-color` plans it, keyed on the fill, and the refusal it used to
    /// produce belongs to an SVG with no geometry at all — which
    /// `plain_cut_of_an_empty_svg_says_nothing_to_cut` covers.
    #[test]
    fn a_fill_only_svg_is_planned_by_color_on_its_fill() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect width="5" height="5" fill="#ff0000"/>
        </svg>"##;
        let plan = plan_cut_from_svg(svg, cameo5().as_ref(), &cut_settings(), &[], None, false).unwrap();
        assert_eq!(plan.passes.len(), 1);
        assert_eq!(plan.passes[0].color, Some(0xFF0000FF));
    }

    #[test]
    fn noninteractive_multicolor_is_error() {
        assert_eq!(
            check_interactive(false, 2),
            Err("--by-color requires an interactive terminal".into())
        );
        assert!(check_interactive(false, 1).is_ok());
        assert!(check_interactive(true, 2).is_ok());
    }

    #[test]
    fn parse_hex_color_requires_eight_digits() {
        assert_eq!(parse_hex_color("ff0000ff"), Ok(0xFF0000FF));
        assert!(parse_hex_color("ff0000").is_err(), "6-digit RRGGBB must be rejected, not zero-padded");
        assert!(parse_hex_color("nothex12").is_err());
    }

    /// A fill-only SVG is what Illustrator, Inkscape and most clipart emit. The plain
    /// cut path has always cut it, so routing that path through `plan_passes` — which
    /// skips strokeless shapes — must not change what it cuts.
    #[test]
    fn fill_only_svg_plans_exactly_one_pass() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <rect width="5" height="5" fill="#ff0000"/><rect x="6" width="5" height="5" fill="#00ff00"/></svg>"##;
        let doc = doc_from_svg_all_cuttable(svg).expect("import");
        let planned = cutplan::plan_passes(&doc).expect("plan");
        assert_eq!(planned.passes.len(), 1, "all geometry belongs to one pass");
        assert_eq!(planned.passes[0].color, Some(CUT_STROKE));
    }

    #[test]
    fn plain_cut_plans_one_pass() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <rect width="5" height="5" fill="#ff0000"/></svg>"##;
        let plan = plan_plain_cut(svg, cameo5().as_ref(), &Settings::default(), false).expect("plan");
        assert_eq!(plan.passes.len(), 1);
    }

    /// The whole point of the change: the plain path is preflighted. A shape past the
    /// bed's edge was silently sent to the machine before.
    #[test]
    fn plain_cut_refuses_out_of_bounds_geometry() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10000mm" height="10mm">
            <rect x="9000" width="500" height="5" fill="#000000"/></svg>"##;
        let err = plan_plain_cut(svg, cameo5().as_ref(), &Settings::default(), false)
            .expect_err("out of bounds must be refused");
        assert!(err.contains("outside"), "unexpected message: {err}");
        // ...and the escape hatch works, now that there is a check to overrule.
        assert!(plan_plain_cut(svg, cameo5().as_ref(), &Settings::default(), true).is_ok());
    }

    /// With no paths at all, `plan_passes` yields no passes, so the requested colour
    /// matches nothing. Without the empty check that surfaces as `UnknownPassColor`,
    /// which reads as an internal error rather than "there is nothing here".
    #[test]
    fn plain_cut_of_an_empty_svg_says_nothing_to_cut() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"></svg>"##;
        let err = plan_plain_cut(svg, cameo5().as_ref(), &Settings::default(), false).expect_err("empty");
        assert_eq!(err, "no cuttable paths in SVG");
    }

    /// `--skip-color` and `--order` name colours, and a plain cut deliberately
    /// collapses every colour into one pass. Accepting them silently — which is what
    /// happened before — reports success for a flag that did nothing.
    #[test]
    fn colour_flags_are_refused_without_by_color() {
        let red = vec!["FF0000FF".to_string()];
        let err = check_color_flag_scope(&red, &None, false).expect_err("must refuse");
        assert!(err.contains("--skip-color"), "unexpected message: {err}");
        let err = check_color_flag_scope(&[], &Some("FF0000FF".into()), false).expect_err("must refuse");
        assert!(err.contains("--order"), "unexpected message: {err}");
        // Both are fine with --by-color, and absence is fine either way.
        assert!(check_color_flag_scope(&red, &Some("FF0000FF".into()), true).is_ok());
        assert!(check_color_flag_scope(&[], &None, false).is_ok());
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
}
