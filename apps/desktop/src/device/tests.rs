// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use cutplan::{plan_passes_with, DocumentPasses};
use driver_core::{Driver, Job, MachineCaps, MachineProfile, Phase, Transport, TransportError, TransportKind};

struct TestDriver { profile: MachineProfile, caps: MachineCaps }
impl Driver for TestDriver {
    fn profile(&self) -> &MachineProfile { &self.profile }
    fn caps(&self) -> MachineCaps { self.caps }
    fn session_begin(&self) -> Vec<u8> { vec![] }
    fn encode_pass(&self, _pass: &Job) -> Result<Vec<u8>, driver_core::DriverError> { Ok(vec![]) }
    fn pass_park(&self) -> Vec<u8> { vec![] }
    fn session_end(&self) -> Vec<u8> { vec![] }
    fn abort_bytes(&self) -> Option<Vec<u8>> { None }
}

struct TestFactory;
impl DeviceBackendFactory for TestFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> { vec![test_instance()] }
    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
        // Mirrors the real registry: an id nobody claims resolves to nothing,
        // rather than silently handing back some other machine's encoder.
        if machine_id != "cameo5" {
            return None;
        }
        Some(Box::new(TestDriver {
            profile: MachineProfile { id: "cameo5".into(), name: "Test Cameo".into(), width_mm: 500.0, height_mm: 500.0 },
            // A machine that cannot be polled parks the cut at `AwaitingConfirmation`
            // instead of driving it to completion, so a cut submitted here stops at a
            // stable mid-flight phase. `MockTransport` answers no status query, so a
            // pollable machine would instead sit out the manager's 60s completion
            // budget and then fail.
            caps: MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true },
        }))
    }
    fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
        Ok(Box::new(driver_core::MockTransport::default()))
    }
}

fn test_instance() -> DeviceInfo {
    DeviceInfo {
        instance_id: "usb:1:4".into(),
        machine_id: "cameo5".into(),
        transport: TransportKind::Usb { locator: "1:4".into() },
        candidate: false,
        host: None,
    }
}

fn test_device_setup() -> DeviceManagerHandle {
    let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
    dev.connect(test_instance()).unwrap();
    dev
}

fn plan_for(app: &AppState) -> DocumentPasses {
    plan_passes_with(&app.editor.doc, Grouping::Color).unwrap()
}

fn request_from(plan: DocumentPasses) -> CutRequest {
    CutRequest {
        device_instance_id: test_instance().instance_id,
        doc_revision: plan.doc_revision.to_string(),
        // The mode the passes were planned under. `plan_for` uses colour grouping, so
        // this must too, or every request here would be refused as an unknown key.
        grouping: Grouping::Color,
        weed: None,
        passes: plan.passes.iter().map(|p| ConfiguredPassDto {
            key: p.key.clone(), enabled: true, preset_id: None,
            speed: None, force: None, repeat_count: None,
        }).collect(),
    }
}

fn colour(c: u32) -> PassKey { PassKey::Color(Some(c)) }

const GREEN: u32 = 0x00FF00FF;

/// Paint an existing shape, so a fixture can carry a stroke and a fill that key
/// differently under `Stroke` and `Fill`.
fn paint(app: &mut AppState, id: document::NodeId, stroke: Option<u32>, fill: Option<u32>) {
    let before = app.editor.doc.get(id).unwrap().clone();
    let mut after = before.clone();
    after.style = document::Style { stroke, fill };
    app.editor.doc.apply(document::Delta(vec![document::NodeOp::Update { id, before, after }]));
}

#[test]
fn cut_request_with_stale_revision_is_rejected() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    app.add_rect(10.0, 10.0);
    let plan = plan_for(&app);
    app.add_rect(5.0, 5.0);
    let err = dev.cut_from_request(&app, request_from(plan)).unwrap_err();
    assert_eq!(err.code, "stale_plan");
}

#[test]
fn preflight_failures_map_to_ipc_codes() {
    let app = AppState::new();
    let dev = test_device_setup();
    let revision = cutplan::doc_revision(&app.editor.doc);
    let request = CutRequest { device_instance_id: test_instance().instance_id,
        doc_revision: revision.to_string(), grouping: Grouping::Color, weed: None, passes: vec![] };
    let err = dev.cut_from_request(&app, request).unwrap_err();
    assert_eq!(err.code, "nothing_to_cut");
}

#[test]
fn an_unknown_pass_is_rejected_not_dropped() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    app.add_rect(10.0, 10.0);
    let plan = plan_for(&app);
    let mut request = request_from(plan);
    request.passes[0].key = colour(0xDEADBEEF); // doesn't match any planned pass
    let err = dev.cut_from_request(&app, request).unwrap_err();
    assert_eq!(err.code, "unknown_pass");
}

const RED: u32 = 0xFF0000FF;
const BLUE: u32 = 0x0000FFFF;

/// A stroked rect offset along x, so travel direction between shapes is assertable.
fn add_stroked_rect(app: &mut AppState, stroke: u32, x: f64) {
    use document::{Delta, Node, NodeOp, ShapeKind, Style};
    let id = app.editor.doc.ids.next();
    let mut node = Node::shape(id, ShapeKind::Rect { w: 10.0, h: 10.0 });
    node.style = Style { stroke: Some(stroke), fill: None };
    node.transform = geometry::Affine::translate(x, 0.0);
    app.editor.commit(Delta(vec![NodeOp::Add { parent: app.editor.doc.root, node, index: usize::MAX }]));
}

fn two_color_doc() -> (AppState, String) {
    let mut app = AppState::new();
    add_stroked_rect(&mut app, RED, 0.0);
    add_stroked_rect(&mut app, BLUE, 100.0);
    let revision = cutplan::doc_revision(&app.editor.doc).to_string();
    (app, revision)
}

fn on(key: PassKey) -> TravelPassDto { TravelPassDto { key, enabled: true } }
fn off(key: PassKey) -> TravelPassDto { TravelPassDto { key, enabled: false } }

#[test]
fn travel_for_order_follows_the_requested_order() {
    let (app, revision) = two_color_doc();

    // The plan's own first-seen order reproduces exactly what plan_cut_response sent.
    let planned = travel_for_order(&app.editor.doc, &revision, Grouping::Color, None, &[on(colour(RED)), on(colour(BLUE))]).unwrap();
    assert_eq!(planned, plan_cut_response(&app.editor.doc, Grouping::Color, None).unwrap().travel);
    assert_eq!(planned.len(), 1);
    assert!(planned[0][2] >= 100.0, "red first: travel lands on the blue rect at x=100, got {planned:?}");

    let reversed = travel_for_order(&app.editor.doc, &revision, Grouping::Color, None, &[on(colour(BLUE)), on(colour(RED))]).unwrap();
    assert_eq!(reversed.len(), 1);
    assert!(reversed[0][0] >= 100.0 && reversed[0][2] <= 10.0,
        "blue first: travel leaves x=100 for the red rect at the origin, got {reversed:?}");
}

/// The head does not travel to a pass that will not be cut. A disabled pass is still
/// named, so the completeness check below survives.
#[test]
fn travel_for_order_skips_a_disabled_pass() {
    let (app, revision) = two_color_doc();
    let travel = travel_for_order(&app.editor.doc, &revision, Grouping::Color, None, &[on(colour(RED)), off(colour(BLUE))]).unwrap();
    assert!(travel.is_empty(), "nothing to travel to with only one pass cut, got {travel:?}");

    // And with everything off there is no motion at all.
    let none = travel_for_order(&app.editor.doc, &revision, Grouping::Color, None, &[off(colour(RED)), off(colour(BLUE))]).unwrap();
    assert!(none.is_empty(), "no pass is cut, so the head does not move: {none:?}");
}

#[test]
fn travel_for_order_with_a_stale_revision_is_refused() {
    let (mut app, revision) = two_color_doc();
    app.add_rect(5.0, 5.0);
    let err = travel_for_order(&app.editor.doc, &revision, Grouping::Color, None, &[on(colour(RED)), on(colour(BLUE))]).unwrap_err();
    assert_eq!(err.code, "stale_plan");
}

#[test]
fn travel_for_order_with_an_unknown_key_is_refused() {
    let (app, revision) = two_color_doc();
    let unknown = TravelPassDto { key: colour(0xDEADBEEF), enabled: true };
    let err = travel_for_order(&app.editor.doc, &revision, Grouping::Color, None, &[on(colour(RED)), unknown]).unwrap_err();
    assert_eq!(err.code, "unknown_pass");
}

/// Disabling a pass drops it from the travel, never from the list — a pass genuinely
/// missing is a frontend bug, and silently drawing travel around it would hide it.
#[test]
fn travel_for_order_missing_a_planned_pass_is_refused() {
    let (app, revision) = two_color_doc();
    let err = travel_for_order(&app.editor.doc, &revision, Grouping::Color, None, &[on(colour(RED))]).unwrap_err();
    assert_eq!(err.code, "plan_mismatch");
}

#[test]
fn travel_for_order_naming_a_pass_twice_is_a_mismatch_not_an_unknown_pass() {
    let (app, revision) = two_color_doc();
    let err = travel_for_order(&app.editor.doc, &revision, Grouping::Color, None, &[on(colour(RED)), on(colour(RED))]).unwrap_err();
    assert_eq!(err.code, "plan_mismatch");
}

#[test]
fn plan_cut_response_carries_each_shapes_first_world_point() {
    let (app, _) = two_color_doc();
    let response = plan_cut_response(&app.editor.doc, Grouping::Color, None).unwrap();
    for pass in &response.passes {
        assert_eq!(pass.starts.len(), pass.node_ids.len(), "starts is parallel to node_ids");
    }
    // The blue rect's translate must show in its start — a local-space point here
    // would put the badge at the origin instead of on the shape.
    let blue = response.passes.iter().find(|p| p.key == colour(BLUE)).unwrap();
    let start = blue.starts[0].unwrap();
    assert!(start[0] >= 100.0, "world-space start, got {start:?}");
}

fn border_only() -> WeedOptions {
    WeedOptions { margin_mm: 3.0, lines: cutplan::weed::WeedLines::None, spacing_mm: 25.0, clearance_mm: 1.5 }
}

/// Moves a shape to (x, y), so a weed border around it stays on the test machine's mat.
fn place(app: &mut AppState, id: document::NodeId, x: f64, y: f64) {
    let before = app.editor.doc.get(id).unwrap().clone();
    let mut after = before.clone();
    after.transform = geometry::Affine::translate(x, y);
    app.editor.doc.apply(document::Delta(vec![document::NodeOp::Update { id, before, after }]));
}

#[test]
fn a_weeded_plan_carries_each_pass_border_for_the_preview() {
    let (app, _) = two_color_doc();
    let response = plan_cut_response(&app.editor.doc, Grouping::Color, Some(&border_only())).unwrap();
    for pass in &response.passes {
        assert_eq!(pass.weed.len(), 1, "{}: the border alone", pass.key);
        assert_eq!(pass.weed[0].len(), 5, "a closed rectangle");
    }
    // Each pass's own 10 mm rect, grown by the 3 mm margin: red at x = 0, blue at x = 100.
    let border = |key: PassKey| response.passes.iter().find(|p| p.key == key).unwrap().weed[0].clone();
    assert_eq!(border(colour(RED)), vec![[-3.0, -3.0], [13.0, -3.0], [13.0, 13.0], [-3.0, 13.0], [-3.0, -3.0]]);
    assert_eq!(border(colour(BLUE))[2], [113.0, 13.0]);
    let plain = plan_cut_response(&app.editor.doc, Grouping::Color, None).unwrap();
    assert!(plain.passes.iter().all(|p| p.weed.is_empty()));
}

#[test]
fn weeded_travel_goes_to_each_border_after_its_pass_shapes() {
    let (app, revision) = two_color_doc();
    let order = [on(colour(RED)), on(colour(BLUE))];
    let plain = travel_for_order(&app.editor.doc, &revision, Grouping::Color, None, &order).unwrap();
    let weeded = travel_for_order(&app.editor.doc, &revision, Grouping::Color, Some(&border_only()), &order).unwrap();
    assert_eq!(weeded.len(), plain.len() + 2, "one more stop per pass: its border");
    // The preview's first travel and the replanned one agree.
    assert_eq!(weeded, plan_cut_response(&app.editor.doc, Grouping::Color, Some(&border_only())).unwrap().travel);
}

/// The reason the options travel with the request: the cut is built from what it was sent.
#[test]
fn a_cut_cuts_the_weed_it_was_sent() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    let id = app.add_rect(10.0, 10.0);
    place(&mut app, id, 50.0, 50.0);
    let (_, plain) = dev.prepare_cut(&app, request_from(plan_for(&app))).unwrap();
    let mut request = request_from(plan_for(&app));
    request.weed = Some(border_only());
    let (_, weeded) = dev.prepare_cut(&app, request).unwrap();

    assert_eq!(weeded[0].job.polylines.len(), plain[0].job.polylines.len() + 1);
    let border = weeded[0].job.polylines.last().unwrap();
    let xs = border.iter().map(|p| p.x);
    assert_eq!(xs.clone().fold(f64::MAX, f64::min), 47.0);
    assert_eq!(xs.fold(f64::MIN, f64::max), 63.0);
}

#[test]
fn a_weed_border_off_the_mat_is_refused_as_out_of_bounds() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    app.add_rect(10.0, 10.0); // at the origin, so a 3 mm border reaches -3
    let mut request = request_from(plan_for(&app));
    request.weed = Some(border_only());
    let err = dev.prepare_cut(&app, request).err().expect("refused");
    assert_eq!(err.code, "out_of_bounds");
    assert!(err.message.contains("weed border"), "{}", err.message);
}

#[test]
fn weed_options_out_of_range_are_refused_with_their_sentence() {
    let (app, _) = two_color_doc();
    let bad = WeedOptions { margin_mm: 99.0, ..border_only() };
    let err = plan_cut_response(&app.editor.doc, Grouping::Color, Some(&bad)).err().expect("refused");
    assert_eq!(err.code, "plan_error");
    assert_eq!(err.message, "the weed margin must be 0.5–50 mm");
}

/// The grouping the dialog asked for is the grouping that gets cut. Without it the
/// operator could preview a fill-grouped plan and cut a stroke-grouped one, because each
/// command plans the document itself.
#[test]
fn a_cut_honours_the_grouping_it_was_sent() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    // A red stroke over a green fill: the two colour modes key this shape differently,
    // so the request's grouping is observable in what matches.
    let id = app.add_rect(10.0, 10.0);
    paint(&mut app, id, Some(RED), Some(GREEN));
    let revision = plan_cut_response(&app.editor.doc, Grouping::Fill, None).unwrap().doc_revision;

    let request = CutRequest {
        device_instance_id: test_instance().instance_id,
        doc_revision: revision,
        grouping: Grouping::Fill,
        weed: None,
        passes: vec![ConfiguredPassDto {
            key: colour(RED), enabled: true, preset_id: None,
            speed: None, force: None, repeat_count: None }],
    };
    // Fill grouping keys that shape on its fill, so the stroke's key names nothing.
    assert_eq!(dev.cut_from_request(&app, request).unwrap_err().code, "unknown_pass");
}

/// Travel is replanned with the same grouping, for the same reason.
#[test]
fn travel_honours_the_grouping_it_was_sent() {
    let mut app = AppState::new();
    let id = app.add_rect(10.0, 10.0);
    paint(&mut app, id, Some(RED), Some(GREEN));
    let revision = plan_cut_response(&app.editor.doc, Grouping::Fill, None).unwrap().doc_revision;

    assert!(travel_for_order(&app.editor.doc, &revision, Grouping::Fill, None,
        &[on(colour(GREEN))]).is_ok());
    assert_eq!(travel_for_order(&app.editor.doc, &revision, Grouping::Fill, None,
        &[on(colour(RED))]).unwrap_err().code, "unknown_pass");
}

/// The response names its passes in the spelling a request must send back.
#[test]
fn a_plan_response_names_its_passes_by_key() {
    let mut app = AppState::new();
    app.add_rect(10.0, 10.0);
    let response = plan_cut_response(&app.editor.doc, Grouping::Single, None).unwrap();
    assert_eq!(response.passes[0].key, PassKey::All);
}

/// A preset-keyed pass is cut with that preset's settings. This is the whole point of
/// grouping by material: `prepare_cut` reads only `preset_id`, so a row that arrives with
/// none is cut with defaults no matter what its key says.
#[test]
fn a_preset_keyed_pass_cuts_with_that_presets_settings() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    let id = app.add_rect(10.0, 10.0);
    app.set_material_preset(vec![id], document::PresetAssignment::Preset("cameo5-htv".into()))
        .expect("assignable");
    let revision = plan_cut_response(&app.editor.doc, Grouping::Preset, None).unwrap().doc_revision;

    let request = CutRequest {
        device_instance_id: test_instance().instance_id,
        doc_revision: revision,
        grouping: Grouping::Preset,
        weed: None,
        passes: vec![ConfiguredPassDto {
            key: PassKey::Preset(Some("cameo5-htv".into())),
            enabled: true,
            // What the dialog sends for a preset-keyed row: the key's own id.
            preset_id: Some("cameo5-htv".into()),
            speed: None, force: None, repeat_count: None }],
    };
    let (_, passes) = dev.prepare_cut(&app, request).expect("planned");
    let builtin = cutplan::presets::builtin_presets().into_iter()
        .find(|p| p.id == "cameo5-htv").expect("premise: the builtin exists");
    assert_eq!(passes[0].job.settings.speed, builtin.settings.speed);
    assert_eq!(passes[0].job.settings.force, builtin.settings.force);
}

/// Greptile's P1 on PR #152 read this as "a preset key silently uses defaults". It is the
/// operator clearing the preset on a row: the key says which shapes share the pass, the row's
/// `preset_id` says what settings to cut them with, and those are deliberately two fields. A
/// colour-keyed pass has no preset either and cuts with the operator's settings - the ordinary
/// case nobody calls a bug. Refusing here would make preset-keyed passes the only kind that
/// cannot be cut with settings of the operator's own choosing.
///
/// It is not silent: the row shows "No preset" where it showed the material, and the speed and
/// force fields show what will be used. The refusal below is for an id that *cannot resolve* -
/// the file changing under the operator - which is a different fact from a choice.
#[test]
fn a_preset_keyed_pass_with_the_preset_cleared_cuts_with_the_operators_settings() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    let id = app.add_rect(10.0, 10.0);
    app.set_material_preset(vec![id], document::PresetAssignment::Preset("cameo5-htv".into()))
        .expect("assignable");
    let revision = plan_cut_response(&app.editor.doc, Grouping::Preset, None).unwrap().doc_revision;

    let request = CutRequest {
        device_instance_id: test_instance().instance_id,
        doc_revision: revision,
        grouping: Grouping::Preset,
        weed: None,
        passes: vec![ConfiguredPassDto {
            key: PassKey::Preset(Some("cameo5-htv".into())),
            enabled: true,
            // The operator picked "No preset" on a row keyed by a material.
            preset_id: None,
            speed: Some(7), force: None, repeat_count: None }],
    };
    let (_, passes) = dev.prepare_cut(&app, request).expect("planned, not refused");
    let builtin = cutplan::presets::builtin_presets().into_iter()
        .find(|p| p.id == "cameo5-htv").expect("premise: the builtin exists");
    assert_eq!(passes[0].job.settings.speed, Some(7), "the operator's own speed is used");
    // Force is what discriminates, and speed cannot: an override beats a preset either way
    // (`presets.rs:44`), so a speed assertion passes even if the preset were wrongly
    // re-derived from the key. Force was left unset, so it says which of the two happened.
    assert!(builtin.settings.force.is_some(),
        "premise: HTV states a force, so re-deriving it from the key would show here");
    assert_eq!(passes[0].job.settings.force, None,
        "no preset was selected, so no preset's force is applied");
}

/// A pass naming a preset the file cannot resolve is refused rather than cut with
/// defaults. Greptile's P1 on PR #152: a machine-scoped preset disappears for ordinary
/// reasons — the project was converted, the entry was deleted — and cutting real material
/// with settings unrelated to the pass's own name is the failure that costs a sheet.
#[test]
fn a_pass_naming_an_unavailable_preset_is_refused_not_defaulted() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    let id = app.add_rect(10.0, 10.0);
    app.set_material_preset(vec![id], document::PresetAssignment::Preset("deleted-by-hand".into()))
        .expect("assignable");
    let revision = plan_cut_response(&app.editor.doc, Grouping::Preset, None).unwrap().doc_revision;

    let request = CutRequest {
        device_instance_id: test_instance().instance_id,
        doc_revision: revision,
        grouping: Grouping::Preset,
        weed: None,
        passes: vec![ConfiguredPassDto {
            key: PassKey::Preset(Some("deleted-by-hand".into())),
            enabled: true,
            preset_id: Some("deleted-by-hand".into()),
            speed: None, force: None, repeat_count: None }],
    };
    let err = dev.prepare_cut(&app, request).unwrap_err();
    assert_eq!(err.code, "unknown_preset");
    assert!(err.message.contains("deleted-by-hand"), "names the preset it cannot find: {}", err.message);
}

