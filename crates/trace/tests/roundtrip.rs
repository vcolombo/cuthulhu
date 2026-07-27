// SPDX-License-Identifier: GPL-3.0-or-later
use trace::{trace, TraceControls, TraceMode};

fn png_black_square() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(128, 128, |x, y| {
        if (32..96).contains(&x) && (32..96).contains(&y) {
            image::Rgba([0, 0, 0, 255])
        } else {
            image::Rgba([255, 255, 255, 255])
        }
    });
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}

#[test]
fn traced_svg_imports_cleanly_in_both_modes() {
    for mode in [TraceMode::Binary, TraceMode::Color] {
        let controls = TraceControls { mode, ..TraceControls::default() };
        let r = trace(&png_black_square(), &controls).unwrap();
        let imp = fileio::svg_to_paths(r.svg.as_bytes())
            .unwrap_or_else(|e| panic!("{mode:?}: importer rejected trace output: {e:?}"));
        assert_eq!(imp.skipped, Vec::<String>::new(), "{mode:?}: importer skipped elements");
        assert!(!imp.paths.is_empty(), "{mode:?}: importer produced no paths");
        // Importing cleanly is not enough to be cuttable. `cutplan` groups by stroke and skips
        // shapes that have none, so a trace that imports as fill-only geometry plans zero
        // passes and silently cannot be cut at all.
        // Mirrors `plan_passes`, which skips on `stroke.filter(|c| c & 0xFF != 0)`: a present but
        // fully transparent stroke is skipped exactly like an absent one, so asserting only
        // `is_some()` would pass for geometry the planner still refuses to cut.
        for (i, (_, hint)) in imp.paths.iter().enumerate() {
            let stroke = hint.stroke.unwrap_or_else(|| {
                panic!("{mode:?}: imported path {i} has no stroke, so the cut planner would skip it")
            });
            assert!(
                stroke & 0xFF != 0,
                "{mode:?}: imported path {i} has a fully transparent stroke ({stroke:#010x}), which the cut planner skips too",
            );
        }
    }
}
