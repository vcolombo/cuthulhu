// SPDX-License-Identifier: GPL-3.0-or-later
use cli::pipeline::{build_bytes, cutpass_from_color_pass, pass_stream_bytes, plan_from_svg, Device};
use driver_core::Settings;

#[test]
fn hpgl_dry_run_matches_documented_stream() {
    let svg = std::fs::read("tests/fixtures/square.svg").unwrap();
    let bytes = build_bytes(&svg, Device::Puma, &Settings::default()).unwrap();
    // 20 user-units = 20px → 20*25.4/96 mm → ×1016/25.4 = 20*1016/96 ≈ 212 units.
    let s = String::from_utf8(bytes).unwrap();
    assert!(s.starts_with("IN;PU0,0;PD212,0;") || s.starts_with("IN;PU0,0;PD211,0;"), "{s}");
    assert!(s.ends_with("PU;"));
}

#[test]
fn multi_pass_dry_run_parks_between_passes_like_the_device_manager() {
    // Two stroke colors → two passes. The dry-run stream must mirror
    // DeviceManager framing: session_begin, pass, pass_park, pass, session_end.
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg">
        <path d="M0 0 L20 0" stroke="#ff0000"/>
        <path d="M0 10 L20 10" stroke="#00ff00"/>
    </svg>"##;
    let passes = plan_from_svg(svg, &[], None).unwrap();
    assert_eq!(passes.len(), 2);

    let d = Device::Puma.driver();
    let settings = Settings::default();
    let streams: Vec<String> = passes
        .iter()
        .enumerate()
        .map(|(i, pass)| {
            let cutpass = cutpass_from_color_pass(pass, &settings);
            String::from_utf8(pass_stream_bytes(d.as_ref(), &cutpass.job, i, passes.len()).unwrap()).unwrap()
        })
        .collect();

    // HPGL: session_begin = "IN;", pass_park and session_end are both "PU;".
    assert!(streams[0].starts_with("IN;"), "{}", streams[0]);
    assert!(streams[0].ends_with("PU;"), "pass 0 must park: {}", streams[0]);
    assert!(!streams[1].starts_with("IN;"), "session_begin only once: {}", streams[1]);
    assert!(streams[1].ends_with("PU;"), "last pass must close the session: {}", streams[1]);

    // Silhouette distinguishes park (empty) from session_end (SO0/FN0), so it can
    // catch a stream that wrongly closes the session between passes.
    let cameo = Device::Cameo5.driver();
    let contains = |bytes: &[u8], needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    let c0 = {
        let cutpass = cutpass_from_color_pass(&passes[0], &settings);
        pass_stream_bytes(cameo.as_ref(), &cutpass.job, 0, passes.len()).unwrap()
    };
    let c1 = {
        let cutpass = cutpass_from_color_pass(&passes[1], &settings);
        pass_stream_bytes(cameo.as_ref(), &cutpass.job, 1, passes.len()).unwrap()
    };
    assert!(!contains(&c0, b"FN0"), "pass 0 must not close the session");
    assert!(contains(&c1, b"FN0"), "last pass must close the session");
}