/// The fail-open Codex found on the gate re-run, from the other end: a pass keyed `preset:`
/// carries an *empty* preset id, and an empty id resolves to no preset. The refusal has to
/// fire — before this, the TypeScript mirror dropped the id, the request named no preset at
/// all, and `prepare_cut` skipped its lookup and produced default speed and force.
#[test]
fn a_pass_keyed_on_an_empty_preset_id_is_refused_not_defaulted() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    app.add_rect(10.0, 10.0);
    let revision = plan_cut_response(&app.editor.doc, Grouping::Preset, None).unwrap().doc_revision;

    let request = CutRequest {
        device_instance_id: test_instance().instance_id,
        doc_revision: revision,
        grouping: Grouping::Preset,
        weed: None,
        passes: vec![ConfiguredPassDto {
            // What the dialog sends for a `preset:` row now that both grammars parse it.
            key: PassKey::Preset(Some(String::new())),
            enabled: true,
            preset_id: Some(String::new()),
            speed: None, force: None, repeat_count: None }],
    };
    let err = dev.prepare_cut(&app, request).unwrap_err();
    assert_eq!(err.code, "unknown_preset",
        "an empty id names no material, so the cut is refused rather than defaulted");
}

/// A preset belonging to another cutter is not this cut's preset. Greptile's P1 on the third
/// push: `load_presets` returns every machine's entries, and while builtin ids are
/// machine-prefixed, a *user* preset's id is the operator's own string — so a Puma entry
/// could resolve for a Cameo cut and hand it that machine's speed and force.
///
/// `puma-htv` stands in for the collision: it is a real builtin for the other machine, so
/// the lookup finds it if and only if the machine filter is missing. (Greptile could not run
/// this itself — its container lacked `gdk-3.0.pc` — so it is written here from its
/// finding.)
#[test]
fn a_preset_owned_by_a_different_machine_is_refused() {
    let mut app = AppState::new();
    let dev = test_device_setup(); // connects `cameo5`
    let id = app.add_rect(10.0, 10.0);
    app.set_material_preset(vec![id], document::PresetAssignment::Preset("puma-htv".into()))
        .expect("assignable");
    let revision = plan_cut_response(&app.editor.doc, Grouping::Preset, None).unwrap().doc_revision;

    let request = CutRequest {
        device_instance_id: test_instance().instance_id,
        doc_revision: revision,
        grouping: Grouping::Preset,
        weed: None,
        passes: vec![ConfiguredPassDto {
            key: PassKey::Preset(Some("puma-htv".into())),
            enabled: true,
            preset_id: Some("puma-htv".into()),
            speed: None, force: None, repeat_count: None }],
    };
    let err = dev.prepare_cut(&app, request).unwrap_err();
    assert_eq!(err.code, "unknown_preset",
        "a Puma preset must not supply settings for a Cameo cut");

    // Premise: that id really is a preset — just not one for this machine.
    assert!(cutplan::presets::builtin_presets().iter().any(|p| p.id == "puma-htv"),
        "premise: puma-htv is a builtin, so only the machine filter can refuse it");
}

/// The bridge used to synthesize `Transmitting` from `Progress` because the
/// worker never re-emitted a state mid-transmit. `CutStatus` carries the
/// progress in the phase itself, so the synthesis has nothing left to do —
/// note that nothing here consumes the event channel at all.
#[test]
fn status_reports_a_mid_flight_cut_without_the_bridge_synthesizing_it() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    app.add_rect(10.0, 10.0);
    let plan = plan_for(&app);
    dev.cut_from_request(&app, request_from(plan)).expect("cut");
    let s = dev.status();
    // The exact phase, not merely a mid-flight one: a fixture that let the cut
    // run to completion would leave nothing in flight for the status to report,
    // and this test would then pass without ever exercising the question.
    assert_eq!(s.phase, Phase::AwaitingConfirmation, "the cut should be parked mid-flight");
    assert!(s.is_active(), "a parked cut is what makes the close guard block a quit");
}

#[test]
fn status_without_a_manager_reads_disconnected() {
    let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
    dev.shutdown();
    assert_eq!(dev.status().phase, Phase::Disconnected);
}

#[test]
fn caps_for_returns_the_drivers_own_answer() {
    let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
    let caps = dev.caps_for("cameo5").expect("known machine id");
    assert_eq!(
        caps,
        MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true }
    );
}

#[test]
fn caps_for_unknown_machine_is_an_error_not_a_default() {
    let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
    let err = dev.caps_for("nope").expect_err("no driver claims this id");
    assert_eq!(err.code, "unknown_machine");
}

/// The UI reads `caps.supportsSpeed`. Drop the serde rename and it reads
/// `undefined`, `!undefined` is `true`, and every field greys out on every
/// machine — silent, and wrong in the direction that looks plausible.
#[test]
fn machine_caps_serializes_in_the_casing_the_ui_reads() {
    let json = serde_json::to_value(MachineCaps {
        supports_speed: true,
        supports_force: false,
        needs_operator_pass_confirm: true,
    })
    .unwrap();
    assert_eq!(json["supportsSpeed"], serde_json::json!(true));
    assert_eq!(json["supportsForce"], serde_json::json!(false));
    assert_eq!(json["needsOperatorPassConfirm"], serde_json::json!(true));
}

use crate::hosts::PairedHost;
use driver_core::HostId;

fn a_paired_host(id: &str, addr: &str) -> PairedHost {
    PairedHost {
        id: HostId(id.into()),
        name: "Workshop Pi".into(),
        address: addr.into(),
        fingerprint: "aa:bb:cc".into(),
        token: "s3cret".into(),
    }
}

/// A user who never pairs a Pi must see exactly what they see today. This is the test that
/// says the feature is optional by construction rather than by intention.
#[test]
fn with_no_host_paired_the_device_list_is_the_local_one() {
    let dev = test_device_setup();
    let listed = dev.list_devices();

    // Not just "nothing is host-tagged" — an empty list satisfies that vacuously, and the
    // regression this guards against is exactly one that returns nothing.
    assert!(!listed.is_empty(), "the local factory's devices must still be listed");
    assert_eq!(listed.len(), TestFactory.list_devices().len());
    assert!(listed.iter().all(|d| d.host.is_none()), "{listed:?}");
}

/// A host that cannot be reached keeps its place in the list rather than vanishing — a
/// cutter that disappears looks like one that was never paired.
#[test]
fn an_unreachable_host_is_still_listed_with_its_reason() {
    let dev = test_device_setup();
    // Nothing is listening on this port, so connecting fails.
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

    // `add_host` records the host without dialling it — connections are lazy — so the
    // failure only exists once something asks for its cutters.
    let listed = dev.list_devices();
    assert!(listed.iter().all(|d| d.host.is_none()), "an unreachable host contributes none");

    let views = dev.host_views();
    assert_eq!(views.len(), 1, "the host stays known: {views:?}");
    assert!(views[0].unreachable.is_some(), "and says why it is unreachable");
}

/// A host this desktop cannot speak to stays listed, and the row says which side to update.
/// The cutters stay off the list: nothing was admitted, so there is nothing to show as cutting.
#[test]
fn a_protocol_mismatch_names_which_side_to_update_and_lists_no_cutter() {
    let older = cut_host::client::testing::start_unversioned_host();
    let newer = cut_host::client::testing::start_host_at_version(
        cut_host::protocol::PROTOCOL_VERSION + 1,
        vec![],
    );
    let dev = test_device_setup();
    dev.add_host(PairedHost {
        fingerprint: older.fingerprint.clone(),
        ..a_paired_host("host-old", &older.addr)
    });
    dev.add_host(PairedHost {
        id: HostId("host-new".into()),
        name: "Office Pi".into(),
        address: newer.addr.clone(),
        fingerprint: newer.fingerprint.clone(),
        token: "s3cret".into(),
    });

    let listed = dev.list_devices();
    assert!(
        listed.iter().all(|d| d.host.is_none()),
        "a mismatched host contributes no cutter: {listed:?}"
    );

    let views = dev.host_views();
    let message = |id: &str| {
        views
            .iter()
            .find(|v| v.id.0 == id)
            .and_then(|v| v.unreachable.clone())
    };
    assert_eq!(
        message("host-old").as_deref(),
        Some("This Cut Host is running an older version. Update your Cut Host to connect.")
    );
    assert_eq!(
        message("host-new").as_deref(),
        Some("This version of Cuthulhu is older than the Cut Host. Update Cuthulhu to connect.")
    );
    let side = |id: &str| {
        views
            .iter()
            .find(|v| v.id.0 == id)
            .and_then(|v| v.protocol_side)
    };
    assert_eq!(side("host-old"), Some(cut_host::client::ProtocolSide::Host));
    assert_eq!(side("host-new"), Some(cut_host::client::ProtocolSide::Desktop));
}

/// A cutter with no host is this computer's, and must reach the local DeviceManager — the
/// path every existing user is on.
#[test]
fn a_local_device_still_routes_to_the_local_manager() {
    let dev = test_device_setup();
    assert_eq!(dev.status().phase, driver_core::Phase::Idle);
    assert!(dev.cancel().is_ok(), "a local cancel reaches the local manager");
}

/// A moving blade must not become unstoppable: the window-close guard and `cancel` both act
/// on the *current* aim, so letting `connect` move it to a host mid-cut would deafen the
/// guard (it would see the host's `Idle`) and mis-route `cancel` to a machine that was never
/// asked to stop. Mirrors `forget`'s own busy refusal.
#[test]
fn connect_refuses_to_move_the_aim_off_an_active_local_cut() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    app.add_rect(10.0, 10.0);
    let plan = plan_for(&app);
    dev.cut_from_request(&app, request_from(plan)).expect("cut");
    assert!(dev.status().is_active(), "the local cut must be parked mid-flight for this test to mean anything");

    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let elsewhere = DeviceInfo { host: Some(HostId("host-1".into())), ..test_instance() };
    let err = dev.connect(elsewhere).unwrap_err();
    assert_eq!(err.code, "device_error", "got {err:?}");

    // A refused connect is not a half-connect: the aim must not have moved either.
    assert_eq!(dev.connected.lock().unwrap().as_ref().unwrap().host, None, "aim must stay local");
}

/// Aiming at a host used to leave the local `DeviceManager` connected and `Idle` — and
/// `DeviceManager::connect` refuses anything but `Disconnected`/`Error` — so re-aiming
/// locally afterward returned `Busy`, stranding the operator's own cutter for the rest of
/// the session. `connect`'s `Route::Host` arm must release the local manager first.
#[test]
fn re_aiming_locally_after_a_host_succeeds() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let elsewhere = DeviceInfo { host: Some(HostId("host-1".into())), ..test_instance() };
    dev.connect(elsewhere).expect("aiming at a host is bookkeeping only");

    dev.connect(test_instance()).expect("must be able to aim back at the local cutter");
}

/// A remote cutter's connection belongs to the Cut Host, not this desktop. Routing
/// `disconnect` unconditionally to the local manager used to close the local cutter's
/// transport and discard its parked job while the operator was aimed at a Pi, silently.
#[test]
fn disconnect_while_aimed_at_a_host_leaves_the_local_manager_untouched() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    dev.connected.lock().unwrap().replace(DeviceInfo { host: Some(HostId("host-1".into())), ..test_instance() });

    dev.disconnect().expect("disconnecting a remote aim is bookkeeping only");
    assert!(dev.connected.lock().unwrap().is_none(), "the aim itself must still clear");

    // Re-aim locally: if `disconnect` had reached the local manager, this would now read
    // `Disconnected` instead of the `Idle` it started at.
    dev.connected.lock().unwrap().replace(test_instance());
    assert_eq!(dev.status().phase, Phase::Idle, "the local manager's connection must have survived untouched");
}

/// A cancel whose stop nothing confirmed refuses another cut, and `DeviceManager::connect`
/// refuses from that state too — so the disconnect is the whole exit, and the dialog now
/// offers one. Asserted through `actions`: the phase is `Idle` either side of the reconnect,
/// which is exactly why it cannot answer this.
#[test]
fn disconnecting_and_reconnecting_makes_a_cut_legal_after_an_unconfirmed_cancel() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    app.add_rect(10.0, 10.0);
    let plan = plan_for(&app);
    dev.cut_from_request(&app, request_from(plan)).expect("cut");

    dev.cancel().expect("cancel");
    // The test Driver parks rather than polls, so nothing can confirm its stop — the Puma's
    // ordinary case, and the one the operator has to be able to get out of.
    let stuck = wait_for_cancelled(&dev);
    assert!(!stuck.actions.cut, "nothing saw the machine stop, so no Job may follow it");

    dev.disconnect().expect("disconnect");
    dev.connect(test_instance()).expect("reconnect");
    assert!(dev.status().actions.cut, "a reconnected cutter accepts a Job again");
}

/// The control the dialog offers is hidden mid-Job, but that guard lives in the webview and
/// reads a status documented to lag the worker by one event. Both verbs drop the local
/// cutter's transport, so the refusal has to be here too — `Host::reconnect` already answers
/// the same question for a remote cutter.
#[test]
fn dropping_the_local_transport_is_refused_while_a_cut_is_working() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    app.add_rect(10.0, 10.0);
    let plan = plan_for(&app);
    dev.cut_from_request(&app, request_from(plan)).expect("cut");
    assert!(dev.status().is_active(), "the Job is parked mid-flight, not finished");

    for refused in [dev.disconnect(), dev.reconnect()] {
        let err = refused.expect_err("a working cutter must keep its transport");
        assert_eq!(err.code, "device_error");
        assert!(err.message.contains("cancel it"), "and must say the way through: {}", err.message);
    }
    // Still parked, and still answerable — the refusal changed nothing.
    assert!(dev.status().actions.confirm);
}

/// The wrong-state refusal, which needs no hardware to reach: `resume` with nothing parked.
/// It used to arrive as `device_error` with `Busy` inside a `Debug` string, indistinguishable
/// to a caller from a cable pull — the two things this issue exists to separate (#73).
#[test]
fn a_verb_the_cutter_cannot_accept_reports_the_busy_code() {
    let dev = test_device_setup();
    let err = dev.resume().unwrap_err();
    assert_eq!(err.code, "device_busy", "got {err:?}");
    assert_eq!(err.message, "the cutter cannot do that right now");
}

/// A cutter whose transport takes the worker thread down as it opens.
///
/// The only way to reach `Disconnected`'s worker-gone site from out here:
/// `DeviceManagerHandle::shutdown` clears the stored manager, so a verb after it reports
/// `shut_down` rather than reaching a dead worker at all, and `DeviceManager`'s own command
/// channel is private to `driver-core`.
struct DeadWorkerFactory;
impl DeviceBackendFactory for DeadWorkerFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> { vec![test_instance()] }
    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
        TestFactory.driver_for(machine_id)
    }
    fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
        panic!("a transport that takes the worker thread with it")
    }
}

/// The other half of the code the eight sites collapsed: nothing refused this verb, there is
/// simply no worker left to answer it. The panic the harness prints is this test working.
#[test]
fn a_verb_with_no_worker_left_reports_the_disconnected_code() {
    let (dev, _events) = DeviceManagerHandle::new(Arc::new(DeadWorkerFactory));
    let err = dev.connect(test_instance()).unwrap_err();
    assert_eq!(err.code, "device_disconnected", "got {err:?}");
    assert_eq!(err.message, "the cutter is not connected");
}

/// A cutter jamming, unplugged or asked for a verb it cannot do must read the same whether it
/// hangs off this laptop or off a Pi. The Cut Host route carries a real `DeviceError` across
/// the wire and `host_error` used to fold it — with every other refusal — into
/// `host_refused`, so the identical fault had two codes and two sentences (#73).
#[test]
fn a_device_fault_from_a_host_reports_what_the_same_fault_reports_locally() {
    use cut_host::client::ClientError;
    use cut_host::protocol::Refusal;

    let dev = test_device_setup();
    let local = dev.resume().unwrap_err();
    let remote = host_error(&ClientError::Refused(Refusal::Device(
        driver_core::manager::DeviceError::Busy,
    )));
    assert_eq!((remote.code, remote.message), (local.code, local.message));

    // And the arm is not a catch-all: a refusal that is the host's own keeps the host's code.
    let its_own = host_error(&ClientError::Refused(Refusal::UnknownDevice("usb:1:4".into())));
    assert_eq!(its_own.code, "host_refused", "got {its_own:?}");
}

/// A host that answered was reached, and the code has to say so. A wrong reply used to arrive
/// as `host_unreachable` with a `Debug` rendering of the reply in the message, so the code
/// claimed the opposite of what had just happened (#283).
#[test]
fn a_reply_a_verb_cannot_use_is_not_reported_as_an_unreachable_host() {
    use cut_host::client::ClientError;

    let err = host_error(&ClientError::WrongReply { expected: "Snapshots", found: "Devices" });
    assert_eq!(err.code, "host_wrong_reply", "got {err:?}");
    assert_eq!(err.message, "this host answered with `Devices` where `Snapshots` was expected");

    // And it did not take the arm next to it with it. A poll that timed out is one example of
    // the many payloads `Transport` carries; what they have in common is that no reply this
    // desktop could use ever arrived.
    let offline = host_error(&ClientError::Transport("timed out".into()));
    assert_eq!(offline.code, "host_unreachable", "got {offline:?}");
    assert_eq!(offline.protocol_side, None);

    // And a version mismatch is not one of those either: the host answered, and the thing to
    // change is a program, not the network.
    let mismatch = host_error(&ClientError::Protocol {
        host_version: None,
        desktop_version: cut_host::protocol::PROTOCOL_VERSION,
    });
    assert_eq!(mismatch.code, "host_protocol", "got {mismatch:?}");
    assert_eq!(mismatch.protocol_side, Some(cut_host::client::ProtocolSide::Host));
    assert_eq!(
        mismatch.message,
        "This Cut Host is running an older version. Update your Cut Host to connect."
    );

    let newer = host_error(&ClientError::Protocol {
        host_version: Some(cut_host::protocol::PROTOCOL_VERSION + 1),
        desktop_version: cut_host::protocol::PROTOCOL_VERSION,
    });
    assert_eq!(newer.code, "host_protocol");
    assert_eq!(newer.protocol_side, Some(cut_host::client::ProtocolSide::Desktop));
    assert_eq!(
        newer.message,
        "This version of Cuthulhu is older than the Cut Host. Update Cuthulhu to connect."
    );
}

fn wait_for_cancelled(dev: &DeviceManagerHandle) -> CutStatus {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let s = dev.status();
        if s.ended == Some(driver_core::Ended::Cancelled) {
            return s;
        }
        assert!(std::time::Instant::now() < deadline, "never cancelled, sat at {:?}", s.phase);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Naming a host that was forgotten (or never paired) must be refused rather than falling
/// back to the local cutter — a Job aimed at a Pi must never be cut on the desk.
#[test]
fn a_device_naming_an_unknown_host_is_refused_not_run_locally() {
    let dev = test_device_setup();
    let elsewhere = DeviceInfo {
        host: Some(HostId("host-does-not-exist".into())),
        ..test_instance()
    };
    let err = dev.connect(elsewhere).unwrap_err();
    assert_eq!(err.code, "unknown_host", "got {err:?}");
}

/// A device aimed at a host that was forgotten (or never paired) must report a status the UI
/// cannot act on. Asserted on `actions`, not `phase` — this project's rule is that a caller
/// learns what is legal from `actions` and never re-derives it from the phase.
#[test]
fn status_for_a_device_on_an_unknown_host_is_not_a_local_idle() {
    let dev = test_device_setup();
    dev.connected.lock().unwrap().replace(DeviceInfo {
        host: Some(HostId("host-does-not-exist".into())),
        ..test_instance()
    });
    assert!(!dev.status().actions.cut, "an unreachable host must not offer a cut it cannot run");
}

#[test]
fn forgetting_a_host_removes_it_and_its_cutters() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    assert_eq!(dev.host_views().len(), 1, "known as soon as it is added");

    dev.remove_host(&HostId("host-1".into()));
    assert!(dev.host_views().is_empty());
    assert!(dev.list_devices().iter().all(|d| d.host.is_none()));
}

/// The token must not leave the Rust side. A view type is the guard, and this is what stops
/// a later refactor from "simplifying" it back to sending `PairedHost`.
#[test]
fn a_host_view_carries_no_token() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

    // `unreachable` is `None` here on purpose: nothing has dialled this host yet, and the
    // view reports what is known rather than provoking a connection to find out.
    let views = dev.host_views();
    assert_eq!(views.len(), 1);
    let json = serde_json::to_string(&views[0]).unwrap();
    assert!(!json.contains("s3cret"), "a token reached the view: {json}");
    assert!(json.contains("host-1"), "the id is what the UI addresses: {json}");
}

#[test]
fn pairing_mints_past_the_highest_id_instead_of_filling_a_gap() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    dev.add_host(a_paired_host("host-3", "127.0.0.1:1"));
    assert_eq!(
        dev.claim_next_host_id().unwrap(),
        HostId("host-4".into()),
        "a gap is not an id made safe to reuse"
    );
}

