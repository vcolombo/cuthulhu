// SPDX-License-Identifier: GPL-3.0-or-later
use trace::{trace, TraceMode, TraceOptions};

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
        let opts = TraceOptions { mode, ..TraceOptions::default() };
        let r = trace(&png_black_square(), &opts).unwrap();
        let imp = fileio::svg_to_paths(r.svg.as_bytes())
            .unwrap_or_else(|e| panic!("{mode:?}: importer rejected trace output: {e:?}"));
        assert_eq!(imp.skipped, Vec::<String>::new(), "{mode:?}: importer skipped elements");
        assert!(!imp.paths.is_empty(), "{mode:?}: importer produced no paths");
        // Importing cleanly is not enough to be cuttable. `cutplan` groups by stroke and skips
        // shapes that have none, so a trace that imports as fill-only geometry plans zero
        // passes and silently cannot be cut at all.
        for (i, (_, hint)) in imp.paths.iter().enumerate() {
            assert!(
                hint.stroke.is_some(),
                "{mode:?}: imported path {i} has no stroke, so the cut planner would skip it",
            );
        }
    }
}
