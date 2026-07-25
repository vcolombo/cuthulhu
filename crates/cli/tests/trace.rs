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
