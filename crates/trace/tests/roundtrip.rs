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
        // Importing cleanly is enough now. `cutplan` cuts a shape because its `CutLineType`
        // says `Cut` — which is what import defaults to — and keys its pass on the fill when
        // there is no stroke, so fill-only trace output plans a pass per traced colour. The
        // planning half of that contract is pinned in `cutplan`
        // (`a_fill_only_shape_that_is_cut_plans_into_a_pass_keyed_on_its_fill`); what this
        // test owns is that the trace really does arrive as fill-only geometry.
        for (i, (_, hint)) in imp.paths.iter().enumerate() {
            assert_eq!(hint.stroke, None, "{mode:?}: imported path {i} carries an invented stroke");
            let fill = hint.fill.unwrap_or_else(|| {
                panic!("{mode:?}: imported path {i} has no fill, so nothing keys its pass")
            });
            assert!(fill & 0xFF != 0,
                "{mode:?}: imported path {i} has a fully transparent fill ({fill:#010x})");
        }
    }
}