#[test]
fn an_exhausted_host_id_suffix_is_a_refusal_not_an_overflow() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host(&format!("host-{}", u128::MAX), "127.0.0.1:1"));
    let err = dev.claim_next_host_id().expect_err("there is no larger numeric suffix");
    assert_eq!(err.code, "host_id_exhausted");
}

#[test]
fn a_non_numeric_host_id_does_not_derail_minting() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("imported-by-hand", "127.0.0.1:1"));
    assert_eq!(dev.claim_next_host_id().unwrap(), HostId("host-1".into()));
}

// --- pair()/forget(): network refusals, persist-then-mutate, busy-host refusal ---
//
// `crates/cut-host/tests/fixtures/mod.rs` has this exact fixture already, but it lives in a
// separate integration-test crate and cannot be imported from here — so this is the same
// loopback host, built from the same public `cut_host` API, kept alive by the same
// `_dir: TempDir` trick (dropping it deletes the cert directory `serve_on` reads its key from).

struct LoopbackHost {
    addr: String,
    fingerprint: String,
    _dir: tempfile::TempDir,
}

const HOST_TOKEN: &str = "test-token";

fn start_loopback_host() -> LoopbackHost {
    let dir = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let config = cut_host::config::Config {
        bind: listener.local_addr().unwrap(),
        tokens: [("test-client".to_string(), HOST_TOKEN.to_string())].into_iter().collect(),
        max_frame: cut_host::frame::DEFAULT_MAX_FRAME,
        cert_dir: dir.path().to_path_buf(),
    };
    // Generated before `Host::start`, so the client has something to pin before the
    // server has accepted anything.
    let fingerprint = cut_host::serve::fingerprint_of_cert_dir(&config.cert_dir).unwrap();
    let host = cut_host::host::Host::start(Arc::new(cut_host::host::testing::TwoCutterFactory));

    std::thread::spawn(move || {
        let _ = cut_host::serve::serve_on(listener, host, config);
    });
    LoopbackHost { addr, fingerprint, _dir: dir }
}

/// A regular file where a directory needs to be, so `create_dir_all` — and therefore every
/// `hosts::save` — fails without touching real filesystem permissions.
fn unwritable_hosts_path(dir: &std::path::Path) -> std::path::PathBuf {
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, b"").unwrap();
    blocker.join("hosts.json")
}

/// The code, not just the failure. A rejected token, a changed certificate and an offline Pi
/// call for three different things from the operator, and all three used to arrive as
/// `host_unreachable` with prose that could not be told apart (#112).
#[test]
fn pairing_with_the_wrong_token_is_refused_and_saves_nothing() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let err = dev
        .pair("Pi".into(), host.addr.clone(), "wrong-token".into(), host.fingerprint.clone(), &hosts_path)
        .expect_err("a token this host does not hold must not pair");
    assert_eq!(err.code, "host_unauthorized", "got {err:?}");
    assert!(!hosts_path.exists(), "a pairing that never proved itself must not be written");
}

#[test]
fn pairing_with_the_wrong_fingerprint_is_refused_and_saves_nothing() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let err = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), "wrong:fingerprint".into(), &hosts_path)
        .expect_err("a certificate that is not the pinned one is a hard refusal");
    assert_eq!(err.code, "host_fingerprint", "got {err:?}");
    assert!(!hosts_path.exists());
}

#[test]
fn pairing_an_unreachable_address_is_refused_and_saves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    // The conventional black-holed address: routable, never answering.
    let err = dev
        .pair("Pi".into(), "10.255.255.1:7878".into(), "token".into(), "aa:bb:cc".into(), &hosts_path)
        .expect_err("nothing answers this address");
    assert_eq!(err.code, "host_unreachable", "got {err:?}");
    assert!(!hosts_path.exists());
}

/// Re-pairing an address that is already paired mints a second entry, and must: a changed
/// fingerprint is refused on every later connection, so pairing again is the only recovery
/// there is. What was missing is anyone saying so — two rows with the same name and address,
/// one of them permanently broken, and nothing in the UI explaining which or why (#107).
#[test]
fn an_address_that_is_already_paired_is_recognised_before_anything_is_written() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "pi.local:7878"));

    assert!(
        dev.existing_pairing("elsewhere.local:7878", "aa:bb:cc").is_none(),
        "an address nobody has paired is a first pairing"
    );

    let same = dev.existing_pairing("pi.local:7878", "aa:bb:cc").expect("this address is paired");
    assert_eq!(same.id, HostId("host-1".into()));
    assert!(same.same_fingerprint, "the certificate has not changed — this is a re-pair, no more");

    // The case worth saying out loud: the Pi answers with a certificate that is not the one
    // pinned. Either it was reinstalled, or something else is at that address.
    let changed = dev.existing_pairing("pi.local:7878", "ff:ee:dd").expect("this address is paired");
    assert!(!changed.same_fingerprint);
    assert_eq!(changed.name, "Workshop Pi", "and it can be named, so the warning can point at a row");
}

#[test]
fn a_pair_that_fails_to_save_adds_nothing_in_memory() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = unwritable_hosts_path(dir.path());
    let dev = test_device_setup();

    let err = dev.pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path);
    assert!(err.is_err(), "the save must fail against this path");
    assert!(dev.paired_hosts().is_empty(), "a failed save must not leave a phantom host in memory");
}

#[test]
fn a_forget_that_fails_to_save_removes_nothing_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = unwritable_hosts_path(dir.path());
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

    // Forced, so this reaches the save at all: unforced it would stop at the idle check,
    // and the failure under test here is the write, not the network.
    let err = dev.forget(&HostId("host-1".into()), &hosts_path, true);
    assert_eq!(err.unwrap_err().code, "hosts_unwritable");
    assert_eq!(dev.paired_hosts().len(), 1, "a failed save must not remove a host still on disk");
    let conn = dev.host_conn(&HostId("host-1".into())).expect("the host remains paired");
    assert_eq!(
        conn.lock().unwrap().lifecycle,
        HostLifecycle::Active,
        "a failed save left the connection refusing calls"
    );
}

/// The persistence seam owns this ordering: calls are refused while the file is written, but
/// the host-control mutex is not held behind the disk; a failed write restores the old pairing.
#[test]
fn a_forget_transition_refuses_calls_without_holding_host_control_behind_the_save() {
    let dev = test_device_setup();
    let id = HostId("host-1".into());
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let conn = dev.host_conn(&id).unwrap();

    let err = dev
        .forget_with_save(&id, true, |prospective| {
            assert!(prospective.is_empty(), "the saved list still carried the forgotten host");
            let lifecycle = conn
                .try_lock()
                .expect("the filesystem save was holding the host-control mutex")
                .lifecycle;
            assert_eq!(
                lifecycle,
                HostLifecycle::Forgetting,
                "calls were not stood down before the save"
            );
            assert!(
                dev.paired_hosts().is_empty(),
                "a host mid-forget was still a row a concurrent save would write back"
            );
            // The question above already failed to reach 127.0.0.1:1, so the error it left is
            // not evidence of this call. Cleared first: dialling would write it straight back.
            conn.lock().unwrap().last_error = None;
            dev.list_devices();
            assert!(
                conn.lock().unwrap().last_error.is_none(),
                "list_devices dialled a host that is being forgotten"
            );
            let call = dev.with_host(&id, |_| Ok(()));
            assert_eq!(
                call.unwrap_err().code,
                "host_busy",
                "a call crossed the save into the host"
            );
            Err(crate::hosts::HostsError::Unwritable("simulated".into()))
        })
        .expect_err("the simulated save succeeded");

    assert_eq!(err.code, "hosts_unwritable");
    assert_eq!(
        conn.lock().unwrap().lifecycle,
        HostLifecycle::Active,
        "a failed save left the pairing stood down"
    );
}

/// `pair` and `forget` each rewrite `hosts.json` from a snapshot. The forget publishes
/// `Forgetting` and then writes; a pair that runs in that window must not put the forgotten
/// row back, and the forget's own write must not drop the pair.
#[test]
fn a_pair_during_a_forget_save_keeps_the_pair_and_drops_the_forgotten_host() {
    let live = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = Arc::new(test_device_setup());
    let forgotten = dev
        .pair(
            "Pi".into(),
            live.addr.clone(),
            HOST_TOKEN.into(),
            live.fingerprint.clone(),
            &hosts_path,
        )
        .expect("the first pairing");

    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let dev_forget = Arc::clone(&dev);
    let id = forgotten.clone();
    let forgotten_id = id.clone();
    let forget = std::thread::spawn(move || {
        dev_forget.forget_with_save(&id, true, move |prospective| {
            assert!(
                prospective.iter().all(|h| h.id != forgotten_id),
                "the save still named the host being forgotten"
            );
            entered_tx.send(()).expect("the test stopped listening");
            release_rx.recv().expect("the pair was not released");
            Ok(())
        })
    });
    entered_rx.recv().expect("the forget never reached its save");

    let dev_pair = Arc::clone(&dev);
    let path_pair = hosts_path.clone();
    let addr = live.addr.clone();
    let fingerprint = live.fingerprint.clone();
    let pair = std::thread::spawn(move || {
        dev_pair.pair("Again".into(), addr, HOST_TOKEN.into(), fingerprint, &path_pair)
    });
    // The pair has to get through `pair_check` and then block on the save this forget holds.
    // Long enough for a loopback handshake, short of the forget still waiting on us.
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        dev.paired_hosts().is_empty(),
        "the pair finished during the forget's save and rewrote the file out from under it"
    );

    release_tx.send(()).unwrap();
    forget.join().unwrap().expect("the forget's save failed");
    let paired = pair.join().unwrap().expect("the pair that waited out the forget failed");
    assert_ne!(paired, forgotten);

    let left = dev.paired_hosts();
    assert_eq!(left.len(), 1, "the forgotten host came back, or the pair was dropped: {left:?}");
    assert_eq!(left[0].id, paired);
    let on_disk = crate::hosts::load(&hosts_path).expect("hosts.json");
    assert_eq!(on_disk.len(), 1);
    assert_eq!(on_disk[0].id, paired);
}

/// The reversal: a host that cannot be asked is not a host known to be idle. Drop Wi-Fi
/// mid-Job and the Pi keeps cutting — discarding the token there throws away the only route
/// left to cancel it. Refused unforced, with its own code, and let through by the force the
/// operator has to ask for explicitly.
#[test]
fn an_unreachable_host_refuses_to_be_forgotten_until_it_is_forced() {
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

    let err = dev.forget(&HostId("host-1".into()), &hosts_path, false).expect_err("silence is not idleness");
    assert_eq!(err.code, "host_unconfirmed", "distinct from the busy refusal: {err:?}");
    assert_eq!(dev.paired_hosts().len(), 1, "refused, so still paired");

    dev.forget(&HostId("host-1".into()), &hosts_path, true).expect("a Pi that is gone for good must stay forgettable");
    assert!(dev.paired_hosts().is_empty());
}

#[test]
fn a_host_with_an_active_cut_refuses_to_be_forgotten() {
    use std::time::{Duration, Instant};

    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("this host answers and the fingerprint matches");

    // `dispatch` returns as soon as the daemon accepts the id; the cut itself runs on a
    // thread there (`Host::dispatch`). Poll until the cutter reports busy rather than
    // asserting on a race.
    let client = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
    client
        .dispatch(
            &cut_host::protocol::DispatchId("d-1".into()),
            cut_host::host::testing::CAMEO,
            "cameo5",
            vec![CutPass {
                job: driver_core::Job {
                    polylines: vec![vec![
                        geometry::Point { x: 0.0, y: 0.0 },
                        geometry::Point { x: 10.0, y: 0.0 },
                        geometry::Point { x: 10.0, y: 10.0 },
                        geometry::Point { x: 0.0, y: 0.0 },
                    ]],
                    settings: driver_core::Settings::default(),
                },
            }],
        )
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let busy = client
            .snapshots()
            .unwrap()
            .iter()
            .any(|s| s.info.instance_id == cut_host::host::testing::CAMEO && s.status.is_active());
        if busy {
            break;
        }
        assert!(Instant::now() < deadline, "the cutter never went active");
        std::thread::sleep(Duration::from_millis(20));
    }

    let err = dev.forget(&id, &hosts_path, false).expect_err("a moving blade must not become unstoppable");
    assert_eq!(err.code, "host_busy");
    assert_eq!(dev.paired_hosts().len(), 1, "refused, so still paired");

    // Force is for a host that cannot answer, not one that answered "busy": this host is
    // reachable, so `cancel` still works and the blade was never orphaned.
    let forced = dev.forget(&id, &hosts_path, true).expect_err("force must not override a confirmed cut");
    assert_eq!(forced.code, "host_busy");
    assert_eq!(dev.paired_hosts().len(), 1, "still paired");
}

/// The two cutters here share an `instance_id` on purpose — the local factory's device and
/// the loopback host's Cameo are both `usb:1:4`, because a fallback id is assigned by
/// location and two identically-wired machines really do collide. A guard comparing ids
/// alone would call these the same cutter and send A's Passes to B.
#[test]
fn a_dispatch_whose_aim_moved_after_planning_is_refused() {
    use std::time::{Duration, Instant};

    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let mut app = AppState::new();
    let dev = test_device_setup();
    app.add_rect(10.0, 10.0);

    let host_id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("this host answers and the fingerprint matches");

    // Preflight runs against the local cutter...
    let (planned_for, passes) = dev.prepare_cut(&app, request_from(plan_for(&app))).unwrap();
    assert_eq!(planned_for.host, None);

    // ...and the operator connects the Pi's Cameo before the dispatch lands.
    *dev.connected.lock().unwrap() = Some(DeviceInfo {
        instance_id: cut_host::host::testing::CAMEO.into(),
        machine_id: "cameo5".into(),
        transport: TransportKind::Usb { locator: "1:4".into() },
        candidate: false,
        host: Some(host_id),
    });

    let err = dev.execute_cut(planned_for, passes).expect_err("Passes approved for A must not go to B");
    assert_eq!(err.code, "device_mismatch");

    // The daemon accepts a dispatch before its cut thread starts, so a single snapshot could
    // read idle on a host that is about to move. Hold the assertion open instead.
    let client = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        let active = client
            .snapshots()
            .unwrap()
            .iter()
            .any(|s| s.info.instance_id == cut_host::host::testing::CAMEO && s.status.is_active());
        assert!(!active, "the refused Passes reached the host anyway");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_idle_host_is_forgotten() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("this host answers and the fingerprint matches");

    dev.forget(&id, &hosts_path, false).expect("it answers, and nothing is running on it");
    assert!(dev.paired_hosts().is_empty());
}

/// Otherwise `get_connected_device` keeps answering with a device `list_devices` no longer
/// lists — the aim would silently outlive the host it named.
#[test]
fn forgetting_the_connected_host_clears_the_aim() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("this host answers and the fingerprint matches");
    dev.connected.lock().unwrap().replace(DeviceInfo { host: Some(id.clone()), ..test_instance() });

    dev.forget(&id, &hosts_path, false).expect("it answers, and nothing is running on it");
    assert!(dev.connected.lock().unwrap().is_none(), "the aim must not survive the host it named");
}

/// A TCP listener that accepts and then says nothing at all — a Pi that answers the SYN and
/// then wedges, which is the shape that used to hold the `hosts` map lock for everyone.
/// Loopback rather than a black-holed address on purpose: how long an unroutable address
/// takes to fail is the network's business, and a test that needs a real stall must not
/// depend on it.
fn start_silent_host() -> (String, std::sync::mpsc::Receiver<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (accepted, saw_accept) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // Held rather than dropped: closing the socket would let the client fail instantly,
        // which is the one thing this fixture must not do.
        let mut open = Vec::new();
        while let Ok((sock, _)) = listener.accept() {
            open.push(sock);
            let _ = accepted.send(());
        }
    });
    (addr, saw_accept)
}

/// The property the per-connection locks buy: a call aimed at one host is not delayed by a
/// call already in flight against another. Before the split this failed — `with_host` held
/// the whole `hosts` map across its network call, so the healthy host's answer queued behind
/// the wedged host's timeout, and so did `cancel` and the window-close guard's `status()`.
///
/// Deterministic without sleeping on a guess: the healthy call is only made once the wedged
/// connection's own lock is *observed* taken, and the assertion is ordering ("the healthy one
/// answered first"), not a duration.
#[test]
fn a_call_to_one_host_is_not_delayed_by_a_wedged_call_to_another() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    let (silent_addr, saw_accept) = start_silent_host();
    let healthy = start_loopback_host();
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-wedged", &silent_addr));
    dev.add_host(PairedHost {
        id: HostId("host-healthy".into()),
        name: "Workshop Pi".into(),
        address: healthy.addr.clone(),
        fingerprint: healthy.fingerprint.clone(),
        token: HOST_TOKEN.into(),
    });

    // Taken before anything is in flight, so that observing it later cannot itself be what
    // blocks — under the old shape this very call would have queued on the map lock, and the
    // test would have failed for a reason one step removed from the one it is about.
    let wedged_conn = dev.host_conn(&HostId("host-wedged".into())).unwrap();

    let wedged_done = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            let _ = dev.with_host(&HostId("host-wedged".into()), |c| c.devices());
            wedged_done.store(true, Ordering::SeqCst);
        });

        // Two anchors, so "in flight" is observed rather than assumed: the fixture has taken
        // the connection, and the connection's lock is held by the call that took it.
        saw_accept.recv_timeout(Duration::from_secs(10)).expect("the wedged host was never dialled");
        let deadline = Instant::now() + Duration::from_secs(10);
        while wedged_conn.try_lock().is_ok() {
            assert!(Instant::now() < deadline, "the wedged call never took its connection lock");
            std::thread::sleep(Duration::from_millis(5));
        }

        dev.with_host(&HostId("host-healthy".into()), |c| c.devices())
            .expect("the healthy host answers regardless of what the wedged one is doing");
        assert!(
            !wedged_done.load(Ordering::SeqCst),
            "the healthy host only answered once the wedged call gave up — the two are still serialized"
        );
    });
}

/// The window-close guard's read must not dial. Its 2s budget only starts once the host's
/// connection lock is in hand, and `list_devices` holds that same lock across a 30s call — so
/// a close arriving during a device-list poll waited for the listing first, on a synchronous
/// callback that runs on the main thread (#115).
///
/// Driven by holding the connection lock outright, which is the state a slow call leaves it
/// in, and asserted on time: the guard has to answer while the lock is held by someone else.
#[test]
fn the_close_guards_read_answers_while_another_call_holds_the_hosts_connection() {
    use std::time::{Duration, Instant};

    let (silent_addr, _saw_accept) = start_silent_host();
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-wedged", &silent_addr));
    let aimed = host_cameo(&HostId("host-wedged".into()));
    *dev.connected.lock().unwrap() = Some(aimed);

    let conn = dev.host_conn(&HostId("host-wedged".into())).unwrap();
    let held = conn.lock().unwrap();

    let started = Instant::now();
    let status = dev.status_without_dialling();
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "the close guard waited on the host's connection lock: {:?}",
        started.elapsed()
    );
    // Nothing has ever been heard from this host, so there is nothing to warn about — and a
    // guard that cannot ask must not invent an active cut either.
    assert!(!status.is_active());
    drop(held);
}

/// And it reports what was actually last heard, so a cut that a poll saw start still holds the
/// window closed after the poll stops.
#[test]
fn the_close_guards_read_remembers_the_last_status_a_poll_managed() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let host_id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&host_id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    assert!(!dev.status_without_dialling().is_active(), "nothing has been heard yet");

    dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");
    // The dialog's own poll, which is what fills the cache.
    wait_for_remote(&dev, |s| s.is_active(), "the cut to show up in a poll");

    assert!(
        dev.status_without_dialling().is_active(),
        "a cut a poll saw start must still hold the window closed"
    );
}

/// Forgetting one host must not disturb an aim pointed at a different one.
#[test]
fn forgetting_an_unrelated_host_leaves_the_aim_alone() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("this host answers and the fingerprint matches");
    dev.add_host(a_paired_host("host-elsewhere", "127.0.0.1:1"));
    dev.connected.lock().unwrap().replace(DeviceInfo { host: Some(id.clone()), ..test_instance() });

    // Forced: this one is unreachable, and an unreachable host is no longer forgettable
    // without it. What the test is about is the *other* host's aim, which must not move.
    dev.forget(&HostId("host-elsewhere".into()), &hosts_path, true).expect("forced");
    assert_eq!(dev.connected.lock().unwrap().as_ref().unwrap().host, Some(id), "a different host's forget must not move the aim");
}

// --- dispatch idempotency ---
//
// Two failures, pulling opposite ways, and both are the material: a retry after a lost reply
// that cuts the sheet twice, and a fix so eager that a second sheet can never be cut at all.
// Every test below holds one of the two ends down.

/// A TCP relay in front of a loopback host that can swallow one reply. Everything the client
/// sends still reaches the host, so the Job runs; once armed, the host's answer is dropped
/// and the connection taken away instead. Waiting for that answer to *exist* before dropping
/// it is the point — it means the host had already committed the Job when the sender lost it.
struct ReplyEater {
    addr: String,
    swallow: Arc<std::sync::atomic::AtomicBool>,
}

