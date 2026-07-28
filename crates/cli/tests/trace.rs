// SPDX-License-Identifier: GPL-3.0-or-later
use std::process::Command;

fn fixture_png(dir: &std::path::Path) -> std::path::PathBuf {
    let img = image::RgbaImage::from_fn(64, 64, |x, y| {
        if (16..48).contains(&x) && (16..48).contains(&y) {
            image::Rgba([0, 0, 0, 255])
        } else {
            image::Rgba([255, 255, 255, 255])
        }
    });
    let p = dir.join("in.png");
    img.save(&p).unwrap();
    p
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cuthulhu"))
}

/// A filled circle, unlike `fixture_png`'s square, has no corners — it is traced entirely with
/// curve (`C`) commands at every detail level, so the curve count is what actually moves when
/// `--detail` moves. A polygon's corners are corners regardless of threshold and cannot observe
/// the control at all.
fn fixture_circle_png(dir: &std::path::Path) -> std::path::PathBuf {
    let (cx, cy, r) = (100.0_f64, 100.0_f64, 70.0_f64);
    let img = image::RgbaImage::from_fn(200, 200, |x, y| {
        let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
        if dx * dx + dy * dy <= r * r {
            image::Rgba([0, 0, 0, 255])
        } else {
            image::Rgba([255, 255, 255, 255])
        }
    });
    let p = dir.join("circle.png");
    img.save(&p).unwrap();
    p
}

#[test]
fn trace_writes_svg_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_png(dir.path());
    let out = dir.path().join("out.svg");
    let status = bin().args([
        "trace", input.to_str().unwrap(), "-o", out.to_str().unwrap(), "--mode", "binary",
    ]).status().unwrap();
    assert!(status.success());
    let svg = std::fs::read_to_string(&out).unwrap();
    assert!(svg.contains("<path"));
}

#[test]
fn trace_rejects_out_of_range_flag() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_png(dir.path());
    let out = dir.path().join("out.svg");
    let output = bin().args([
        "trace", input.to_str().unwrap(), "-o", out.to_str().unwrap(), "--speckle", "17",
    ]).output().unwrap();
    assert!(!output.status.success());
    assert!(!out.exists(), "no file written on failure");
}

#[test]
fn trace_reports_empty_result_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    // 4px dot, speckle 16 — filtered to nothing.
    let img = image::RgbaImage::from_fn(64, 64, |x, y| {
        if (30..34).contains(&x) && (30..34).contains(&y) {
            image::Rgba([0, 0, 0, 255])
        } else {
            image::Rgba([255, 255, 255, 255])
        }
    });
    let input = dir.path().join("dot.png");
    img.save(&input).unwrap();
    let out = dir.path().join("out.svg");
    let output = bin().args([
        "trace", input.to_str().unwrap(), "-o", out.to_str().unwrap(), "--speckle", "16",
    ]).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.to_lowercase().contains("nothing traced"), "stderr: {stderr}");
    assert!(!out.exists());
}

/// `--detail` is stated in the same units as the desktop's Detail slider: higher means more
/// detail. It used to carry vtracer's `length_threshold`, which runs the other way, so the two
/// interfaces printed opposite advice for the same failure. This asserts strictly (`>`, not `>=`)
/// on purpose: `>=` also passes when the control does nothing at all, which is the failure this
/// test exists to catch — a hard-edged square fixture traces with zero curve commands at every
/// detail level, so `>=` on that fixture cannot distinguish "detail works" from "detail is dead".
#[test]
fn detail_reads_high_for_more_detail() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_circle_png(dir.path());
    let curve_commands = |detail: &str| {
        let out = dir.path().join(format!("out-{detail}.svg"));
        let status = bin().args([
            "trace", input.to_str().unwrap(), "-o", out.to_str().unwrap(), "--detail", detail,
        ]).status().unwrap();
        assert!(status.success(), "--detail {detail} failed");
        std::fs::read_to_string(&out).unwrap().matches('C').count()
    };
    assert!(
        curve_commands("10") > curve_commands("3.5"),
        "higher --detail must trace strictly more curve commands"
    );
}

/// The desktop caps what it reads before decoding; the CLI is a second entry point into the same
/// tracer and had no ceiling at all, so a huge file was pulled into memory in full before the
/// decoder ever got to reject it. Uses a sparse file, so the test costs no real disk.
#[test]
fn trace_refuses_a_file_larger_than_the_read_cap() {
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.png");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(300 * 1024 * 1024).unwrap();
    drop(f);
    let out = dir.path().join("out.svg");
    let output = bin()
        .args(["trace", big.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("too large"), "expected a size error, got: {err}");
    assert!(!out.exists(), "no file written on failure");
}
