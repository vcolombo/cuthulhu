// SPDX-License-Identifier: GPL-3.0-or-later
use cli::pipeline::{build_bytes, Device};
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