fn start_reply_eater(upstream: &str) -> ReplyEater {
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let swallow = Arc::new(AtomicBool::new(false));
    let (upstream, flag) = (upstream.to_string(), swallow.clone());

    std::thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let Ok(server) = std::net::TcpStream::connect(&upstream) else { continue };
            let (mut from_client, mut to_client) = (client.try_clone().unwrap(), client);
            let (mut from_server, mut to_server) = (server.try_clone().unwrap(), server);

            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = from_client.read(&mut buf) {
                    if n == 0 || to_server.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
            let armed = flag.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = from_server.read(&mut buf) {
                    if n == 0 || armed.load(Ordering::SeqCst) {
                        break;
                    }
                    if to_client.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                let _ = to_client.shutdown(std::net::Shutdown::Both);
            });
        }
    });
    ReplyEater { addr, swallow }
}

fn host_cameo(host: &HostId) -> DeviceInfo {
    DeviceInfo {
        instance_id: cut_host::host::testing::CAMEO.into(),
        machine_id: "cameo5".into(),
        transport: TransportKind::Usb { locator: "1:4".into() },
        candidate: false,
        host: Some(host.clone()),
    }
}

fn host_puma(host: &HostId) -> DeviceInfo {
    DeviceInfo {
        instance_id: cut_host::host::testing::PUMA.into(),
        machine_id: "puma".into(),
        transport: TransportKind::Serial { path: "/dev/ttyUSB0".into(), baud: 9600 },
        candidate: true,
        host: Some(host.clone()),
    }
}

fn a_square(side: f64) -> Vec<CutPass> {
    vec![CutPass {
        job: Job {
            polylines: vec![vec![
                geometry::Point { x: 0.0, y: 0.0 },
                geometry::Point { x: side, y: 0.0 },
                geometry::Point { x: side, y: side },
                geometry::Point { x: 0.0, y: 0.0 },
            ]],
            settings: driver_core::Settings::default(),
        },
    }]
}

fn cutter_is_active(client: &cut_host::client::HostClient, instance_id: &str) -> bool {
    client
        .snapshots()
        .map(|snaps| snaps.iter().any(|s| s.info.instance_id == instance_id && s.status.is_active()))
        .unwrap_or(false)
}

fn cameo_is_active(client: &cut_host::client::HostClient) -> bool {
    cutter_is_active(client, cut_host::host::testing::CAMEO)
}

fn wait_until(mut done: impl FnMut() -> bool, complaint: &str) {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(2);
    while !done() {
        assert!(Instant::now() < deadline, "{complaint}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Holds an assertion open rather than sampling once: the host accepts a dispatch before its
/// cut thread starts, so a single reading can miss a cut that is about to happen.
fn stays_false(mut claim: impl FnMut() -> bool, complaint: &str) {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline {
        assert!(!claim(), "{complaint}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The finding's own scenario, end to end: the host takes dispatch `d1` and starts cutting,
/// the reply is lost, the Job finishes, and the operator presses Cut again.
/// The remote half of the exit. This desktop never opened the Pi's transport, so its own
/// `disconnect` there is bookkeeping and would leave the cutter exactly as stuck — only the
/// host's `Reconnect` re-opens it. Driven over the real loopback host so the verb is proved on
/// the wire, not just in the router. Asserted through `actions`: the phase reads `Idle` on
/// both sides of the reconnect.
#[test]
fn reconnecting_a_remote_cutter_clears_a_cancel_that_could_not_confirm_the_stop() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let host_id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&host_id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");
    // The host's TestDriver parks rather than polls, so nothing can confirm a stop there.
    wait_for_remote(&dev, |s| s.actions.confirm, "the Job to park");
    dev.cancel().expect("cancel");
    wait_for_remote(&dev, |s| s.ended == Some(driver_core::Ended::Cancelled), "the cancel to land");
    assert!(!dev.status().actions.cut, "nothing saw the machine stop, so no Job may follow it");
    // And the mark stands through it, which is `snapshot_is_free`'s second half: a cutter that
    // will not take a Job because nothing saw it stop is claimed by nothing and still not free,
    // so no poll may clear the warning over it.
    assert!(dev.a_cut_may_be_running(), "an unconfirmed remote cancel stopped holding the window");

    dev.reconnect().expect("the host re-opens its own cutter");
    assert!(dev.status().actions.cut, "a re-opened cutter takes a Job again");
}

/// One Pi, two cutters. The snapshot keeps both, and a command names the cutter it means
/// rather than whichever one the aim happens to be (#307).
#[test]
fn every_cutter_on_a_host_keeps_its_status_and_takes_its_own_commands() {
    use cut_host::host::testing::{CAMEO, PUMA};

    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");

    let listed = dev.list_devices();
    let remote: Vec<_> = listed.iter().filter(|d| d.host.as_ref() == Some(&id)).collect();
    assert!(remote.iter().any(|d| d.instance_id == CAMEO), "the cameo was dropped: {listed:?}");
    assert!(remote.contains(&&host_puma(&id)), "the puma was dropped: {listed:?}");

    let statuses = dev.cutter_statuses();
    assert!(
        statuses.iter().any(|s| s.host == id && s.instance_id == CAMEO && s.status.actions.cut),
        "cameo status: {statuses:?}"
    );
    assert!(
        statuses.iter().any(|s| s.host == id && s.instance_id == PUMA && s.status.actions.cut),
        "puma status: {statuses:?}"
    );
    assert!(
        dev.host_views().iter().any(|v| v.id == id && v.last_seen.is_some()),
        "a snapshot that landed left no last_seen"
    );

    *dev.connected.lock().unwrap() = Some(host_cameo(&id));
    let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
    direct
        .dispatch(
            &cut_host::protocol::DispatchId("d-puma".into()),
            PUMA,
            "puma",
            a_square(10.0),
        )
        .unwrap();
    wait_until(|| cutter_is_active(&direct, PUMA), "the puma never went active");

    dev.list_devices();
    let cached = dev.cutter_statuses();
    let puma = cached.iter().find(|s| s.instance_id == PUMA).expect("puma left the cache");
    let cameo = cached.iter().find(|s| s.instance_id == CAMEO).expect("cameo left the cache");
    assert!(puma.status.is_active(), "the sibling's status was dropped: {puma:?}");
    assert!(cameo.status.actions.cut && !cameo.status.is_active(), "the idle cutter was reported busy");

    let aimed = dev.status();
    assert!(
        aimed.actions.cut && !aimed.is_active(),
        "status() stopped answering for the aim: {aimed:?}"
    );
    assert!(
        dev.cutter_statuses().iter().any(|s| s.instance_id == PUMA && s.status.is_active()),
        "reading the aim replaced the sibling's status"
    );

    dev.cancel().expect("cancel follows the aim");
    assert!(cutter_is_active(&direct, PUMA), "cancelling the aim stopped the other cutter");

    dev.cancel_addressed(Some(id), Some(PUMA.into())).expect("the puma is addressed by name");
    wait_until(|| !cutter_is_active(&direct, PUMA), "the addressed cancel never reached the puma");
}

fn cache_snapshot(instance_id: &str) -> cut_host::protocol::DeviceSnapshot {
    cut_host::protocol::DeviceSnapshot {
        info: DeviceInfo {
            instance_id: instance_id.into(),
            machine_id: "cameo5".into(),
            transport: TransportKind::Usb { locator: "1:4".into() },
            candidate: false,
            host: None,
        },
        status: CutStatus::disconnected(),
        job_id: None,
        claimed: false,
    }
}

/// The cache is what the Devices tab and the cutting dot read. A snapshot that names fewer
/// cutters has unplugged one; a failed dial never calls `remember_snapshots`, so this is only
/// the answer a host actually gave.
#[test]
fn a_snapshot_that_omits_a_cutter_drops_it_from_the_status_cache() {
    let dev = test_device_setup();
    let id = HostId("host-9".into());
    let other = HostId("host-8".into());
    let cameo = cache_snapshot("usb:cameo");
    dev.remember_snapshots(&id, &[cameo.clone(), cache_snapshot("usb:puma")]);
    dev.remember_snapshots(&other, &[cache_snapshot("usb:other")]);

    dev.remember_snapshots(&id, &[cameo]);
    let left: Vec<_> = dev
        .cutter_statuses()
        .into_iter()
        .map(|s| (s.host.0, s.instance_id))
        .collect();
    assert!(left.iter().any(|(h, i)| h == "host-9" && i == "usb:cameo"), "{left:?}");
    assert!(
        !left.iter().any(|(_, i)| i == "usb:puma"),
        "an unplugged cutter stayed cached: {left:?}"
    );
    assert!(
        left.iter().any(|(h, i)| h == "host-8" && i == "usb:other"),
        "another host was pruned: {left:?}"
    );

    dev.remember_snapshots(&id, &[]);
    let after = dev.cutter_statuses();
    assert!(
        !after.iter().any(|s| s.host == id),
        "an empty answer left this host's cutters: {after:?}"
    );
    assert!(after.iter().any(|s| s.host == other), "an empty answer pruned the other host");
}

fn wait_for_remote(dev: &DeviceManagerHandle, want: impl Fn(&CutStatus) -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let s = dev.status();
        if want(&s) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "waited out {what}, sat at {:?}", s.phase);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn a_retry_after_a_lost_reply_does_not_cut_the_material_twice() {
    use std::sync::atomic::Ordering;

    let host = start_loopback_host();
    let eater = start_reply_eater(&host.addr);
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let host_id = dev
        .pair("Pi".into(), eater.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("the relay forwards the pairing check to a host that answers");
    // Establishes the desktop's own connection through the relay first, so arming it takes
    // away a dispatch reply rather than a piece of the TLS handshake.
    dev.list_devices();

    let aimed = host_cameo(&host_id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    eater.swallow.store(true, Ordering::SeqCst);
    let err = dev
        .execute_cut(aimed.clone(), a_square(10.0))
        .expect_err("the host's answer never came back");
    assert_eq!(err.code, "dispatch_unconfirmed", "{}", err.message);

    // Straight to the host, around the relay: the Job really did land, and really did run.
    let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
    wait_until(|| cameo_is_active(&direct), "the dispatch never reached the host");
    direct.confirm_pass_done(cut_host::host::testing::CAMEO).unwrap();
    wait_until(|| !cameo_is_active(&direct), "the job never finished");

    // Connectivity is back and the operator presses Cut on the same design.
    eater.swallow.store(false, Ordering::SeqCst);
    let retry = dev.execute_cut(aimed, a_square(10.0)).expect("the retry reaches the host");

    stays_false(|| cameo_is_active(&direct), "the same material was cut a second time");
    // And the operator is told which of the two things happened. `Ok` alone said "your Job has
    // started" about a dispatch that started nothing, in front of a cutter that never moved.
    assert!(retry.duplicate, "a Job the host had already accepted must say so");
}

/// The other end: the fix must not make a second sheet impossible. Same design, same cutter,
/// dispatched again after the first one was answered and finished — and the blade moves.
#[test]
fn cutting_the_same_design_again_after_an_answered_dispatch_starts_a_new_job() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let host_id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("this host answers and the fingerprint matches");
    let aimed = host_cameo(&host_id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();

    dev.execute_cut(aimed.clone(), a_square(10.0)).expect("first sheet");
    wait_until(|| cameo_is_active(&direct), "the first cut never started");
    direct.confirm_pass_done(cut_host::host::testing::CAMEO).unwrap();
    wait_until(|| !cameo_is_active(&direct), "the first cut never finished");

    // Fresh material, same file: a legitimate second Job, not a retry of anything.
    let second = dev.execute_cut(aimed, a_square(10.0)).expect("second sheet");
    assert!(!second.duplicate, "a deliberate re-cut is not a duplicate");
    wait_until(|| cameo_is_active(&direct), "the second sheet was refused as a duplicate");
}

/// The headline verb of the whole feature, driven from the desktop side: the Passes reach the
/// host intact and the Job it starts is the one that was planned. Every other routed verb had
/// a test against a loopback host; this one had none (#108).
///
/// The count is asserted through the host's own `pass` position rather than by inspecting what
/// the mock Driver encoded: a dispatch that arrived with a Pass missing would park at
/// `1 of 1` here, which is exactly the silent loss worth catching.
#[test]
fn a_remote_dispatch_carries_every_pass_and_starts_the_job_it_planned() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let host_id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("this host answers and the fingerprint matches");
    let aimed = host_cameo(&host_id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    let two_passes = [a_square(10.0), a_square(20.0)].concat();
    let started = dev.execute_cut(aimed, two_passes).expect("dispatch");
    assert!(!started.duplicate, "nothing was in doubt, so this is a new Job");

    let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
    wait_until(|| cameo_is_active(&direct), "the dispatch never reached the host");

    let snap = direct
        .snapshots()
        .unwrap()
        .into_iter()
        .find(|s| s.info.instance_id == cut_host::host::testing::CAMEO)
        .expect("the host knows this cutter");
    assert_eq!(
        snap.status.pass.map(|p| p.total),
        Some(2),
        "the host is running a different number of Passes than were sent"
    );
    assert!(snap.job_id.is_some(), "and it registered a Job for them");
}

/// Two presses of Cut on one Job cannot mint two ids. Choosing an id and recording it used to
/// be separate steps, and Start Cut stays enabled until a poll moves the status — so two
/// presses could both find nothing and mint one each. If the first reached the host and lost
/// its reply, the second's entry replaced the only record of it, and the retry that should
/// have been recognised went out under a name the host had never seen.
#[test]
fn two_presses_of_cut_cannot_mint_two_ids_for_one_job() {
    let dev = test_device_setup();
    let key = key_for(&a_square(10.0));

    let (first, reserved_first) = dev.ledger.reserve_dispatch_id(&key);
    let (second, reserved_second) = dev.ledger.reserve_dispatch_id(&key);
    assert_eq!(first, second, "the second press must join the first, not race it");
    assert!(reserved_first, "the first press is what recorded it");
    assert!(!reserved_second, "and the second must know it did not");
}

fn key_for(passes: &[CutPass]) -> JobKey {
    JobKey {
        host: HostId("host-1".into()),
        device: cut_host::host::testing::CAMEO.into(),
        digest: job_digest("cameo5", passes),
    }
}

#[test]
fn two_different_jobs_to_one_cutter_get_different_ids() {
    let dev = test_device_setup();
    assert_ne!(
        dev.ledger.reserve_dispatch_id(&key_for(&a_square(10.0))).0,
        dev.ledger.reserve_dispatch_id(&key_for(&a_square(20.0))).0,
        "different geometry is a different Job"
    );

    // Settings, not only geometry: the same shape cut at a different force is a different
    // Job, and a digest that missed it would let one be swallowed as a retry of the other.
    let mut harder = a_square(10.0);
    harder[0].job.settings.force = Some(driver_core::Settings::default().force.unwrap_or(0) + 1);
    assert_ne!(job_digest("cameo5", &a_square(10.0)), job_digest("cameo5", &harder));

    // And the same Passes aimed at a different machine are not interchangeable either.
    assert_ne!(job_digest("cameo5", &a_square(10.0)), job_digest("puma", &a_square(10.0)));
}

/// The whole distinction, in the one place it lives. Flip either half and this fails. Asserted
/// against the entry rather than against "an answer", because since #283 an answer that does not
/// settle what the host did leaves the entry standing — what reuse turns on is the entry.
#[test]
fn an_id_is_reused_only_while_its_job_still_has_an_entry() {
    let dev = test_device_setup();
    let key = key_for(&a_square(10.0));

    // One dispatch outstanding: the next press of Cut is that dispatch, tried again.
    let (first, _) = dev.ledger.reserve_dispatch_id(&key);
    assert_eq!(
        dev.ledger.reserve_dispatch_id(&key).0,
        first,
        "a retry must arrive under the id it went out with"
    );

    // ...and only for that Job. A different one is never mistaken for the retry.
    assert_ne!(dev.ledger.reserve_dispatch_id(&key_for(&a_square(20.0))).0, first);

    // A settling answer clears the entry, and a deliberate re-cut of the same design is then its
    // own Job rather than a duplicate of the one that finished.
    dev.ledger.forget(&key);
    assert_ne!(dev.ledger.reserve_dispatch_id(&key).0, first, "a re-cut must not be swallowed");
}

/// A cutter enumerated by path can be most of a `/dev/serial/by-id/...` string, and the host
/// refuses a dispatch id longer than it will remember — which would have refused *every*
/// dispatch to that cutter, permanently, for a reason nothing in the message points at.
#[test]
fn a_dispatch_id_fits_what_a_host_will_accept_even_for_a_long_cutter_id() {
    let dev = test_device_setup();
    let key = JobKey {
        host: HostId("host-1".into()),
        device: "serial:at:/dev/serial/by-id/usb-FTDI_FT232R_USB_UART_A50285BI-if00-port0".into(),
        digest: job_digest("puma", &a_square(10.0)),
    };
    let (id, _) = dev.ledger.reserve_dispatch_id(&key);
    assert!(
        id.0.len() <= 128,
        "the host refuses ids over 128 characters; this one is {}: {}",
        id.0.len(),
        id.0
    );
    // Still distinguishing two Jobs on that cutter — the truncation drops the part the digest
    // already covers, not the part that tells Jobs apart.
    let other = JobKey { digest: job_digest("puma", &a_square(20.0)), ..key.clone() };
    assert_ne!(dev.ledger.reserve_dispatch_id(&other).0, id);
}

/// Entries clear on a settling answer and expire on `ID_RETENTION`, so this is a backstop
/// rather than a policy — but without it a session dispatching thousands of distinct designs at
/// hosts that never answer grows a map nothing ever prunes.
#[test]
fn jobs_held_in_doubt_are_bounded() {
    let dev = test_device_setup();
    for n in 0..MAX_JOBS_IN_DOUBT + 20 {
        dev.ledger.reserve_dispatch_id(&JobKey {
            host: HostId("host-1".into()),
            device: cut_host::host::testing::CAMEO.into(),
            digest: n as u64,
        });
    }
    assert!(
        dev.ledger.jobs_in_doubt() <= MAX_JOBS_IN_DOUBT,
        "in doubt: {}",
        dev.ledger.jobs_in_doubt()
    );
}

/// A press that never reached a host keeps its reservation. Undoing it would need this call to
/// know it owns the entry, and it cannot — a concurrent press of Cut on the same Job is using
/// that same id. Keeping it costs nothing: a dispatch no host received is one no host can
/// deduplicate, so the next Cut under that id is cut normally.
#[test]
fn a_press_that_never_reached_a_host_keeps_its_id_for_the_next_one() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let aimed = host_cameo(&HostId("host-1".into()));
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    let err = dev.execute_cut(aimed, a_square(10.0)).expect_err("nothing answers this host");
    // A first press against an offline Pi must not claim the Job may be cutting there.
    assert_eq!(err.code, "host_unreachable", "got {err:?}");

    let key = JobKey {
        host: HostId("host-1".into()),
        device: cut_host::host::testing::CAMEO.into(),
        digest: job_digest("cameo5", &a_square(10.0)),
    };
    assert!(!dev.ledger.reserve_dispatch_id(&key).1, "the next press joins the id already reserved");
}

/// An abandoned entry is deliberately not aged out on a timer of this desktop's own choosing.
/// Time cannot show that the operator loaded fresh material, so expiring one early mints a new
/// id for a Job the host still remembers under the old one — and cuts the material a second
/// time. What makes an entry nobody revisits harmless is that the next press is told it was read
/// as a retry, and that a settling answer clears it, so pressing Cut again really does cut
/// (#121). The two prunings that do exist are the host's own `ID_RETENTION` and the cap, both on
/// `in_doubt`; neither is reached here, and neither is a timer of this desktop's invention.
///
/// Named for what is asserted rather than for the scenario: nothing is dispatched, no time
/// passes, and nothing settles. Two reservations, one entry, one id.
#[test]
fn an_entry_nobody_clears_keeps_naming_the_same_id() {
    let dev = test_device_setup();
    let key = key_for(&a_square(10.0));
    let (abandoned, _) = dev.ledger.reserve_dispatch_id(&key);

    // The next press joins it, and the entry is still the only one.
    assert_eq!(dev.ledger.reserve_dispatch_id(&key).0, abandoned);
    assert_eq!(dev.ledger.jobs_in_doubt(), 1);
}

/// The boundary the offered retry turns on, asked from both sides of it. An entry this call is
/// reusing can age past `ID_RETENTION` while the call is still in flight — the reservation is
/// taken before the request and the sentence is written after the answer, and a dispatch has a
/// 30-second body budget to spend in between — and the next press would then prune it and mint
/// a fresh id. Offering to reuse an id that is about to be discarded is the promise #286
/// removed, arrived at from the other direction, so the sentence asks the same question
/// `reserve_dispatch_id` answers rather than only whether an entry exists.
#[test]
fn a_retry_is_offered_only_while_the_id_is_one_the_next_press_would_reuse() {
    use std::time::Duration;
    let dev = test_device_setup();
    let key = key_for(&a_square(10.0));
    dev.ledger.reserve_dispatch_id(&key);
    let written = dev.ledger.written_at(&key).expect("just reserved");

    assert!(dev.ledger.retry_would_reuse(&key, written), "the id it was just given is not offered");
    assert!(
        dev.ledger.retry_would_reuse(&key, written + cut_host::host::ID_RETENTION - Duration::from_secs(1)),
        "an id the host still remembers is not offered"
    );
    assert!(
        !dev.ledger.retry_would_reuse(&key, written + cut_host::host::ID_RETENTION),
        "an id the next press would prune was offered as a retry"
    );
    assert!(
        !dev.ledger.retry_would_reuse(&key_for(&a_square(20.0)), written),
        "a Job with no entry at all was offered a retry"
    );
}

/// A dispatch that never reached a host must not drop an *earlier* dispatch's reservation: that
/// entry is what a later retry still needs, and clearing it here would send that retry out
/// under a name the host has never seen. That a call keeps its *own* reservation as well is
/// asserted separately, by `a_press_that_never_reached_a_host_keeps_its_id_for_the_next_one`.
#[test]
fn a_dispatch_that_never_reached_a_host_leaves_an_earlier_one_in_doubt() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let aimed = host_cameo(&HostId("host-1".into()));
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    // An earlier dispatch of this Job is outstanding.
    let key = JobKey {
        host: HostId("host-1".into()),
        device: aimed.instance_id.clone(),
        digest: job_digest(&aimed.machine_id, &a_square(10.0)),
    };
    let (earlier, _) = dev.ledger.reserve_dispatch_id(&key);

    // Nothing is listening on this address, so the dispatch never gets a connection.
    let err = dev.execute_cut(aimed, a_square(10.0)).expect_err("nothing answers this host");
    // Still `dispatch_unconfirmed` rather than a plain refusal, and rightly: the earlier
    // dispatch of this Job was never answered, so it may be cutting there right now.
    assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
    assert_eq!(
        dev.ledger.held_id(&key),
        Some(earlier),
        "a call that never reached the host discarded a previous dispatch's id"
    );
}

/// A dispatch answered with a reply that is not `Accepted` learned nothing about whether the
/// Job began, so it stays in doubt and keeps its id. Asserted here on the desktop's own side of
/// that: the entry survives, and the next press reserves the same id rather than a fresh one.
/// What the *host* then does with a repeated id is the host's, and this fixture does not model
/// it — a peer answering outside the protocol may honour no dedupe at all, which is why the
/// refusal is `unconfirmed` rather than an all-clear.
///
/// The reply is one no real `Host` sends to a `Dispatch`, so it takes a peer built to break the
/// protocol; `handle_request` answers every request with a reply that request admits (#283).
#[test]
fn a_dispatch_answered_with_the_wrong_reply_stays_in_doubt() {
    let host = cut_host::client::testing::start_host_answering(vec![
        cut_host::protocol::Response::Ok,
        cut_host::protocol::Response::Ok,
    ]);
    let dev = test_device_setup();
    dev.add_host(PairedHost {
        fingerprint: host.fingerprint.clone(),
        ..a_paired_host("host-1", &host.addr)
    });
    let aimed = host_cameo(&HostId("host-1".into()));
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    let key = JobKey {
        host: HostId("host-1".into()),
        device: aimed.instance_id.clone(),
        digest: job_digest(&aimed.machine_id, &a_square(10.0)),
    };
    let err = dev
        .execute_cut(aimed, a_square(10.0))
        .expect_err("`Ok` is not a dispatch this desktop can read as accepted");
    assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
    assert!(
        err.message.contains("answered with `Ok` where `Accepted` was expected"),
        "the reply is not named: {}",
        err.message
    );

    let held = dev.ledger.held_id(&key);
    assert!(held.is_some(), "a dispatch nothing confirmed must keep the id its retry needs");
    // And the next press reserves that id rather than minting a second name the host has never
    // seen — `reserve_dispatch_id` is the call `execute_cut` makes to choose one.
    assert_eq!(dev.ledger.reserve_dispatch_id(&key).0, held.unwrap());

    // The same failure is what the host's own row then shows, because `with_host_within`
    // records it and drops the client alongside it. Pinned here because the row used to say
    // the host could not be reached, directly under the cutters it had just listed.
    let view = dev.host_views().pop().expect("the host is still paired");
    assert_eq!(
        view.unreachable.as_deref(),
        Some("this host answered with `Ok` where `Accepted` was expected")
    );
}

/// The same, for the one reply that would otherwise have been believed. `Accepted` is the
/// variant this dispatch asked for, so nothing about its shape refuses it — only the
/// `dispatch_id` inside it does. Without that check `Admitted::Started` told the operator
/// their Job had begun on a host that was answering about a different one, and cleared the
/// entry a real retry needs in the same breath (#285).
///
/// The fixture cannot know the id the desktop minted — a digest plus a nonce, chosen inside
/// `execute_cut` — which is what makes any id scripted here a mismatch.
#[test]
fn a_dispatch_answered_about_another_job_stays_in_doubt() {
    let host = cut_host::client::testing::start_host_answering(vec![
        cut_host::protocol::Response::Ok,
        cut_host::protocol::Response::Accepted {
            dispatch_id: cut_host::protocol::DispatchId("some-other-job".into()),
            admitted: cut_host::protocol::Admitted::Started,
        },
    ]);
    let dev = test_device_setup();
    dev.add_host(PairedHost {
        fingerprint: host.fingerprint.clone(),
        ..a_paired_host("host-1", &host.addr)
    });
    let aimed = host_cameo(&HostId("host-1".into()));
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    let key = JobKey {
        host: HostId("host-1".into()),
        device: aimed.instance_id.clone(),
        digest: job_digest(&aimed.machine_id, &a_square(10.0)),
    };
    let err = dev
        .execute_cut(aimed, a_square(10.0))
        .expect_err("an `Accepted` about another dispatch is not this Job's start");
    assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
    assert!(
        err.message.contains("answered about a different dispatch than the one it was sent"),
        "the mismatch is not named: {}",
        err.message
    );
    assert!(
        !err.message.contains("some-other-job"),
        "the peer's own id reached the operator: {}",
        err.message
    );

    let held = dev.ledger.held_id(&key);
    assert!(held.is_some(), "an answer about another Job settles nothing about this one");
    assert_eq!(dev.ledger.reserve_dispatch_id(&key).0, held.unwrap());

    let view = dev.host_views().pop().expect("the host is still paired");
    assert_eq!(
        view.unreachable.as_deref(),
        Some("this host answered about a different dispatch than the one it was sent")
    );
}

/// A refusal reached the host — that is what a refusal is — and the guard keeping a plain error
/// plain asked only whether the host had been reached. So every remote refusal was reported as a
/// dispatch that might be cutting: an off-the-bed cut on a *first* press lost its own code and
/// told the operator their Job may already be running and to press Cut again (#288). A
/// mis-scaled document is the ordinary way to arrive here.
#[test]
fn a_refused_remote_cut_is_reported_as_the_refusal_it_is() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    // 400 mm square on the test Cameo's 300x200 bed.
    let err = dev.execute_cut(aimed, a_square(400.0)).expect_err("a cut off the bed is refused");
    assert_eq!(err.code, "host_refused", "got {err:?}");
    assert!(
        err.message.contains("outside the 300 x 200 mm cutting area"),
        "the refusal lost its own sentence: {}",
        err.message
    );
    assert!(
        !err.message.contains("may already be cutting"),
        "a refusal invented a cut in flight: {}",
        err.message
    );
    // And the same refusal holds nothing: a host that refused started nothing, so the
    // window-close guard has no dispatch of this to warn about either.
    assert!(!dev.ledger.any_mark(), "a refusal held the window open");
}

/// What an unconfirmed dispatch is allowed to promise. The sentence used to end "the host
/// recognizes the same Job and will not cut it twice", which three bounds can make false — the
/// host forgets an accepted id past its retention and past its capacity cap, and `in_doubt` is
/// process-local, so a desktop restarted between the two presses has nothing to reuse. That is
/// the one message in the app telling an operator it is safe to press a button that starts a
/// blade, and it promised an outcome this side cannot know (#286).
///
/// What is left is what is true: the retry goes out under the same id, a host that still
/// remembers that id reads it as this Job, and the cutter is the only place to learn which
/// happened.
#[test]
fn an_unconfirmed_dispatch_promises_no_more_than_the_same_id() {
    let host = cut_host::client::testing::start_host_answering(vec![
        cut_host::protocol::Response::Ok,
        cut_host::protocol::Response::Ok,
    ]);
    let dev = test_device_setup();
    dev.add_host(PairedHost {
        fingerprint: host.fingerprint.clone(),
        ..a_paired_host("host-1", &host.addr)
    });
    let aimed = host_cameo(&HostId("host-1".into()));
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    let err = dev
        .execute_cut(aimed, a_square(10.0))
        .expect_err("`Ok` is not a dispatch this desktop can read as accepted");
    assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
    assert!(
        !err.message.contains("will not cut it twice"),
        "the promise survived: {}",
        err.message
    );
    assert!(
        err.message.contains("only the cutter can tell you"),
        "the one way to find out is not offered: {}",
        err.message
    );
    assert!(
        err.message.contains("under the same id"),
        "the retry's one protection is not stated: {}",
        err.message
    );
}

/// A refusal on a press made while an earlier dispatch is unsettled settles *this* attempt and
/// says nothing about that one, so it stays unconfirmed — and clears the entry, which leaves no
/// id for the next press to send. The advice inverts with it: pressing Cut again is a new Job,
/// not the retry the other branch describes, and telling the operator otherwise is the promise
/// this stopped making (#286, #288).
#[test]
fn an_unconfirmed_dispatch_with_no_id_left_offers_no_retry() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    // An earlier dispatch of this Job is outstanding when the press below is made.
    let key = JobKey {
        host: id.clone(),
        device: aimed.instance_id.clone(),
        digest: job_digest(&aimed.machine_id, &a_square(400.0)),
    };
    dev.ledger.reserve_dispatch_id(&key);

    let err = dev.execute_cut(aimed, a_square(400.0)).expect_err("a cut off the bed is refused");
    assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
    assert!(
        err.message.contains("new Job rather than a retry"),
        "a press with no id left was offered a retry: {}",
        err.message
    );
    assert!(
        !err.message.contains("same id"),
        "an id the refusal cleared was offered anyway: {}",
        err.message
    );
    assert!(!dev.ledger.holds(&key), "the refusal settled this attempt");
}

/// The window-close guard has to see a dispatch that has not been polled yet. The newest
/// status anyone holds at that moment is the `Idle` from before the cut started, so a guard
/// reading only the cache waves the operator past the cut they just pressed Cut for.
#[test]
fn a_dispatch_nothing_has_polled_yet_still_holds_the_window() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();

    let host_id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&host_id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    // A poll before the cut, which is what fills the cache with `Idle`.
    assert!(!dev.status().is_active());
    assert!(!dev.a_cut_may_be_running(), "nothing has been dispatched yet");

    dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");
    // Deliberately no poll in between: this is the window the cache cannot answer for.
    assert!(
        dev.a_cut_may_be_running(),
        "the guard waved through a cut this desktop had just started"
    );

    // And it stops holding the window once the cutter says it would take another Job.
    let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
    wait_until(|| cameo_is_active(&direct), "the cut never started");
    direct.confirm_pass_done(cut_host::host::testing::CAMEO).unwrap();
    wait_until(|| !cameo_is_active(&direct), "the cut never finished");
    wait_until(
        || {
            dev.status();
            !dev.a_cut_may_be_running()
        },
        "a finished cut still held the window closed",
    );
}

/// A refusal settles the press it answered and no other, so the Job's id survives it while a
/// sibling press is still dispatching under that id — the record that press's retry sends
/// under, and the only thing that can stop a lost reply from cutting the material twice
/// (#290). The operator's advice inverts with it: a retry is still the thing to press.
///
/// The sibling is staged here, because this pins the rule and not the interleaving: `begin_dispatch`
/// is exactly what a press mid-dispatch holds. The interleaving itself, a sibling parked inside
/// `on_host` with a second press inside `on_host` behind it, is
/// `a_refusal_leaves_the_id_of_a_sibling_still_inside_on_host`.
#[test]
fn a_refusal_leaves_a_concurrent_press_the_id_its_retry_needs() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    let key = JobKey {
        host: id.clone(),
        device: aimed.instance_id.clone(),
        digest: job_digest(&aimed.machine_id, &a_square(400.0)),
    };
    // A press of the same Job, mid-dispatch and holding the id.
    let (sibling, held, first) = dev.ledger.begin_dispatch(&key);
    assert!(first, "the sibling is what reserved the id this test is about");

    let err = dev.execute_cut(aimed, a_square(400.0)).expect_err("a cut off the bed is refused");
    assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
    assert_eq!(
        dev.ledger.held_id(&key),
        Some(held),
        "a refusal cleared the id a concurrent press was dispatching under"
    );
    assert!(
        err.message.contains("under the same id"),
        "the retry that is still available was not offered: {}",
        err.message
    );

    // Once that press is over and nothing settled it, the entry is still the retry's — and
    // this is what makes the assertion above about the sibling rather than about nothing.
    drop(sibling);
    assert!(dev.ledger.holds(&key), "an unsettled press keeps its id");
}

/// The same rule with the sibling where #290's failure actually happened: inside `on_host`,
/// holding the host's connection, while a second press has begun and is inside `on_host` behind it.
/// The staged test above covers settlement with a live sibling registration — its guard is held
/// across `execute_cut`, so both presses are in flight when the refusal settles. This one
/// additionally establishes overlapping `on_host` calls before the connection is released.
///
/// Synchronised on observed state rather than timing assumptions: the sibling parks inside
/// `on_host` until released, and the test waits until the second press is both in flight in the
/// ledger and counted inside its own `on_host`, between asking for the connection and holding it,
/// while the sibling still holds it.
#[test]
fn a_refusal_leaves_the_id_of_a_sibling_still_inside_on_host() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());
    let key = JobKey {
        host: id.clone(),
        device: aimed.instance_id.clone(),
        digest: job_digest(&aimed.machine_id, &a_square(400.0)),
    };

    let (inside_tx, inside) = std::sync::mpsc::channel::<()>();
    let (dev_ref, key_ref, id_ref) = (&dev, &key, &id);
    std::thread::scope(|s| {
        // Made inside the scope, so a failing `wait_until` drops it while unwinding: `scope`
        // joins the sibling before it re-raises the panic, and a sender that outlived the scope
        // would leave that join waiting on a release that never comes.
        let (release, released) = std::sync::mpsc::channel::<()>();
        let sibling = s.spawn(move || {
            let (sibling, held, first) = dev_ref.ledger.begin_dispatch(key_ref);
            assert!(first, "the sibling is what reserved the id this test is about");
            dev_ref.on_host(id_ref, cut_host::client::CONNECT_TIMEOUT, |_| {
                inside_tx.send(()).unwrap();
                // `Err` is the test body unwinding with the sender dropped: let go, don't hang.
                let _ = released.recv();
                Ok(())
            })
            .expect("the sibling reaches its host");
            // Still in flight: it left `on_host` without an answer, which is the reply that was lost.
            (sibling, held)
        });
        inside.recv().expect("the sibling never reached on_host");

        let second = s.spawn(|| dev.execute_cut(aimed.clone(), a_square(400.0)));
        // Both facts, not either: a press can be registered and still be descheduled before it
        // asks for the connection, in which case releasing the sibling would stage exactly what
        // the test above already stages. `on_host_waiting` is counted across the connection lock
        // acquisition, so it is the second press *inside its own `on_host`*, between asking for
        // the connection and holding it, while the sibling still holds it. That is overlapping
        // `on_host` calls, not proof the second has attempted the lock: it may be descheduled on
        // the way to it. What the assertions need is the overlap — the refusal settles while the
        // sibling's press is in flight.
        wait_until(
            || {
                dev.ledger.presses_in_flight(&key) == 2
                    && dev.on_host_waiting.load(std::sync::atomic::Ordering::SeqCst) == 1
            },
            "the second press never reached the connection inside on_host while the sibling held it",
        );
        release.send(()).unwrap();

        let err = second.join().unwrap().expect_err("a cut off the bed is refused");
        let (sibling, held) = sibling.join().unwrap();
        assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
        assert_eq!(
            dev.ledger.held_id(&key),
            Some(held),
            "a refusal cleared the id a sibling inside on_host was dispatching under"
        );
        assert!(
            err.message.contains("under the same id"),
            "the retry that is still available was not offered: {}",
            err.message
        );
        drop(sibling);
    });
    assert!(dev.ledger.holds(&key), "an unsettled press keeps its id");
}

/// The same rule for the mark that holds the window, and one step wider: the mark is the
/// cutter's, so a refused press must not clear one left by an accepted press of *any* Job on
/// that cutter. Clearing it waved the operator past a running remote cut, which is #158
/// arriving by a second route (#290).
#[test]
fn a_refusal_leaves_a_concurrent_press_holding_the_window() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    // An accepted press of some Job on this cutter, not yet seen to end.
    dev.ledger.mark_dispatched(&id, &aimed.instance_id, AttemptId::next());

    let err = dev.execute_cut(aimed, a_square(400.0)).expect_err("a cut off the bed is refused");
    // The press's own outcome is settled and nothing of *its* Job is outstanding, so it is
    // the plain refusal (#288) — the mark it must not clear belongs to a different Job.
    assert_eq!(err.code, "host_refused", "got {err:?}");
    assert!(
        dev.a_cut_may_be_running(),
        "a refused press cleared the mark an accepted press was still held by"
    );
}

/// A refused press with nothing else outstanding does stop holding the window, which is what
/// makes the test above about attribution rather than about a mark that never clears.
#[test]
fn a_refusal_with_nothing_else_outstanding_releases_the_window() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    dev.execute_cut(aimed, a_square(400.0)).expect_err("a cut off the bed is refused");
    assert!(!dev.a_cut_may_be_running(), "a refusal is the host saying it started nothing");
}

/// The guard answers for the cut, not for the aim: dispatch, then use either verb the UI
/// offers for looking elsewhere, and the Job is still running on the Pi. Both used to close
/// the window in silence — the second one especially, since with no aim at all the guard
/// returned `false` however many dispatches were outstanding (#158).
#[test]
fn a_dispatch_holds_the_window_after_the_aim_moves_away() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    dev.connect(aimed.clone()).expect("aiming at a host is bookkeeping only");

    dev.execute_cut(aimed.clone(), a_square(10.0)).expect("dispatch");

    dev.connect(test_instance()).expect("aiming back at the local cutter");
    assert!(dev.a_cut_may_be_running(), "the remote cut stopped counting when the aim moved");

    dev.connect(aimed).expect("aiming at the host again");
    dev.disconnect().expect("disconnecting a remote aim is bookkeeping only");
    assert!(dev.connected.lock().unwrap().is_none(), "the aim must really be gone");
    assert!(dev.a_cut_may_be_running(), "with no aim the guard stopped answering at all");
}

/// Quitting stops what only this process can stop. A Job parked on the local cutter is
/// cancelled here rather than left, because nothing would be able to stop or resume it once
/// this process — which owns that transport — is gone.
#[test]
fn quitting_cancels_local_motion() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    app.add_rect(10.0, 10.0);
    let plan = plan_for(&app);
    dev.cut_from_request(&app, request_from(plan)).expect("cut");
    assert!(dev.status().is_active(), "the local cut must be parked mid-flight to be cancellable");

    dev.stop_local_motion();
    let stopped = wait_for_cancelled(&dev);
    assert!(!stopped.actions.cut, "quitting left the local cutter taking Jobs");
}

/// The other half, and the load-bearing one: a Cut Host owns the Job it accepted, so quitting
/// leaves it cutting. That is the rule `disconnect` already followed, and the reason
/// `force_quit` no longer routes a cancel by whatever happens to be aimed (#158).
#[test]
fn quitting_leaves_a_cut_host_job_running() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());
    dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");

    let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
    wait_until(|| cameo_is_active(&direct), "the cut never started");

    dev.stop_local_motion();
    assert!(cameo_is_active(&direct), "quitting reached into a Job the Cut Host owns");
}

/// Forgetting a host retracts the marks its answer covered, because the poll that would
/// otherwise clear one is a call to the host that has just gone: left behind, a mark raises the
/// quit prompt on every close for the rest of the session with no verb that could ever satisfy
/// it (#158). The status cache goes with it, because nothing can read it once the id it is
/// keyed by can never be minted again.
#[test]
fn forgetting_a_host_releases_the_window_its_dispatch_held() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());
    // Polled before the cut, so the cache holds a reading for the forget to clear. Nothing
    // polls after it, which is what leaves the mark standing over a Job that has since ended —
    // the state an operator who closed the dialog is in.
    dev.status();
    dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");

    // The Job has to be over before the host can be forgotten at all — `forget` refuses while
    // it says it is cutting.
    let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
    wait_until(|| cameo_is_active(&direct), "the cut never started");
    direct.confirm_pass_done(cut_host::host::testing::CAMEO).unwrap();
    wait_until(|| !cameo_is_active(&direct), "the cut never finished");
    assert!(dev.a_cut_may_be_running(), "the mark is what this test is about");

    dev.forget(&id, &hosts_path, false).expect("an idle host can be forgotten");
    assert!(!dev.a_cut_may_be_running(), "a forgotten host's mark can never be cleared again");
    assert!(
        dev.last_remote_status.lock().unwrap().is_empty(),
        "a reading nothing can ever read again was kept"
    );
}

/// A press that has returned is a press that is no longer dispatching, and the desktop reads
/// exactly that through `a_press_is_dispatching`: a registration left behind would hold the
/// window for the rest of the process — every close raising a prompt no verb could satisfy —
/// as well as leaving the Job in doubt until `ID_RETENTION`, with every press until then read
/// as a retry rather than a cut. The map is asserted directly because it is the tighter
/// observation of the two.
#[test]
fn a_finished_press_stops_counting_as_dispatching() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    dev.execute_cut(aimed.clone(), a_square(10.0)).expect("dispatch");
    assert!(dev.ledger.no_press_registered(), "a finished press stayed registered");

    // And the settling answer it got really did clear the Job, which is what a leaked
    // registration would have prevented.
    let key = JobKey {
        host: id,
        device: aimed.instance_id,
        digest: job_digest(&aimed.machine_id, &a_square(10.0)),
    };
    assert!(!dev.ledger.holds(&key), "an accepted dispatch settles the Job");
}

/// Two presses of one Job that are both answered before either finishes must not each defer to
/// the other: the Job would stay in doubt with nothing outstanding, and every later press would
/// be read as a retry and answered "already accepted" while the cutter never moved.
#[test]
fn two_presses_that_both_settle_leave_nothing_in_doubt() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let key = JobKey {
        host: HostId("host-1".into()),
        device: cut_host::host::testing::CAMEO.into(),
        digest: job_digest("cameo5", &a_square(10.0)),
    };

    let (first, id, _) = dev.ledger.begin_dispatch(&key);
    let (second, joined, _) = dev.ledger.begin_dispatch(&key);
    assert_eq!(id, joined, "the second press joins the id already reserved");

    dev.ledger.settle_dispatch(&key, &id, first.attempt);
    assert!(
        dev.ledger.holds(&key),
        "the first answer said nothing about the press still dispatching"
    );

    dev.ledger.settle_dispatch(&key, &joined, second.attempt);
    assert!(
        !dev.ledger.holds(&key),
        "with every press answered the Job is no longer in doubt"
    );
}

/// An answer about an id the entry no longer holds settles that id, not the entry: the entry
/// can be pruned and re-minted while a dispatch is in flight, and whoever minted the
/// replacement may still need it.
#[test]
fn an_answer_about_a_replaced_id_leaves_the_replacement() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let key = JobKey {
        host: HostId("host-1".into()),
        device: cut_host::host::testing::CAMEO.into(),
        digest: job_digest("cameo5", &a_square(10.0)),
    };

    let (press, stale, _) = dev.ledger.begin_dispatch(&key);
    // What pruning plus a later press leaves behind: the same Job, a different id.
    let minted = cut_host::protocol::DispatchId("minted-by-someone-else".into());
    dev.ledger.replace_held(&key, minted.clone());

    dev.ledger.settle_dispatch(&key, &stale, press.attempt);
    assert_eq!(
        dev.ledger.held_id(&key),
        Some(minted),
        "an answer about the id that was replaced deleted its replacement"
    );
}

/// The production order, unlike the test above: one answered press returns before its sibling
/// is answered. Its evidence must outlive its guard, or the last answer cannot finish the Job.
#[test]
fn a_settled_press_answer_outlives_its_guard() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let key = JobKey {
        host: HostId("host-1".into()),
        device: cut_host::host::testing::CAMEO.into(),
        digest: job_digest("cameo5", &a_square(10.0)),
    };

    let (first, id, _) = dev.ledger.begin_dispatch(&key);
    let (second, joined, _) = dev.ledger.begin_dispatch(&key);
    dev.ledger.settle_dispatch(&key, &id, first.attempt);
    drop(first);
    assert!(dev.ledger.holds(&key), "the sibling is still unanswered");

    dev.ledger.settle_dispatch(&key, &joined, second.attempt);
    assert!(!dev.ledger.holds(&key), "the returned press's answer was forgotten");
}

/// A concurrent press that returns without a settling answer remains unresolved. A sibling's
/// answer cannot clear it; the next press after the group is idle is the retry that can.
#[test]
fn an_unsettled_returned_press_keeps_the_id_until_a_retry_settles() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let key = JobKey {
        host: HostId("host-1".into()),
        device: cut_host::host::testing::CAMEO.into(),
        digest: job_digest("cameo5", &a_square(10.0)),
    };

    let (answered, id, _) = dev.ledger.begin_dispatch(&key);
    let (unsettled, joined, _) = dev.ledger.begin_dispatch(&key);
    drop(unsettled);
    dev.ledger.settle_dispatch(&key, &id, answered.attempt);
    drop(answered);
    assert!(dev.ledger.holds(&key), "a sibling erased the retry id");

    let (retry, retried, first_attempt) = dev.ledger.begin_dispatch(&key);
    assert!(!first_attempt);
    assert_eq!(retried, joined);
    dev.ledger.settle_dispatch(&key, &retried, retry.attempt);
    assert!(!dev.ledger.holds(&key), "the settling retry did not resolve the id");
}

/// Only a press that began when nothing was in flight is the retry. A *concurrent* press knows
/// nothing about the outcomes an earlier press left unresolved, so its answer must not resolve
/// them — the id it shares is what a real retry still needs.
#[test]
fn a_concurrent_press_is_not_the_retry_that_resolves_an_unknown_outcome() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let key = JobKey {
        host: HostId("host-1".into()),
        device: cut_host::host::testing::CAMEO.into(),
        digest: job_digest("cameo5", &a_square(10.0)),
    };

    // One press ends with nothing that settles it: something may be cutting under this id.
    let (unsettled, id, _) = dev.ledger.begin_dispatch(&key);
    drop(unsettled);

    // The retry, and then a sibling alongside it — which is not one.
    let (retry, retried, _) = dev.ledger.begin_dispatch(&key);
    let (sibling, also, _) = dev.ledger.begin_dispatch(&key);
    assert_eq!((&retried, &also), (&id, &id), "both presses join the id in doubt");
    drop(retry);

    dev.ledger.settle_dispatch(&key, &also, sibling.attempt);
    assert!(
        dev.ledger.holds(&key),
        "a concurrent press resolved outcomes it says nothing about"
    );
}

/// A current-id answer can return before a stale-id press. The groups are independent: the
/// current id settles, and the later stale answer neither restores nor clears anything.
#[test]
fn a_current_id_answer_survives_a_later_stale_answer() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let key = JobKey {
        host: HostId("host-1".into()),
        device: cut_host::host::testing::CAMEO.into(),
        digest: job_digest("cameo5", &a_square(10.0)),
    };

    let (stale_press, stale, _) = dev.ledger.begin_dispatch(&key);
    dev.ledger.forget(&key);
    let (current_press, current, _) = dev.ledger.begin_dispatch(&key);
    assert_ne!(stale, current);

    dev.ledger.settle_dispatch(&key, &current, current_press.attempt);
    drop(current_press);
    assert!(!dev.ledger.holds(&key), "the current answer did not settle its id");

    dev.ledger.settle_dispatch(&key, &stale, stale_press.attempt);
    assert!(!dev.ledger.holds(&key), "a stale answer recreated state");
}

/// A local cut holds the window too, through the arm of the guard that involves no mark at all.
#[test]
fn a_local_cut_holds_the_window() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    assert!(!dev.a_cut_may_be_running(), "nothing has been cut yet");

    app.add_rect(10.0, 10.0);
    let plan = plan_for(&app);
    dev.cut_from_request(&app, request_from(plan)).expect("cut");
    assert!(dev.a_cut_may_be_running(), "the guard waved through a local cut in progress");
}

/// A press that never reached a host wrote no mark, so it has nothing to retract and another
/// press's mark stands. Separate from the refusal branch because it reaches the same outcome by
/// writing nothing rather than by retracting one thing: `with_host` never enters the closure,
/// so the mark and its retraction are both absent.
#[test]
fn a_press_that_never_reached_a_host_leaves_another_press_holding_the_window() {
    let dev = test_device_setup();
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
    let aimed = host_cameo(&HostId("host-1".into()));
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    // An accepted press on this cutter, not yet seen to end.
    dev.ledger.mark_dispatched(&HostId("host-1".into()), &aimed.instance_id, AttemptId::next());

    dev.execute_cut(aimed, a_square(10.0)).expect_err("nothing answers this host");
    assert!(
        dev.a_cut_may_be_running(),
        "a press that never left the machine cleared a running cut's mark"
    );
}

/// The mark is written before the request precisely so a dispatch whose reply is lost still
/// holds the window: by the time the reply fails to arrive there is nothing left to write it
/// from, and the Job may be cutting on the Pi.
#[test]
fn a_dispatch_whose_reply_was_lost_holds_the_window() {
    let host = cut_host::client::testing::start_host_answering(vec![
        cut_host::protocol::Response::Ok,
        cut_host::protocol::Response::Ok,
    ]);
    let dev = test_device_setup();
    dev.add_host(PairedHost {
        fingerprint: host.fingerprint.clone(),
        ..a_paired_host("host-1", &host.addr)
    });
    let aimed = host_cameo(&HostId("host-1".into()));
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    let err = dev.execute_cut(aimed, a_square(10.0)).expect_err("`Ok` is not an accepted dispatch");
    assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
    assert!(
        dev.a_cut_may_be_running(),
        "a dispatch nothing confirmed left the window free to close"
    );
}

/// Polling one cutter says nothing about another: the clear is keyed to the cutter that
/// answered, or a dialog open on an idle cutter would wipe the mark of the Pi cutting next to
/// it — #158 by way of its own fix.
#[test]
fn a_poll_of_one_cutter_leaves_another_holding_the_window() {
    let idle = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let watched = dev
        .pair("Idle Pi".into(), idle.addr.clone(), HOST_TOKEN.into(), idle.fingerprint.clone(), &hosts_path)
        .expect("pairing with the idle host");
    let elsewhere = HostId("host-elsewhere".into());
    dev.add_host(a_paired_host("host-elsewhere", "127.0.0.1:1"));

    // A press outstanding on the *other* host's cutter.
    dev.ledger.mark_dispatched(&elsewhere, cut_host::host::testing::CAMEO, AttemptId::next());

    *dev.connected.lock().unwrap() = Some(host_cameo(&watched));
    assert!(dev.status().actions.cut, "the aimed cutter is free, which is what makes it clear marks");
    assert!(dev.a_cut_may_be_running(), "polling one cutter cleared a mark belonging to another");
    // And the same for the wide clear `list_devices` performs.
    dev.list_devices();
    assert!(dev.a_cut_may_be_running(), "the device-list poll cleared a mark for an unreachable host");
}

/// The clear has to be as wide as the guard: a Job dispatched to a Pi the operator then aimed
/// away from is released by the device-list poll the cut dialog already runs, and by nothing
/// else. Without it the mark stood for the rest of the session and every close raised a prompt
/// no verb could satisfy (#158).
#[test]
fn a_poll_of_every_cutter_releases_a_mark_the_aim_left_behind() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    dev.connect(aimed.clone()).expect("aiming at a host is bookkeeping only");
    dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");

    // The operator goes back to their own cutter while the Pi finishes.
    dev.connect(test_instance()).expect("aiming back at the local cutter");
    assert!(dev.a_cut_may_be_running(), "the dispatch stopped counting when the aim moved");

    let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
    wait_until(|| cameo_is_active(&direct), "the cut never started");
    direct.confirm_pass_done(cut_host::host::testing::CAMEO).unwrap();
    wait_until(|| !cameo_is_active(&direct), "the cut never finished");

    wait_until(
        || {
            dev.list_devices();
            !dev.a_cut_may_be_running()
        },
        "a finished cut on an un-aimed cutter still held the window",
    );
}

/// Forgetting one host says nothing about another's cutters.
#[test]
fn forgetting_one_host_leaves_another_holding_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let cutting = HostId("host-cutting".into());
    dev.add_host(a_paired_host("host-cutting", "127.0.0.1:1"));
    dev.add_host(a_paired_host("host-elsewhere", "127.0.0.1:1"));
    dev.ledger.mark_dispatched(&cutting, cut_host::host::testing::CAMEO, AttemptId::next());

    dev.forget(&HostId("host-elsewhere".into()), &hosts_path, true).expect("forced past unreachable");
    assert!(
        dev.a_cut_may_be_running(),
        "forgetting an unrelated host discarded the warning about the one that is cutting"
    );
}

/// `retract_marks` drops only the marks it was handed. A mark written after those were captured
/// is not one of them, so it survives the retraction. `forget` refuses before it retracts when
/// it sees that mark; this pins the retraction, not the refusal.
#[test]
fn forgetting_a_host_keeps_a_mark_its_answer_did_not_cover() {
    let dev = test_device_setup();
    let id = HostId("host-1".into());
    dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

    dev.ledger.mark_dispatched(&id, cut_host::host::testing::CAMEO, AttemptId::next());
    let answered_for = dev.ledger.marks_for(&id);
    // The press that lands between the host's answer and the retraction.
    dev.ledger.mark_dispatched(&id, cut_host::host::testing::CAMEO, AttemptId::next());

    dev.ledger.retract_marks(&answered_for);
    assert!(
        dev.a_cut_may_be_running(),
        "a dispatch the host's answer never covered lost its warning anyway"
    );
}

/// Switching the host sweep from `devices` to `snapshots` must not change the list it returns:
/// the statuses are extra information for the guard, not a reason remote cutters disappear.
#[test]
fn a_reachable_hosts_cutters_are_still_listed() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");

    let listed = dev.list_devices();
    let remote: Vec<_> = listed.iter().filter(|d| d.host.as_ref() == Some(&id)).collect();
    assert_eq!(remote.len(), 2, "both of the host's cutters must be listed: {listed:?}");
    assert!(remote.iter().any(|d| d.instance_id == cut_host::host::testing::CAMEO), "{remote:?}");
    assert!(remote.iter().any(|d| d.instance_id == cut_host::host::testing::PUMA), "{remote:?}");
    assert_eq!(listed.len(), TestFactory.list_devices().len() + 2, "the local cutter must survive the host's");
}

/// A peer that misanswers the new Snapshot request is named on its row and its client is
/// discarded, just as a failed bare-device request was before the switch.
#[test]
fn a_host_that_misanswers_the_snapshot_is_recorded_as_unreachable() {
    let host = cut_host::client::testing::start_host_answering(vec![
        cut_host::protocol::Response::Ok,
        cut_host::protocol::Response::Ok,
    ]);
    let dev = test_device_setup();
    dev.add_host(PairedHost {
        fingerprint: host.fingerprint.clone(),
        ..a_paired_host("host-1", &host.addr)
    });

    let listed = dev.list_devices();
    assert!(listed.iter().all(|d| d.host.is_none()), "a host that misanswered contributes none");
    let view = dev.host_views().pop().expect("the host is still paired");
    assert_eq!(
        view.unreachable.as_deref(),
        Some("this host answered with `Ok` where `Snapshots` was expected"),
        "the row claimed a healthy host"
    );
    assert!(
        dev.host_conn(&HostId("host-1".into())).unwrap().lock().unwrap().client.is_none(),
        "the dead client was kept for the next call to reuse"
    );
}

/// An idle cutter clears its own marks, not every mark on its host. A Puma beside a cutting
/// Cameo must not wave the operator past the Cameo when the once-per-second sweep sees both.
#[test]
fn an_idle_cutter_does_not_release_its_neighbours_mark() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());
    dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");

    let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
    wait_until(|| cameo_is_active(&direct), "the cut never started");
    dev.list_devices();
    assert!(dev.a_cut_may_be_running(), "the idle Puma released the cutting Cameo's mark");
}

/// A dispatch queues behind a poll on their shared HostConnection without writing its mark
/// first. Once it owns the connection it writes the mark immediately before the request, so a
/// stale free snapshot can run before the mark or a later poll can run after the dispatch —
/// never clear a mark for a request that has not been sent yet.
#[test]
fn a_dispatch_marks_only_after_it_owns_the_host_connection() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());
    dev.list_devices(); // establish the connection before this test takes its lock
    let conn = dev.host_conn(&id).unwrap();
    let held = conn.lock().unwrap();

    std::thread::scope(|s| {
        let cut = s.spawn(|| dev.execute_cut(aimed, a_square(10.0)));
        wait_until(
            || !dev.ledger.no_press_registered(),
            "the dispatch never reached the host-connection boundary",
        );
        assert!(
            !dev.ledger.any_mark(),
            "a request blocked behind a stale poll wrote a mark the poll could clear"
        );
        drop(held);
        cut.join().unwrap().expect("dispatch");
    });
    assert!(dev.a_cut_may_be_running(), "the accepted dispatch left no mark");
}

/// Host ids remain claimed after forget. A surviving mark is keyed by that id, so recycling it
/// for an idle new Pi would let the new Pi's snapshot erase the old Pi's warning.
#[test]
fn a_forgotten_host_id_is_not_recycled_while_its_marks_can_survive() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let first = dev
        .pair("First Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("first pairing");
    dev.ledger.mark_dispatched(&first, cut_host::host::testing::CAMEO, AttemptId::next());
    dev.remove_host(&first);

    let second = dev
        .pair("Second Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("second pairing");
    assert_ne!(first, second, "a new host inherited a surviving mark's identity");
    dev.list_devices();
    assert!(dev.a_cut_may_be_running(), "the new host's idle cutter erased the forgotten host's warning");
}

/// The rule the close path actually runs, asked of `commit_close`: whether the window may go.
///
/// Readiness is the other half of the decision and is a settable fact this asks for separately;
/// the branch that consumes it is run through the real hook by
/// `the_window_hook_refuses_a_close_only_over_an_outstanding_cut_it_can_warn_about`. What the e2e
/// does pin is the webview end of the same handshake, through `emitBackendEvent`'s wait on
/// `__close_guard_ready__`.
///
/// A fresh handle per case, because committing is a state change rather than a question.
#[test]
fn a_close_is_let_go_only_with_nothing_outstanding() {
    let idle = test_device_setup();
    assert!(idle.commit_close(), "an idle desktop may close");

    let unarmed = test_device_setup();
    unarmed.ledger.mark_dispatched(&HostId("host-1".into()), cut_host::host::testing::CAMEO, AttemptId::next());
    assert!(!unarmed.commit_close(), "the window was let go over an outstanding cut");
    assert!(!unarmed.close_guard_ready(), "a warning with no listener behind it has no escape hatch");

    let armed = test_device_setup();
    armed.ledger.mark_dispatched(&HostId("host-1".into()), cut_host::host::testing::CAMEO, AttemptId::next());
    armed.set_close_guard_ready(true);
    assert!(!armed.commit_close(), "the window was let go over an outstanding cut");
    assert!(armed.close_guard_ready(), "the prompt the refusal needs was not armed");
    armed.set_close_guard_ready(false);
    assert!(!armed.close_guard_ready(), "teardown left the native guard armed");
}

/// The decision both doors share — the window's close button and the Quit menu item (#327). It was
/// inline in `on_window_event`, where no test could reach it; every arm is a different promise to
/// the operator or to a press that has not yet asked for its permit.
#[test]
fn a_request_to_leave_is_decided_the_same_for_every_door() {
    use super::CloseVerdict::{Proceed, Warn};
    let outstanding = || {
        let dev = test_device_setup();
        dev.ledger.mark_dispatched(&HostId("host-1".into()), cut_host::host::testing::CAMEO, AttemptId::next());
        dev
    };
    let can_start_a_press = |dev: &DeviceManagerHandle| dev.close_gate.permit().is_some();

    let idle = test_device_setup();
    let verdict = idle.decide_close(|| -> Result<(), String> { panic!("an idle desktop asked nothing") });
    assert_eq!(verdict, Proceed);
    assert!(!can_start_a_press(&idle), "leaving was decided but a press could still start");

    let armed = outstanding();
    armed.set_close_guard_ready(true);
    assert_eq!(armed.decide_close(|| Ok::<(), String>(())), Warn);
    assert!(can_start_a_press(&armed), "the app stays open, so a press must still be able to start");

    let undeliverable = outstanding();
    undeliverable.set_close_guard_ready(true);
    assert_eq!(undeliverable.decide_close(|| Err("no webview".to_string())), Proceed);
    assert!(!can_start_a_press(&undeliverable), "the app is going, so no press may start");

    let unarmed = outstanding();
    let verdict = unarmed.decide_close(|| -> Result<(), String> { panic!("no listener, so nothing to warn") });
    assert_eq!(verdict, Proceed);
    assert!(!can_start_a_press(&unarmed), "the app is going, so no press may start");

    // Readiness is a fact about the webview, not about the cut: an idle desktop with its listener
    // armed must still leave without asking, or every quit would raise the prompt.
    let idle_and_armed = test_device_setup();
    idle_and_armed.set_close_guard_ready(true);
    let verdict = idle_and_armed.decide_close(|| -> Result<(), String> { panic!("nothing is outstanding") });
    assert_eq!(verdict, Proceed);
    assert!(!can_start_a_press(&idle_and_armed), "leaving was decided but a press could still start");

    // A press holding its permit has written no mark yet, so the ledger calls the desktop idle.
    // It is still a Job about to start, and the operator is asked rather than waved through.
    let sending = test_device_setup();
    sending.set_close_guard_ready(true);
    let permit = sending.close_gate.permit().expect("an open gate grants a permit");
    assert!(!sending.a_cut_may_be_running(), "the ledger has no mark, which is the case under test");
    assert_eq!(sending.decide_close(|| Ok::<(), String>(())), Warn);
    assert!(can_start_a_press(&sending), "the app stays open, so a press must still be able to start");
    drop(permit);
}

/// What a window close did, as far as a test can see it from outside the run loop.
struct WindowClose {
    /// The window was still there after the request, i.e. the hook refused.
    refused: bool,
    /// `cut-in-progress` was emitted, which is what raises the prompt in the webview.
    warned: bool,
    handle: tauri::AppHandle<tauri::test::MockRuntime>,
}

impl WindowClose {
    fn a_press_can_still_start(&self) -> bool {
        use tauri::Manager;
        self.handle.state::<DeviceManagerHandle>().close_gate.permit().is_some()
    }
}

/// Asks a window of Tauri's mock runtime to close and hands the resulting `CloseRequested` to
/// `on_close_requested`, the function `main` registers, not a copy of its body.
///
/// The mock cannot dispatch to a registered window-event handler (its window dispatcher drops
/// `on_window_event` registrations), so the loop's own event is passed to the hook by hand. The
/// event carries the real `CloseRequestApi`, and the loop honours a `prevent_close` sent through
/// it. What stays unproven here is the one-line `.on_window_event(...)` registration in `main`.
///
/// The loop reports nothing about a refusal, so the test observes it the only way the loop
/// allows: a task queued behind the close request runs only if the loop is still going, and the
/// loop ends by itself when the last window goes. A refused window is then destroyed to end it.
fn close_the_window(dev: DeviceManagerHandle) -> WindowClose {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::{Listener, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

    let app = mock_builder()
        .manage(dev)
        .build(mock_context(noop_assets()))
        .expect("a mock app builds");
    let handle = app.handle().clone();

    let warned = Arc::new(AtomicBool::new(false));
    let seen = warned.clone();
    handle.listen("cut-in-progress", move |_| seen.store(true, Ordering::SeqCst));

    let window = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .expect("a mock window builds");
    let refused = Arc::new(AtomicBool::new(false));
    let outlived = refused.clone();
    app.run_return(move |app_handle, event| match event {
        RunEvent::Ready => window.close().expect("the close request is queued"),
        RunEvent::WindowEvent { label, event: event @ WindowEvent::CloseRequested { .. }, .. } => {
            let asked = app_handle
                .get_webview_window(&label)
                .expect("the window being closed exists")
                .as_ref()
                .window();
            on_close_requested(&asked, &event);
            let outlived = outlived.clone();
            app_handle
                .run_on_main_thread(move || outlived.store(true, Ordering::SeqCst))
                .expect("the marker is queued");
            window.destroy().expect("the teardown is queued");
        }
        _ => {}
    });
    WindowClose { refused: refused.load(Ordering::SeqCst), warned: warned.load(Ordering::SeqCst), handle }
}

/// `main`'s window-close hook, run for real: the arm `a_close_is_let_go_only_with_nothing_outstanding`
/// and `a_request_to_leave_is_decided_the_same_for_every_door` could only reach by their pieces.
#[test]
fn the_window_hook_refuses_a_close_only_over_an_outstanding_cut_it_can_warn_about() {
    let outstanding = || {
        let dev = test_device_setup();
        dev.ledger.mark_dispatched(&HostId("host-1".into()), cut_host::host::testing::CAMEO, AttemptId::next());
        dev
    };

    let armed = outstanding();
    armed.set_close_guard_ready(true);
    let close = close_the_window(armed);
    assert!(close.refused, "the window went while a cut may be running");
    assert!(close.warned, "the refusal raised no prompt, so the operator had no way out");
    assert!(close.a_press_can_still_start(), "the app stayed open but its presses were shut off");

    let idle = test_device_setup();
    idle.set_close_guard_ready(true);
    let close = close_the_window(idle);
    assert!(!close.refused, "an idle desktop was refused its close");
    assert!(!close.warned, "an idle desktop raised the quit prompt");
    assert!(!close.a_press_can_still_start(), "the window went but a press could still start");

    let no_listener = close_the_window(outstanding());
    assert!(!no_listener.refused, "a refusal with no listener behind it has no escape");
    assert!(!no_listener.warned, "a prompt was emitted to a page that never acknowledged a listener");
    assert!(!no_listener.a_press_can_still_start(), "the window went but a press could still start");
}

/// The native stand-down on a reload, through the function `on_page_load` calls rather than the
/// predicate it once consulted: only a page *start* clears the previous page's acknowledgement.
#[test]
fn only_a_page_start_stands_down_the_previous_listener() {
    let dev = test_device_setup();
    dev.set_close_guard_ready(true);

    window_events::note_page_load(&dev, tauri::webview::PageLoadEvent::Finished);
    assert!(dev.close_guard_ready(), "a finished load disarmed a listener the new page had already installed");

    window_events::note_page_load(&dev, tauri::webview::PageLoadEvent::Started);
    assert!(!dev.close_guard_ready(), "a reload left the previous page's acknowledgement standing");
}

/// An admitted dispatch is not free merely because the worker still publishes Idle. This is
/// the one bit the host added to Snapshot for the accept-to-worker-publication gap.
#[test]
fn a_claimed_idle_snapshot_is_not_free() {
    let dev = test_device_setup();
    let mut snapshot = cut_host::protocol::DeviceSnapshot {
        info: test_instance(),
        status: dev.status(),
        job_id: None,
        claimed: true,
    };
    assert!(snapshot.status.actions.cut, "status alone calls the cutter free");
    assert!(!snapshot_is_free(&snapshot), "an admitted dispatch was treated as idle");
    snapshot.claimed = false;
    assert!(snapshot_is_free(&snapshot), "the genuinely idle cutter did not clear");
}

/// A press between `begin_dispatch` and the host's connection has written no mark yet, so the
/// mark alone cannot answer for it. It is a Job about to start, which is what the operator must
/// not be waved past (#158).
#[test]
fn a_press_queued_for_a_host_connection_holds_the_window() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());
    dev.list_devices(); // establish the connection before this test holds it
    let conn = dev.host_conn(&id).unwrap();
    let held = conn.lock().unwrap();

    std::thread::scope(|s| {
        let cut = s.spawn(|| dev.execute_cut(aimed, a_square(10.0)));
        wait_until(
            || dev.ledger.a_press_is_dispatching(),
            "the press never reached the host-connection boundary",
        );
        assert!(
            !dev.ledger.any_mark(),
            "the mark is written under the connection, so there is none to answer with yet"
        );
        assert!(dev.a_cut_may_be_running(), "a queued press was invisible to the close guard");
        assert!(!dev.commit_close(), "the window was let go over a press about to dispatch");
        drop(held);
        cut.join().unwrap().expect("dispatch");
    });
}

/// A press queued for a host's connection is not stopped by the map entry going: it lifted the
/// `Arc` out before it queued. So a forget racing one is refused outright, and a press that gets
/// the connection after a forget completed sends nothing — the two halves of the same gap.
///
/// What is at stake is a Job started on a host whose token has just been discarded: no cancel
/// route, and a mark no poll can clear, because the poll is a call to the host that was removed.
#[test]
fn a_forget_and_a_queued_press_cannot_cross() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());
    dev.list_devices(); // establish the connection before this test holds it
    let conn = dev.host_conn(&id).unwrap();
    let held = conn.lock().unwrap();

    std::thread::scope(|s| {
        let cut = s.spawn(|| dev.execute_cut(aimed.clone(), a_square(10.0)));
        wait_until(
            || dev.ledger.a_press_is_dispatching_to(&id),
            "the press never reached the host-connection boundary",
        );

        // Mid-send is the one press this verb cannot answer for, so it is refused rather than
        // raced. Forced makes no difference: the force is about what a host said, not about a
        // press this desktop is still making.
        for forced in [false, true] {
            let err = dev
                .forget(&id, &hosts_path, forced)
                .expect_err("a forget crossed a press that was already sending");
            assert_eq!(err.code, "host_busy", "got {err:?}");
            assert!(err.message.contains("being sent"), "the refusal must say why: {err:?}");
        }

        drop(held);
        cut.join().unwrap().expect("dispatch");
    });

    // The other half, and the reason the transition lives on the connection rather than in the
    // map: a press that has already lifted the `Arc` is not reached by the entry going. During
    // the filesystem write it wakes, sees `Forgetting`, and returns immediately — host control
    // is not held behind the disk and nothing crosses into a send.
    let conn = dev.host_conn(&id).unwrap();
    let mut held = conn.lock().unwrap();
    std::thread::scope(|s| {
        let cut = s.spawn(|| dev.execute_cut(host_cameo(&id), a_square(10.0)));
        wait_until(
            || dev.ledger.a_press_is_dispatching_to(&id),
            "the second press never reached the boundary",
        );
        held.lifecycle = HostLifecycle::Forgetting;
        drop(held);
        let err = cut
            .join()
            .unwrap()
            .expect_err("a host being forgotten must not be dispatched to");
        assert_eq!(err.code, "host_busy", "got {err:?}");
        assert!(
            err.message.contains("being forgotten"),
            "the transition was hidden: {err:?}"
        );
    });
}

/// The early `a_press_is_dispatching_to` check runs before the host is asked. A press that
/// starts after that, marks, and finishes before `forget` takes the connection again is not
/// dispatching anymore — and the idle answer did not cover its mark. Forgetting past it
/// discards the token for a Job that answer never saw.
#[test]
fn a_press_that_lands_after_the_idle_answer_refuses_the_forget() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = Arc::new(test_device_setup());
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");

    // Pair leaves the client unset. The question connects, releases the connection, then
    // blocks on this lock: the window between the idle answer and publishing `Forgetting`.
    let persist = dev.hosts_persist.lock().unwrap();
    let dev_forget = Arc::clone(&dev);
    let forget_id = id.clone();
    let path = hosts_path.clone();
    let forget = std::thread::spawn(move || dev_forget.forget(&forget_id, &path, false));
    let conn = dev.host_conn(&id).unwrap();
    wait_until(
        || conn.try_lock().is_ok_and(|guard| guard.client.is_some()),
        "the idle question never released the connection",
    );
    dev.ledger.mark_dispatched(&id, cut_host::host::testing::CAMEO, AttemptId::next());
    drop(persist);

    let err = forget
        .join()
        .unwrap()
        .expect_err("a press that landed after the answer was forgotten past");
    assert_eq!(err.code, "host_busy", "got {err:?}");
    assert!(err.message.contains("being sent"), "the refusal must say why: {err:?}");
    assert_eq!(dev.paired_hosts().len(), 1, "refused, so still paired");
    assert_eq!(
        conn.lock().unwrap().lifecycle,
        HostLifecycle::Active,
        "a refused forget published Forgetting"
    );
    assert!(dev.a_cut_may_be_running(), "the press's mark was retracted with a token it still needs");
}

/// The other arm of the same re-check. A press that has `begin_dispatch`'d and not yet marked
/// holds no mark the answer failed to cover, and the early check already passed. It is still
/// a Job about to start.
#[test]
fn a_press_still_dispatching_after_the_idle_answer_refuses_the_forget() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = Arc::new(test_device_setup());
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");

    let persist = dev.hosts_persist.lock().unwrap();
    let dev_forget = Arc::clone(&dev);
    let forget_id = id.clone();
    let path = hosts_path.clone();
    let forget = std::thread::spawn(move || dev_forget.forget(&forget_id, &path, false));
    let conn = dev.host_conn(&id).unwrap();
    wait_until(
        || conn.try_lock().is_ok_and(|guard| guard.client.is_some()),
        "the idle question never released the connection",
    );
    let key = JobKey {
        host: id.clone(),
        device: cut_host::host::testing::CAMEO.into(),
        digest: job_digest("cameo5", &a_square(10.0)),
    };
    let press = dev.ledger.begin_dispatch(&key);
    assert!(dev.ledger.marks_for(&id).is_empty(), "this arm is the press that has not marked yet");
    drop(persist);

    let err = forget
        .join()
        .unwrap()
        .expect_err("a press still dispatching was forgotten past");
    assert_eq!(err.code, "host_busy", "got {err:?}");
    assert!(err.message.contains("being sent"), "the refusal must say why: {err:?}");
    assert_eq!(dev.paired_hosts().len(), 1, "refused, so still paired");
    assert_eq!(conn.lock().unwrap().lifecycle, HostLifecycle::Active);
    drop(press);
}

/// The production writer for the flag above: a forget leaves the connection itself answering,
/// so a press that lifted the `Arc` before the map entry went finds the refusal rather than a
/// send.
#[test]
fn a_forget_leaves_the_connection_refusing() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let conn = dev.host_conn(&id).unwrap();
    assert_eq!(
        conn.lock().unwrap().lifecycle,
        HostLifecycle::Active,
        "a fresh pairing starts answering"
    );
    // Aimed before the forget, because the press below has to fail on the host rather than on
    // the aim: `forget` clears an aim that named the host it removed.
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    dev.forget(&id, &hosts_path, false).expect("an idle host is forgettable");
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    assert_eq!(
        conn.lock().unwrap().lifecycle,
        HostLifecycle::Forgotten,
        "the forget left the connection answering"
    );
    let err = dev
        .execute_cut(aimed, a_square(10.0))
        .expect_err("a forgotten host must not be dispatched to");
    assert_eq!(err.code, "unknown_host", "got {err:?}");
}

/// Once a close is committed, a press that reaches the connection afterwards sends nothing: a
/// Job started now would keep cutting with the process that started it gone.
#[test]
fn a_press_that_reaches_the_host_after_a_committed_close_sends_nothing() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());

    assert!(dev.commit_close(), "nothing is outstanding, so the window may go");
    let err = dev.execute_cut(aimed, a_square(10.0)).expect_err("a committed close sends nothing");
    assert_eq!(err.code, "closing", "got {err:?}");

    let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
    stays_false(|| cameo_is_active(&direct), "a cut started after the window was let go");
    assert!(!dev.a_cut_may_be_running(), "a press that sent nothing left a mark behind");
}

/// A cutter whose stop nothing confirmed is claimed by nothing and still not free, and this
/// desktop's own polls refuse to clear its mark. Forgetting refuses it — with or without
/// `force`, because the blade may still be moving and the host's own `Reconnect` is a verb that
/// fixes it. Force is for a state nothing can recover, not for one click away from recovery.
#[test]
fn forgetting_a_host_refuses_a_stop_nothing_confirmed_until_a_reconnect() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());
    dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");

    wait_for_remote(&dev, |s| s.actions.confirm, "the Job to park");
    dev.cancel().expect("cancel");
    wait_for_remote(&dev, |s| s.ended == Some(driver_core::Ended::Cancelled), "the cancel to land");
    assert!(!dev.status().actions.cut, "nothing saw the machine stop, so it is not free");

    for forced in [false, true] {
        let err = dev.forget(&id, &hosts_path, forced).expect_err("an unconfirmed stop is not idleness");
        assert_eq!(err.code, "host_busy", "force={forced}, got {err:?}");
        assert!(err.message.contains("reconnect"), "the way out is not named: {}", err.message);
        assert_eq!(dev.paired_hosts().len(), 1, "refused, so still paired");
    }
    assert!(dev.a_cut_may_be_running(), "the refusals must leave the warning standing");

    // The verb the refusal names, and then the host can go.
    dev.reconnect().expect("the host re-opens its own cutter");
    dev.forget(&id, &hosts_path, false).expect("a reconnected cutter is free");
    assert!(dev.paired_hosts().is_empty(), "the host stayed paired after its cutter came back");
    assert!(!dev.a_cut_may_be_running(), "the marks go with a host that answered every cutter free");
}

/// Which state each answer lands in, asked of the classification rather than of hardware this
/// test cannot unplug. This is what decides both refusals and which of them `force` may pass,
/// and `worst_state` is what a host with several cutters is judged by.
#[test]
fn a_cutter_is_classified_by_what_can_still_reach_it() {
    let claimed = cut_host::protocol::DeviceSnapshot {
        info: test_instance(),
        status: CutStatus::disconnected(),
        job_id: None,
        claimed: true,
    };
    // Claimed outranks everything the status says, which is the whole of #158: a dispatch on
    // its way to the manager is a Job the status cannot describe yet.
    assert_eq!(classify(&claimed), CutterState::Claimed);

    let mut stopped = claimed.clone();
    stopped.claimed = false;
    stopped.status.ended = Some(driver_core::Ended::Cancelled);
    assert_eq!(classify(&stopped), CutterState::Stalled, "a reconnect clears this one");

    let mut disconnected = stopped.clone();
    disconnected.status.ended = None;
    assert_eq!(
        classify(&disconnected),
        CutterState::Unreachable,
        "nothing here recovers a cutter whose hardware is gone, which is what force is for"
    );

    let mut free = disconnected.clone();
    free.status.actions.cut = true;
    assert_eq!(classify(&free), CutterState::Free);
    assert!(snapshot_is_free(&free), "the only state a poll clears a mark on");

    // A cancel something actually saw come to rest is idleness, not a stall: the cutter says it
    // would take another Job, and that outranks how the last one ended. The ordinary confirmed
    // cancel for any pollable cutter lands here, and reading `ended` first would make it
    // unforgettable and its mark unclearable.
    let mut confirmed_stop = stopped.clone();
    confirmed_stop.status.actions.cut = true;
    assert_eq!(classify(&confirmed_stop), CutterState::Free);
    assert!(snapshot_is_free(&confirmed_stop), "a confirmed cancel must let its mark clear");
    assert_eq!(worst_state(&[confirmed_stop]), CutterState::Free);

    // A host is judged by its worst cutter, in that order — one free cutter does not speak for
    // the one beside it that is still holding a Job.
    assert_eq!(worst_state(&[free.clone(), disconnected.clone()]), CutterState::Unreachable);
    assert_eq!(worst_state(&[disconnected, stopped.clone()]), CutterState::Stalled);
    assert_eq!(worst_state(&[stopped, claimed]), CutterState::Claimed);
    assert_eq!(worst_state(&[free]), CutterState::Free);
    assert_eq!(worst_state(&[]), CutterState::Free, "a host with no cutters holds nothing");
}

/// The gate's other half: a press holding the window refuses the close outright, whatever
/// `a_cut_may_be_running` says. It has to be the lock and not a count, because a local press
/// between the aim check and the worker is invisible to every count there is.
#[test]
fn a_close_is_refused_while_a_press_holds_the_send_window() {
    let dev = test_device_setup();
    let permit = dev.close_gate.permit().expect("an open window");

    assert!(!dev.a_cut_may_be_running(), "nothing is outstanding, so only the permit can refuse");
    assert!(!dev.commit_close(), "the window was let go over a press already sending");
    drop(permit);
    assert!(dev.commit_close(), "and it must go once the press is away");
}

/// The stand-down arms: the webview could not be told, so the window goes anyway — and a press
/// that has not yet asked must still start nothing. The third member of a pair this branch
/// wrote for `commit_close` and for the confirmed quit.
#[test]
fn a_press_after_a_stood_down_close_sends_nothing() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());
    let direct =
        cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();

    dev.commit_close_regardless();

    let err = dev.execute_cut(aimed, a_square(10.0)).expect_err("a stood-down close sends nothing");
    assert_eq!(err.code, "closing", "got {err:?}");
    stays_false(|| cameo_is_active(&direct), "a Job started after the window was let go");
}

/// A local press is gated the same way, and has to be: nothing sees it between the aim check
/// and the worker, and a blade that starts as the process exits loses the transport driving it.
#[test]
fn a_local_press_after_a_committed_close_starts_nothing() {
    let dev = test_device_setup();
    let aimed = dev.connected.lock().unwrap().clone().expect("the local cutter is aimed at");

    assert!(dev.commit_close(), "nothing is outstanding, so the window may go");
    let err = dev.execute_cut(aimed, a_square(10.0)).expect_err("a committed close starts nothing");
    assert_eq!(err.code, "closing", "got {err:?}");
    stays_false(|| dev.status().is_active(), "a local cut started after the window was let go");
}

/// The operator answered the prompt, so the close is happening: a press that has not yet asked
/// for its permit sends nothing. The route `commit_close` refused and the operator overrode is
/// the likeliest way into #158, and it commits without asking whether anything is outstanding.
#[test]
fn a_press_after_a_confirmed_quit_sends_nothing() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    let aimed = host_cameo(&id);
    *dev.connected.lock().unwrap() = Some(aimed.clone());
    let direct =
        cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();

    dev.commit_close_after_confirmation();

    let err = dev.execute_cut(aimed, a_square(10.0)).expect_err("a confirmed quit sends nothing");
    assert_eq!(err.code, "closing", "got {err:?}");
    stays_false(|| cameo_is_active(&direct), "a Job started after the operator confirmed the quit");
}

/// The flag is set before the wait, not after it, and the wait has an end.
///
/// Both halves are observed with a permit held, which is the only vantage they differ from: set
/// second, and this press could still send for the whole of `CLOSE_GATE_WAIT` — the Job after
/// the answer the gate exists to prevent. Waited unbounded, and "quit anyway" hangs harder than
/// the guard it is the escape from (#116).
///
/// "Before" has to be measured, not merely observed: the wait is bounded, so a flag set after
/// it is still set eventually and any assertion that waits for it passes either way. The
/// two timescales are orders of magnitude apart — the store is observed in microseconds, the
/// wait is two seconds — so asserting against a quarter of the bound tells them apart without
/// racing anything.
#[test]
fn a_confirmed_quit_commits_before_it_waits_and_the_wait_ends() {
    let dev = test_device_setup();
    let permit = dev.close_gate.permit().expect("an open window");

    std::thread::scope(|s| {
        let began = std::time::Instant::now();
        let quit = s.spawn(|| dev.commit_close_after_confirmation());
        wait_until(
            || dev.close_gate.is_closing(),
            "the close was never committed",
        );
        let took = began.elapsed();
        assert!(
            took < CLOSE_GATE_WAIT / 4,
            "the close was committed only after the wait it should precede, in {took:?}"
        );
        assert!(dev.close_gate.permit().is_none(), "a later press was not refused during the wait");

        // The wait's floor, which the two assertions above cannot see: they are upper bounds, so
        // a wait deleted outright satisfies both. A thread cannot finish early, and a correct
        // one cannot finish at all while this permit stands.
        std::thread::sleep(CLOSE_GATE_WAIT / 4);
        assert!(!quit.is_finished(), "the confirmed quit did not wait for the send in flight");

        // Not `wait_until`: its own deadline is `CLOSE_GATE_WAIT`, the very bound being waited
        // out, so the margin would be one poll interval of luck.
        let ends_by = std::time::Instant::now() + CLOSE_GATE_WAIT * 2;
        while !quit.is_finished() {
            assert!(
                std::time::Instant::now() < ends_by,
                "the confirmed quit waited on a send with no deadline"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        drop(permit);
    });
}

/// A confirmed quit stops the local cutter before it waits out a permit. The permit is what
/// holds the wait; the local Job does not, because its own permit ended when `cut` returned.
/// A cancel placed after the wait would still be in the future when a bound shorter than that
/// wait expires, and the worker would keep transmitting for the whole of it.
#[test]
fn a_confirmed_quit_stops_local_motion_before_it_waits() {
    let mut app = AppState::new();
    let dev = test_device_setup();
    app.add_rect(10.0, 10.0);
    let plan = plan_for(&app);
    dev.cut_from_request(&app, request_from(plan)).expect("cut");
    assert!(dev.status().is_active(), "the local cut must be parked mid-flight to be cancellable");

    let permit = dev.close_gate.permit().expect("an open window");
    std::thread::scope(|s| {
        let quit = s.spawn(|| dev.commit_close_after_confirmation());
        let deadline = std::time::Instant::now() + CLOSE_GATE_WAIT / 2;
        let stopped = loop {
            let status = dev.status();
            if status.ended == Some(driver_core::Ended::Cancelled) {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the local cutter was still moving once the quit should have stopped it, at {:?}",
                status.phase
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!(!quit.is_finished(), "stopping the cutter waited out the permit first");
        assert!(dev.close_gate.permit().is_none(), "a later press was not refused");
        assert!(!stopped.actions.cut, "quitting left the local cutter taking Jobs");
        drop(permit);
        quit.join().unwrap();
    });
}

/// A cutter nothing can reach is the one refusal `force` exists to pass, asked through `forget`
/// in both directions: the unforced attempt refuses under a code of its own, and the forced one
/// removes the host. Its own code because the webview offers the force from the code — under
/// `host_busy` the operator would read a refusal naming a control they cannot see.
#[test]
fn forgetting_a_host_whose_cutter_is_offline_refuses_until_forced() {
    let offline = cut_host::protocol::DeviceSnapshot {
        info: test_instance(),
        status: CutStatus::disconnected(),
        job_id: None,
        claimed: false,
    };
    let host = cut_host::client::testing::start_host_answering(vec![
        cut_host::protocol::Response::Ok,
        cut_host::protocol::Response::Snapshots(vec![offline.clone()]),
        cut_host::protocol::Response::Snapshots(vec![offline]),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    // Added rather than paired: pairing asks its own question, and this host is scripted to
    // answer the two `forget` asks.
    let id = HostId("host-1".into());
    dev.add_host(PairedHost {
        fingerprint: host.fingerprint.clone(),
        ..a_paired_host("host-1", &host.addr)
    });

    let err = dev.forget(&id, &hosts_path, false).expect_err("a cutter that is not free is not idle");
    assert_eq!(err.code, "host_cutter_unreachable", "got {err:?}");
    assert!(err.message.contains("reconnect it"), "the refusal must name the verb to try: {err:?}");
    assert!(err.message.contains("forget the host anyway"), "and the escape: {err:?}");
    assert_eq!(dev.paired_hosts().len(), 1, "a refusal left the host half-removed");

    dev.forget(&id, &hosts_path, true).expect("a cutter nothing can recover must stay forgettable");
    assert!(dev.paired_hosts().is_empty(), "the host row could never be removed");
}

/// The `claimed` bit at the call site that reads it, not just as a predicate: the cut dialog's
/// own once-a-second poll must not clear the mark of a dispatch the worker has not published
/// yet. `status` and `list_devices` are separate readers, so both are asked.
#[test]
fn a_poll_does_not_clear_the_mark_of_a_claimed_cutter() {
    for (reader, poll) in [
        ("status", &(|dev: &DeviceManagerHandle| {
            dev.status();
        }) as &dyn Fn(&DeviceManagerHandle)),
        ("list_devices", &|dev: &DeviceManagerHandle| {
            dev.list_devices();
        }),
    ] {
        let dev = test_device_setup();
        let id = HostId("host-1".into());
        let aimed = DeviceInfo { host: Some(id.clone()), ..test_instance() };
        let claimed = cut_host::protocol::DeviceSnapshot {
            info: aimed.clone(),
            status: dev.status(),
            job_id: None,
            claimed: true,
        };
        assert!(claimed.status.actions.cut, "{reader}: status alone calls this cutter free");
        let host = cut_host::client::testing::start_host_answering(vec![
            cut_host::protocol::Response::Ok,
            cut_host::protocol::Response::Snapshots(vec![claimed]),
        ]);
        dev.add_host(PairedHost {
            fingerprint: host.fingerprint.clone(),
            ..a_paired_host("host-1", &host.addr)
        });
        dev.ledger.mark_dispatched(&id, &aimed.instance_id, AttemptId::next());
        *dev.connected.lock().unwrap() = Some(aimed);

        poll(&dev);
        assert!(
            dev.a_cut_may_be_running(),
            "{reader}: a poll cleared the mark of a dispatch the worker had not published yet"
        );
    }
}

/// A host that answered with every cutter free has said the marks it holds are stale, and they
/// go with it — including one written while the question was still queued, which that answer
/// speaks for.
///
/// What this does *not* pin is where the batch is captured. `forget` reads it inside the same
/// connection the answer comes over, so a press that wins the connection first is inside the
/// answer and a press that wins it afterwards keeps its own mark; both orders retract the same
/// set here, and separating them needs a seam in the middle of `forget` that exists only for
/// the test. Structural, like `begin_dispatch`'s one call: the ordering is the code's shape
/// rather than an assertion.
#[test]
fn forgetting_a_host_retracts_the_marks_its_answer_covered() {
    let host = start_loopback_host();
    let dir = tempfile::tempdir().unwrap();
    let hosts_path = dir.path().join("hosts.json");
    let dev = test_device_setup();
    let id = dev
        .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
        .expect("pairing with the loopback host");
    dev.list_devices(); // establish the connection before this test holds it
    let conn = dev.host_conn(&id).unwrap();
    let held = conn.lock().unwrap();

    std::thread::scope(|s| {
        let forgetting = s.spawn(|| dev.forget(&id, &hosts_path, false));
        dev.ledger.mark_dispatched(&id, cut_host::host::testing::CAMEO, AttemptId::next());
        drop(held);
        forgetting.join().unwrap().expect("an idle host can be forgotten");
    });
    assert!(
        !dev.a_cut_may_be_running(),
        "a mark the host's answer covered outlived the host it named"
    );
}

// --- presets: identity is (machine_id, id), so one cutter's entry cannot destroy another's ---

fn a_user_preset(machine: &str, id: &str, force: u32) -> MaterialPreset {
    MaterialPreset {
        id: id.into(),
        name: format!("{machine} {id}"),
        machine_id: machine.into(),
        settings: cutplan::presets::PresetSettings {
            speed: Some(5), force: Some(force), repeat_count: 1,
        },
        builtin: false,
    }
}

/// The whole of #153, against a temporary presets file: an operator's id is their own string,
/// so `my-vinyl` names one material on a Cameo and another on a Puma. Keyed on the id alone,
/// saving one overwrote the other and deleting one removed both — with the preset editor (#55)
/// that is an operator watching their settings vanish.
#[test]
fn a_presets_id_belongs_to_one_machine_when_saved_deleted_or_shadowed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("presets.json");

    save_preset(&path, a_user_preset("cameo5", "my-vinyl", 11)).expect("saved");
    save_preset(&path, a_user_preset("puma", "my-vinyl", 22)).expect("the other machine's");

    let force_of = |machine: &str| {
        list_presets(&path, machine).unwrap().into_iter()
            .find(|p| p.id == "my-vinyl").map(|p| p.settings.force)
    };
    assert_eq!(force_of("cameo5"), Some(Some(11)), "saving the Puma's entry overwrote it");
    assert_eq!(force_of("puma"), Some(Some(22)));

    // Editing one is a save over the same pair, and touches only that machine's entry.
    save_preset(&path, a_user_preset("puma", "my-vinyl", 33)).expect("edited");
    assert_eq!(force_of("cameo5"), Some(Some(11)), "an edit on one machine changed the other");
    assert_eq!(force_of("puma"), Some(Some(33)));

    // A delete names the machine, and cannot reach another's entry under the same id.
    delete_preset(&path, "puma", "my-vinyl").expect("deleted");
    assert_eq!(force_of("puma"), None);
    assert_eq!(force_of("cameo5"), Some(Some(11)),
        "deleting the Puma's preset removed the Cameo's");

    // Shadowing keys on the pair too: an entry under one machine named with another's builtin
    // id leaves that builtin listed.
    let shadow = cutplan::presets::builtin_presets().into_iter()
        .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins").id;
    save_preset(&path, a_user_preset("puma", &shadow, 24)).expect("saved under the Puma");
    let cameo = list_presets(&path, "cameo5").unwrap();
    let listed: Vec<_> = cameo.iter().filter(|p| p.id == shadow).collect();
    assert_eq!(listed.len(), 1, "one entry for {shadow} on the Cameo, got {listed:#?}");
    assert!(listed[0].builtin, "the Cameo's builtin was shadowed by a Puma entry");
}

/// A `presets.json` written before #153 loads with the same entries afterwards — every entry
/// already carried its `machine_id`, so only the uniqueness key changed — and deleting a user
/// shadow still reveals that machine's builtin.
#[test]
fn a_presets_file_written_before_the_pair_key_loads_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("presets.json");
    let builtin = cutplan::presets::builtin_presets().into_iter()
        .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins");
    std::fs::write(&path, format!(r#"{{"version":1,"presets":[
        {{"id":"{}","name":"Mine","machine_id":"cameo5",
         "settings":{{"speed":1,"force":99,"repeat_count":1}},"builtin":false}},
        {{"id":"my-vinyl","name":"Vinyl","machine_id":"puma",
         "settings":{{"speed":2,"force":2,"repeat_count":1}},"builtin":false}}
    ]}}"#, builtin.id)).unwrap();

    let cameo = list_presets(&path, "cameo5").unwrap();
    let shadowing = cameo.iter().find(|p| p.id == builtin.id).expect("the shadow loads");
    assert_eq!(shadowing.name, "Mine", "the user entry still shadows its own machine's builtin");
    assert!(list_presets(&path, "puma").unwrap().iter().any(|p| p.id == "my-vinyl"),
        "the other machine's entry loads unchanged");

    delete_preset(&path, "cameo5", &builtin.id).expect("deleted");
    let revealed = list_presets(&path, "cameo5").unwrap();
    let after = revealed.iter().find(|p| p.id == builtin.id).expect("the builtin is back");
    assert!(after.builtin, "deleting a user shadow must reveal the builtin it hid");
    assert_eq!(after.name, builtin.name);
}

/// A user entry under a builtin's own pair hides it, and the app offers no way back to the
/// settings it shipped with — so the save is refused rather than the material lost.
#[test]
fn saving_over_a_builtins_pair_is_refused_and_leaves_it_shipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("presets.json");
    let builtin = cutplan::presets::builtin_presets().into_iter()
        .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins");

    let err = save_preset(&path, a_user_preset("cameo5", &builtin.id, 12)).unwrap_err();
    assert_eq!(err.code, "builtin_preset",
        "a save over a builtin's pair was allowed through: {}", err.message);
    assert!(err.message.contains(&builtin.id),
        "the refusal names no preset: {}", err.message);

    assert!(user_entries(&path).unwrap().is_empty(),
        "a refused save still wrote an entry to the file");
    let listed = list_presets(&path, "cameo5").unwrap();
    let after = listed.iter().find(|p| p.id == builtin.id).expect("the builtin is still listed");
    assert!(after.builtin, "the refused save shadowed the builtin anyway");
    assert_eq!(after.settings, builtin.settings, "the shipped settings were overwritten");
}

/// Each of these is a preset the operator cannot use afterwards: an id-less entry is dropped
/// on the next load, a blank name is a picker row naming no material, and settings past the
/// machine's edge are refused at the cut. All three are refused where they are typed.
#[test]
fn a_preset_the_operator_could_not_use_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("presets.json");

    let no_id = MaterialPreset { id: String::new(), ..a_user_preset("cameo5", "mine", 12) };
    let no_machine = MaterialPreset { machine_id: String::new(), ..a_user_preset("cameo5", "mine", 12) };
    let blank_name = MaterialPreset { name: "   ".into(), ..a_user_preset("cameo5", "mine", 12) };
    let past_the_edge = a_user_preset("cameo5", "mine", 99);

    for (preset, what) in [(no_id, "an empty id"), (no_machine, "no machine"),
                           (blank_name, "a blank name"), (past_the_edge, "a force out of range")] {
        let err = save_preset(&path, preset).unwrap_err();
        assert_eq!(err.code, "invalid_preset", "{what} was saved: {}", err.message);
    }
    assert!(user_entries(&path).unwrap().is_empty(),
        "a refused save still wrote an entry to the file");
}

/// An entry with two faults is named by the one the operator can act on. A pair that names a
/// builtin is unsavable whatever it holds, so reporting the force first sends them to fix a
/// number that was never what refused the save.
#[test]
fn what_a_preset_is_refuses_it_before_what_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("presets.json");
    let builtin = cutplan::presets::builtin_presets().into_iter()
        .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins");

    let both_wrong = a_user_preset("cameo5", &builtin.id, 99);
    let err = save_preset(&path, both_wrong).unwrap_err();
    assert_eq!(err.code, "builtin_preset",
        "a shipped pair was reported as a settings fault: {}", err.message);
}

/// A delete that removed nothing used to report success, leaving the entry listed — which
/// reads as the app ignoring the operator. Which nothing it was is worth saying: a builtin is
/// not theirs to delete, an unsaved id never existed.
#[test]
fn a_delete_that_removed_nothing_says_which_nothing_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("presets.json");
    save_preset(&path, a_user_preset("cameo5", "my-vinyl", 12)).expect("saved");

    let unknown = delete_preset(&path, "cameo5", "never-saved").unwrap_err();
    assert_eq!(unknown.code, "unknown_preset",
        "deleting an id nobody saved reported success: {}", unknown.message);

    let builtin = cutplan::presets::builtin_presets().into_iter()
        .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins");
    let shipped = delete_preset(&path, "cameo5", &builtin.id).unwrap_err();
    assert_eq!(shipped.code, "builtin_preset",
        "deleting an unshadowed builtin reported success: {}", shipped.message);

    assert!(list_presets(&path, "cameo5").unwrap().iter().any(|p| p.id == "my-vinyl"),
        "a refused delete removed the operator's own entry");
}

/// The refusals above must not reach the ordinary case: the operator's own preset is still
/// saved, edited over its pair, and deleted.
#[test]
fn a_preset_of_the_operators_own_still_saves_edits_and_deletes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("presets.json");

    save_preset(&path, a_user_preset("cameo5", "my-vinyl", 12)).expect("saved");
    save_preset(&path, a_user_preset("cameo5", "my-vinyl", 21)).expect("edited");
    let edited = list_presets(&path, "cameo5").unwrap().into_iter()
        .find(|p| p.id == "my-vinyl").expect("the edit is listed");
    assert_eq!(edited.settings.force, Some(21), "the edit did not reach the file");

    delete_preset(&path, "cameo5", "my-vinyl").expect("deleted");
    assert!(!list_presets(&path, "cameo5").unwrap().iter().any(|p| p.id == "my-vinyl"),
        "the entry survived its delete");
}

/// The conversion, end to end below the IPC call. Every one of the five sites used to send
/// the code `preset_error` with a `Debug` rendering of the value, so a file this build is
/// too old to read and a damaged one arrived under the same code, told apart only by the
/// discriminant inside the message (#278).
#[test]
fn a_presets_file_this_build_cannot_read_is_refused_in_words_with_its_own_code() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("presets.json");
    std::fs::write(&path, r#"{"presets":[]}"#).expect("wrote the fixture");

    // `list_presets` is the read side; `delete_preset` reaches the same read through
    // `user_entries`, so both must speak cutplan's words rather than each writing their own.
    for err in [list_presets(&path, "cameo5").unwrap_err(),
                delete_preset(&path, "cameo5", "my-vinyl").unwrap_err()] {
        assert_eq!(err.code, "presets_corrupt", "{}", err.message);
        assert_eq!(err.message, "the presets file does not state a usable whole-number \
                                 version, so this build cannot tell what format it is in");
    }

    std::fs::write(&path, r#"{"version":3,"presets":[]}"#).expect("wrote the fixture");
    let newer = list_presets(&path, "cameo5").unwrap_err();
    assert_eq!(newer.code, "presets_unknown_version",
        "a version this build does not read shares a code with a damaged file: {}", newer.message);
}

/// A refusal that does not depend on writing the file must not lose the race to one that
/// does: an entry the editor should never have sent is named as that, not as a disk fault,
/// however the disk is behaving. This is the ordering the e2e fake mirrors.
#[test]
fn what_a_preset_is_refuses_it_before_the_file_is_touched() {
    let dir = tempfile::tempdir().unwrap();
    // A directory at the presets path, not a file where its parent belongs: reading a
    // directory fails on every platform, where a non-directory *parent* is `ENOTDIR` on Unix
    // and `ERROR_PATH_NOT_FOUND` on Windows — which Rust maps to `NotFound`, so `load_presets`
    // would call it a first run there and this test would read a disk fault it never got
    // (Codex on PR #280).
    let path = dir.path().join("presets.json");
    std::fs::create_dir(&path).expect("put a directory where the file goes");

    // Every refusal `save_preset` makes without touching the file, not just one: with only
    // the blank name asserted, moving any of the other four below the read left this green
    // while those saves reported a disk fault (Codex on PR #280).
    let mine = || a_user_preset("cameo5", "mine", 12);
    let builtin = cutplan::presets::builtin_presets().into_iter()
        .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins");
    let unsavable = [
        ("a blank name", MaterialPreset { name: "  ".into(), ..mine() }, "invalid_preset"),
        ("an empty id", MaterialPreset { id: String::new(), ..mine() }, "invalid_preset"),
        ("no machine", MaterialPreset { machine_id: String::new(), ..mine() }, "invalid_preset"),
        ("a force out of range", a_user_preset("cameo5", "mine", 99), "invalid_preset"),
        ("a builtin's pair", a_user_preset("cameo5", &builtin.id, 12), "builtin_preset"),
    ];
    for (what, preset, code) in unsavable {
        let err = save_preset(&path, preset).unwrap_err();
        assert_eq!(err.code, code,
            "{what} was reported as a disk fault: {}", err.message);
    }

    // And the file really is unusable, so the assertion above is not vacuous: a valid entry
    // is refused by it. `presets_unreadable`, not `presets_unwritable`, because `save_preset`
    // re-reads the user entries first and a save that cannot see what it would replace must
    // not write over it.
    let valid = a_user_preset("cameo5", "mine", 12);
    assert_eq!(save_preset(&path, valid).unwrap_err().code, "presets_unreadable");
}
