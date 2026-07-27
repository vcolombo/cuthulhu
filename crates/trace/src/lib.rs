// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Deserialize, Serialize};

pub const MAX_DIM: u32 = 2048;

/// Ceiling on what the decoder may allocate for one image. Applied during decode, so a
/// compressed bomb is rejected before its raw buffer exists — `MAX_DIM` only bounds what
/// vtracer sees, which is far too late. Sized to clear an ordinary large photo (a 6000×4000
/// RGBA frame is ~96 MiB) while refusing the pathological cases.
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;

/// Ceiling on the source *file*, as opposed to `MAX_DECODE_ALLOC`'s ceiling on what decoding it may
/// allocate. The decoder's limit only applies once the bytes are already resident, so without this
/// a huge file exhausts memory before it can be rejected for not being a usable image.
pub const MAX_INPUT_FILE_BYTES: u64 = 256 * 1024 * 1024;

fn too_large() -> TraceError {
    TraceError::Input(format!(
        "file is too large to open: over {} MiB",
        MAX_INPUT_FILE_BYTES / (1024 * 1024)
    ))
}

/// Read a whole stream, refusing input longer than `cap` bytes.
///
/// `cap` is a parameter rather than the constant so the bound can be exercised with a handful of
/// bytes instead of a quarter gigabyte.
fn read_capped<R: std::io::Read>(reader: R, cap: u64) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    // One byte past the ceiling, so landing exactly on it is distinguishable from exceeding it,
    // and so an oversized input costs one extra byte rather than its whole length.
    reader.take(cap + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Ok(None);
    }
    Ok(Some(bytes))
}

/// Read an image file, refusing anything past the ceiling.
///
/// Everything happens through one open handle, rather than `std::fs::metadata` followed by a
/// separate `std::fs::read`. Two *pathname* resolutions describe two moments: the size that passed
/// the check belonged to whatever the path pointed at then, and a file that grew in between was
/// read in full anyway.
///
/// The size check itself is not the problem and is kept — `File::metadata` is `fstat` on the
/// handle, so it cannot describe a different file than the one about to be read. It earns its
/// place by refusing an oversized file for the cost of a syscall, instead of allocating the whole
/// ceiling first only to throw it away.
///
/// Takes the path the caller means to open, and opens exactly that. The desktop authorizes first
/// and passes the already-canonical path it got back, so its check and this open are one
/// resolution; handing an unresolved path here instead would reopen the window
/// `apps/desktop/src/ipc.rs`'s `authorized_path` exists to close.
pub fn read_image(path: &std::path::Path) -> Result<Vec<u8>, TraceError> {
    let file = std::fs::File::open(path)
        .map_err(|e| TraceError::Input(format!("cannot read {}: {e}", path.display())))?;
    // A failed fstat is not fatal: this is a fast path, and `read_capped` below is the real bound.
    if file.metadata().is_ok_and(|m| m.len() > MAX_INPUT_FILE_BYTES) {
        return Err(too_large());
    }
    match read_capped(file, MAX_INPUT_FILE_BYTES)
        .map_err(|e| TraceError::Input(format!("cannot read {}: {e}", path.display())))?
    {
        Some(bytes) => Ok(bytes),
        None => Err(too_large()),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TraceMode { Binary, Color }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceOptions {
    pub mode: TraceMode,
    pub filter_speckle: u8,   // 0–16 px
    pub corner_threshold: u8, // 0–180 degrees
    pub length_threshold: f64, // 3.5–10.0
    pub color_precision: u8,  // 1–8 bits
}
impl Default for TraceOptions {
    fn default() -> Self {
        TraceOptions { mode: TraceMode::Binary, filter_speckle: 4, corner_threshold: 60,
                       length_threshold: 4.0, color_precision: 6 }
    }
}

#[derive(Debug, PartialEq)]
pub enum TraceError { Input(String), InvalidOption(String), Decode(String), Trace(String), EmptyResult }
impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Input's messages are already whole sentences naming the file, so a prefix would read twice.
            TraceError::Input(m) => write!(f, "{m}"),
            TraceError::InvalidOption(m) => write!(f, "invalid option: {m}"),
            TraceError::Decode(m) => write!(f, "could not read image: {m}"),
            TraceError::Trace(m) => write!(f, "trace failed: {m}"),
            TraceError::EmptyResult => write!(f, "empty"),
        }
    }
}
impl std::error::Error for TraceError {}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceResult {
    pub svg: String,
    pub path_count: usize,
    pub width_px: u32,
    pub height_px: u32,
    pub downscaled: bool,
}

pub(crate) fn validate(opts: &TraceOptions) -> Result<(), TraceError> {
    if opts.filter_speckle > 16 {
        return Err(TraceError::InvalidOption("filter_speckle must be 0–16".into()));
    }
    if opts.corner_threshold > 180 {
        return Err(TraceError::InvalidOption("corner_threshold must be 0–180".into()));
    }
    if !(3.5..=10.0).contains(&opts.length_threshold) {
        return Err(TraceError::InvalidOption("length_threshold must be 3.5–10.0".into()));
    }
    if !(1..=8).contains(&opts.color_precision) {
        return Err(TraceError::InvalidOption("color_precision must be 1–8".into()));
    }
    Ok(())
}

pub(crate) fn decode_and_downscale(bytes: &[u8]) -> Result<(image::RgbaImage, bool), TraceError> {
    // Read the header alone and reject oversized images before decoding. `image`'s own
    // `Limits::max_alloc` is not enforced on every decoder path, so the explicit check is what
    // actually holds: without it a compressed bomb allocates its full raw buffer first.
    let (w0, h0) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| TraceError::Decode(e.to_string()))?
        .into_dimensions()
        .map_err(|e| TraceError::Decode(e.to_string()))?;
    // 8 bytes/px, not 4: a 16-bit-per-channel PNG decodes to RGBA16, double the size of the
    // RGBA8 we ultimately convert to. Estimating at 4 would let such an image allocate twice
    // the ceiling before we ever see it.
    // Saturating, not wrapping: the product only stays inside u64 because PNG's own header
    // limits happen to bound it, which is not a property this code should depend on.
    let needed = (w0 as u64).saturating_mul(h0 as u64).saturating_mul(8);
    if needed > MAX_DECODE_ALLOC {
        return Err(TraceError::Decode(format!(
            "image is too large to decode: {w0}×{h0} needs up to {} MiB",
            needed / (1024 * 1024)
        )));
    }

    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| TraceError::Decode(e.to_string()))?;
    reader.limits(limits);
    let img = reader.decode().map_err(|e| TraceError::Decode(e.to_string()))?;
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= MAX_DIM {
        return Ok((img.to_rgba8(), false));
    }
    let resized = img.resize(MAX_DIM, MAX_DIM, image::imageops::FilterType::Triangle);
    Ok((resized.to_rgba8(), true))
}

/// Drop `<path>` elements that carry no geometry, returning the cleaned SVG and the number of
/// paths that actually draw something. vtracer writes one element per line, so this is a line
/// filter; a path is empty when its `d` attribute is `d=""`.
fn strip_empty_paths(svg: &str) -> (String, usize) {
    let mut out = String::with_capacity(svg.len());
    let mut kept = 0usize;
    for line in svg.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("<path") {
            if trimmed.contains("d=\"\"") {
                continue;
            }
            kept += 1;
            out.push_str(&mirror_fill_onto_stroke(line));
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    (out, kept)
}

/// Read the value of `name="..."` from one element's source line.
fn attr_value<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let start = line.find(&key)? + key.len();
    let rest = &line[start..];
    Some(&rest[..rest.find('"')?])
}

/// Copy a path's `fill` onto a `stroke` of the same colour.
///
/// vtracer describes a region by the colour that fills it, which is the right model for an
/// image and the wrong one for a cutter: a blade follows an outline. `cutplan::plan_cut` groups
/// shapes by stroke and counts a strokeless shape into `skipped_no_stroke`, so fill-only trace
/// output plans zero passes — every traced shape is reported as "not cut". Mirroring the colour
/// onto the stroke is what makes a trace reach the machine, and it keeps the fill so the
/// preview still reads as the picture the user traced.
///
/// Done here rather than in `import_svg` or `plan_cut` on purpose. "Filled but unstroked means
/// do not cut" is a deliberate distinction downstream — it is how an imported SVG says which of
/// its shapes are cut lines — so the colour has to be promoted at the point where it is known
/// to describe cuttable geometry, not by weakening that rule for every document.
fn mirror_fill_onto_stroke(line: &str) -> String {
    // Leave a path that already carries a stroke alone: re-adding one would emit the attribute
    // twice and make the SVG invalid.
    if line.contains("stroke=\"") {
        return line.to_string();
    }
    match attr_value(line, "fill") {
        Some(fill) => line.replacen(
            &format!("fill=\"{fill}\""),
            &format!("fill=\"{fill}\" stroke=\"{fill}\""),
            1,
        ),
        None => line.to_string(),
    }
}

/// Decode an image, apply the same ceiling and downscale as tracing, and re-encode it as PNG.
///
/// The desktop thumbnail goes through this instead of returning the file's raw bytes. Handing
/// back raw bytes makes the command a general "read any file and give me its contents" primitive
/// — a non-image path succeeds just as readily as an image one. Round-tripping through the
/// decoder means only real image data can ever come back, and the payload is bounded by
/// `MAX_DIM` rather than by the source file's size.
pub fn preview_png(image_bytes: &[u8]) -> Result<Vec<u8>, TraceError> {
    let (rgba, _) = decode_and_downscale(image_bytes)?;
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| TraceError::Trace(e.to_string()))?;
    Ok(out.into_inner())
}

/// Composite onto white so transparency reads as background. Binary mode only — see `trace`.
///
/// vtracer's binary path is `to_binary_image(|x| x.r < 128)`: it thresholds on the red channel
/// and never looks at alpha, and exporters write transparent regions as `(0,0,0,0)`. A
/// transparent background is therefore indistinguishable from black artwork there — it merges
/// into the shape and the whole canvas traces as one filled rectangle. Because every emitted
/// path is stroked, that rectangle is a cut line, so an invisible image would put a rectangle
/// through the material.
fn flatten_onto_white(img: &mut image::RgbaImage) {
    for px in img.pixels_mut() {
        let a = px[3] as u32;
        if a == 255 {
            continue;
        }
        for c in 0..3 {
            // Standard source-over composite against opaque white, rounded rather than truncated
            // so a fully opaque channel survives the round trip unchanged.
            px[c] = ((px[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        }
        px[3] = 255;
    }
}

/// Append one fully transparent row so vtracer keys transparency out.
///
/// Colour mode only removes transparent regions when `should_key_image` says the frame has enough
/// of them: it counts transparent pixels on five sampled scanlines and needs 20% of two widths.
/// A small hole misses that bar, so its raw RGB survives as ordinary artwork — a `(0,0,0,0)`
/// island traces to a black shape and, being stroked, cuts. One transparent row lands on the
/// last sampled scanline and contributes a full width of transparent pixels, which clears the
/// threshold for any image, and keying then applies to *every* transparent pixel including
/// interior ones. The row itself is keyed away, so it contributes no geometry; only the emitted
/// `height` has to be put back.
fn pad_with_transparent_row(img: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = (img.width(), img.height());
    let mut padded = image::RgbaImage::from_pixel(w, h + 1, image::Rgba([0, 0, 0, 0]));
    for (x, y, px) in img.enumerate_pixels() {
        padded.put_pixel(x, y, *px);
    }
    padded
}

pub fn trace(image_bytes: &[u8], opts: &TraceOptions) -> Result<TraceResult, TraceError> {
    validate(opts)?;
    let (mut rgba, downscaled) = decode_and_downscale(image_bytes)?;
    // Nothing visible means nothing to cut. This has to be decided before flattening, which
    // erases the distinction by making every transparent pixel opaque white — and in colour mode
    // vtracer then clusters that white into a path of its own, which `strip_empty_paths` strokes.
    // The result would be a cut rectangle traced from an image that shows nothing at all.
    if rgba.pixels().all(|p| p[3] == 0) {
        return Err(TraceError::EmptyResult);
    }
    // Only binary mode needs this. Colour mode already keys transparency out — vtracer replaces
    // every fully transparent pixel with a colour absent from the image and has visioncortex drop
    // that colour, so the background produces no cluster. Flattening first would hide the alpha
    // it looks for, and the manufactured white would come back as a stroked path the cut planner
    // reports as a pass. Binary mode has no such handling and needs the composite.
    if opts.mode == TraceMode::Binary {
        flatten_onto_white(&mut rgba);
    }
    let (width, height) = (rgba.width(), rgba.height());

    // Colour mode leans on vtracer's keying, which is threshold-gated, so force it whenever the
    // image has any transparency at all rather than only when enough of it lands on the sampled
    // scanlines. Binary mode has already had its transparency composited away.
    let padded = opts.mode == TraceMode::Color && rgba.pixels().any(|p| p[3] == 0);
    let fed = if padded { pad_with_transparent_row(&rgba) } else { rgba };
    let (fed_width, fed_height) = (fed.width(), fed.height());

    let mut img = vtracer::ColorImage::new();
    img.pixels = fed.into_raw();
    img.width = fed_width as usize;
    img.height = fed_height as usize;

    let config = vtracer::Config {
        color_mode: match opts.mode {
            TraceMode::Binary => vtracer::ColorMode::Binary,
            TraceMode::Color => vtracer::ColorMode::Color,
        },
        hierarchical: vtracer::Hierarchical::Stacked,
        filter_speckle: opts.filter_speckle as usize,
        color_precision: opts.color_precision as i32,
        corner_threshold: opts.corner_threshold as i32,
        length_threshold: opts.length_threshold,
        ..vtracer::Config::default()
    };

    // visioncortex (vtracer's clustering layer) panics on some valid high-frequency images —
    // a 512×512 one-pixel checkerboard overflows in clusters.rs. An unwind here would abort the
    // CLI and take down the Tauri command, so contain it and report it as a normal trace failure.
    // The default panic hook still prints the raw panic to stderr before we swallow the unwind;
    // that noise is cosmetic. Silencing it means swapping the process-global hook, which would
    // also hide genuine panics on other threads for the duration — not worth the trade.
    let converted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vtracer::convert(img, config)
    }))
    .map_err(|_| TraceError::Trace("tracer failed on this image; try a lower detail setting".into()))?;
    let svg_file = converted.map_err(TraceError::Trace)?;
    // vtracer emits a `<path d="">` element for every cluster it could not turn into geometry,
    // and on high-frequency input nearly all of them can be empty — a 256×256 checkerboard at
    // speckle 0 yields 32768 paths that draw nothing. Counting those would report a wildly
    // inflated `path_count` and call an empty trace a success. They are invisible downstream
    // too: usvg discards them before `fileio::svg_to_paths` builds its node list, so they never
    // appear in the `skipped` warnings the Insert path relies on to report loss.
    let (mut svg, path_count) = strip_empty_paths(&svg_file.to_string());
    if path_count == 0 {
        return Err(TraceError::EmptyResult);
    }
    if padded {
        // Undo the extra row in the viewport so the SVG describes the source image. Only the
        // `<svg>` element carries a height, and geometry is in absolute coordinates, so trimming
        // the viewport moves nothing.
        svg = svg.replacen(
            &format!("height=\"{fed_height}\""),
            &format!("height=\"{height}\""),
            1,
        );
    }
    Ok(TraceResult {
        path_count,
        svg,
        width_px: width,
        height_px: height,
        downscaled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, RgbaImage};
    use std::io::Cursor;

    pub(crate) fn png_bytes(img: &RgbaImage) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    /// w×h white image with a centered black square of side `sq`.
    pub(crate) fn black_square(w: u32, h: u32, sq: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            let inside = x >= (w - sq) / 2 && x < (w + sq) / 2 && y >= (h - sq) / 2 && y < (h + sq) / 2;
            if inside { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([255, 255, 255, 255]) }
        })
    }

    #[test]
    fn validate_rejects_out_of_range() {
        let bad = TraceOptions { filter_speckle: 17, ..TraceOptions::default() };
        assert!(matches!(validate(&bad), Err(TraceError::InvalidOption(m)) if m.contains("filter_speckle")));
        let bad = TraceOptions { color_precision: 0, ..TraceOptions::default() };
        assert!(matches!(validate(&bad), Err(TraceError::InvalidOption(m)) if m.contains("color_precision")));
        let bad = TraceOptions { length_threshold: 11.0, ..TraceOptions::default() };
        assert!(matches!(validate(&bad), Err(TraceError::InvalidOption(m)) if m.contains("length_threshold")));
        assert!(validate(&TraceOptions::default()).is_ok());
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(matches!(decode_and_downscale(b"not an image"), Err(TraceError::Decode(_))));
    }

    #[test]
    fn decode_passes_small_images_through() {
        let bytes = png_bytes(&black_square(100, 50, 20));
        let (img, downscaled) = decode_and_downscale(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), (100, 50));
        assert!(!downscaled);
    }

    #[test]
    fn decode_downscales_above_cap_preserving_aspect() {
        let bytes = png_bytes(&black_square(3000, 1000, 200));
        let (img, downscaled) = decode_and_downscale(&bytes).unwrap();
        assert!(downscaled);
        assert_eq!(img.width(), 2048);
        // image::resize fits within the box preserving aspect: 1000 * 2048/3000 ≈ 682.
        assert!((680..=684).contains(&img.height()));
    }

    #[test]
    fn empty_result_displays_empty_sentinel() {
        assert_eq!(TraceError::EmptyResult.to_string(), "empty");
    }

    /// 100×100 image split into 4 solid 50×50 quadrants: red, green, blue, white.
    fn quadrants() -> RgbaImage {
        RgbaImage::from_fn(100, 100, |x, y| match (x < 50, y < 50) {
            (true, true) => image::Rgba([255, 0, 0, 255]),
            (false, true) => image::Rgba([0, 128, 0, 255]),
            (true, false) => image::Rgba([0, 0, 255, 255]),
            (false, false) => image::Rgba([255, 255, 255, 255]),
        })
    }

    #[test]
    fn binary_trace_of_black_square_yields_paths() {
        let bytes = png_bytes(&black_square(128, 128, 64));
        let r = trace(&bytes, &TraceOptions::default()).unwrap();
        assert!(r.path_count >= 1);
        assert!(r.svg.contains("<path"));
        assert_eq!((r.width_px, r.height_px), (128, 128));
        assert!(!r.downscaled);
    }

    #[test]
    fn color_trace_yields_multiple_fills() {
        let opts = TraceOptions { mode: TraceMode::Color, filter_speckle: 0, ..TraceOptions::default() };
        let r = trace(&png_bytes(&quadrants()), &opts).unwrap();
        assert!(r.path_count >= 2);
        // At least two distinct fill colors among the emitted paths.
        let fills: std::collections::HashSet<&str> = r.svg.match_indices("fill=\"")
            .map(|(i, _)| { let rest = &r.svg[i + 6..]; &rest[..rest.find('"').unwrap()] })
            .collect();
        assert!(fills.len() >= 2, "fills: {fills:?}");
    }

    #[test]
    fn trace_is_deterministic() {
        let bytes = png_bytes(&black_square(128, 128, 64));
        let a = trace(&bytes, &TraceOptions::default()).unwrap();
        let b = trace(&bytes, &TraceOptions::default()).unwrap();
        assert_eq!(a.svg, b.svg);
    }

    #[test]
    fn speckle_filter_can_empty_the_result() {
        // Whole image is smaller than a 16px speckle after the square shrinks to 4px.
        let bytes = png_bytes(&black_square(64, 64, 4));
        let opts = TraceOptions { filter_speckle: 16, ..TraceOptions::default() };
        assert!(matches!(trace(&bytes, &opts), Err(TraceError::EmptyResult)));
    }

    #[test]
    fn trace_rejects_invalid_options_before_decoding() {
        let bad = TraceOptions { filter_speckle: 17, ..TraceOptions::default() };
        assert!(matches!(trace(b"irrelevant", &bad), Err(TraceError::InvalidOption(_))));
    }

    /// A 256×256 one-pixel checkerboard at speckle 0 makes vtracer emit tens of thousands of
    /// geometry-free paths. Reporting those as a successful trace is a lie the importer cannot
    /// detect, because usvg drops them before `skipped` is ever computed.
    #[test]
    fn geometry_free_paths_do_not_count_as_a_trace() {
        let img = RgbaImage::from_fn(256, 256, |x, y| {
            if (x + y) % 2 == 0 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([255, 255, 255, 255]) }
        });
        let opts = TraceOptions { mode: TraceMode::Binary, filter_speckle: 0, ..TraceOptions::default() };
        match trace(&png_bytes(&img), &opts) {
            Err(TraceError::EmptyResult) => {}
            Ok(r) => {
                assert!(!r.svg.contains("d=\"\""), "empty paths survived into the output");
                assert_eq!(r.path_count, r.svg.matches("<path").count(), "path_count disagrees with real paths");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// vtracer describes regions with `fill` alone, but a cutter follows outlines: `cutplan`
    /// groups shapes by *stroke* and counts anything with no stroke as not cut. Fill-only trace
    /// output therefore plans zero passes, so nothing traced can be cut at all. Every emitted
    /// path has to carry a stroke matching its fill for the trace to reach the machine.
    #[test]
    fn traced_paths_are_stroked_so_they_can_be_cut() {
        fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
            let key = format!("{name}=\"");
            let start = line.find(&key)? + key.len();
            let rest = &line[start..];
            Some(&rest[..rest.find('"')?])
        }

        for mode in [TraceMode::Binary, TraceMode::Color] {
            let opts = TraceOptions { mode, filter_speckle: 0, ..TraceOptions::default() };
            let r = trace(&png_bytes(&quadrants()), &opts).unwrap();
            let paths: Vec<&str> =
                r.svg.lines().filter(|l| l.trim_start().starts_with("<path")).collect();
            assert!(!paths.is_empty(), "{mode:?}: no paths emitted");
            for p in paths {
                let fill = attr(p, "fill")
                    .unwrap_or_else(|| panic!("{mode:?}: path carries no fill: {p}"));
                assert_eq!(attr(p, "stroke"), Some(fill), "{mode:?}: stroke must match fill: {p}");
            }
        }
    }

    /// vtracer reads raw channel values and ignores alpha, so a fully transparent pixel
    /// (0,0,0,0) reaches it as solid black and an invisible image traces to a filled canvas.
    /// Since every traced path is now stroked, that phantom shape is cut geometry: loading a
    /// transparent PNG would put a rectangle through the material.
    /// Both modes, because the failure differs by mode and testing only one hides the other:
    /// binary reads transparent black as artwork, while color clusters the flattened background
    /// into a white path. Either way the result is a stroked, cuttable canvas rectangle.
    #[test]
    fn a_fully_transparent_image_traces_to_nothing() {
        let img = RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 0]));
        for mode in [TraceMode::Binary, TraceMode::Color] {
            let opts = TraceOptions { mode, ..TraceOptions::default() };
            assert!(
                matches!(trace(&png_bytes(&img), &opts), Err(TraceError::EmptyResult)),
                "{mode:?}: a transparent image must trace to nothing, not to a cuttable rectangle",
            );
        }
    }

    /// Colour mode keys transparency out on its own: vtracer swaps every fully transparent pixel
    /// for an unused colour and tells visioncortex to drop it, so a transparent background yields
    /// no cluster at all. Flattening it to white first destroys that — the background arrives
    /// opaque, is clustered like any other colour, and comes back as a stroked path, which the
    /// cut planner then reports as a pass. The artwork alone must survive.
    #[test]
    fn a_transparent_background_is_not_a_path_in_color_mode() {
        let img = RgbaImage::from_fn(128, 128, |x, y| {
            if (32..96).contains(&x) && (32..96).contains(&y) {
                image::Rgba([0, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        let opts = TraceOptions { mode: TraceMode::Color, ..TraceOptions::default() };
        let r = trace(&png_bytes(&img), &opts).expect("the opaque square should trace");
        assert_eq!(r.path_count, 1, "expected only the square, got {} paths:\n{}", r.path_count, r.svg);
        assert!(
            !r.svg.to_uppercase().contains("#FFFFFF"),
            "the transparent background came back as a white path:\n{}",
            r.svg,
        );
    }

    /// vtracer only keys transparency once enough of it shows up on the five scanlines
    /// `should_key_image` samples, so a small interior hole falls under the threshold and its raw
    /// RGB is traced as artwork — a `(0,0,0,0)` island becomes a stroked black shape that cuts,
    /// indistinguishable from an opaque one. Keying has to happen for any transparency at all,
    /// and the padding used to force it must not show up in the result.
    #[test]
    fn a_transparent_island_is_keyed_out_below_the_keying_threshold() {
        let img = RgbaImage::from_fn(100, 100, |x, y| {
            if (45..55).contains(&x) && (45..55).contains(&y) {
                image::Rgba([0, 0, 0, 0])
            } else {
                image::Rgba([255, 255, 255, 255])
            }
        });
        let opts = TraceOptions { mode: TraceMode::Color, ..TraceOptions::default() };
        let r = trace(&png_bytes(&img), &opts).expect("the opaque background should trace");
        assert_eq!(r.path_count, 1, "the invisible island became geometry:\n{}", r.svg);
        assert!(!r.svg.contains("#000000"), "the island was traced as black:\n{}", r.svg);
        assert_eq!((r.width_px, r.height_px), (100, 100), "reported size must be the source size");
        assert!(
            r.svg.contains("height=\"100\""),
            "padding leaked into the emitted SVG:\n{}",
            r.svg.lines().take(4).collect::<Vec<_>>().join("\n"),
        );
    }

    /// A transparent image is empty whatever its RGB happens to be: exporters vary between
    /// (0,0,0,0) and (255,255,255,0), and neither is artwork.
    #[test]
    fn transparency_is_empty_regardless_of_its_hidden_color() {
        for hidden in [[0, 0, 0, 0], [255, 255, 255, 0], [17, 200, 90, 0]] {
            let img = RgbaImage::from_pixel(32, 32, image::Rgba(hidden));
            let opts = TraceOptions { mode: TraceMode::Color, ..TraceOptions::default() };
            assert!(
                matches!(trace(&png_bytes(&img), &opts), Err(TraceError::EmptyResult)),
                "hidden colour {hidden:?} must still count as empty",
            );
        }
    }

    /// The ordinary case for a logo: opaque artwork on a transparent background, which exporters
    /// write as `(0,0,0,0)` — transparent *black*. Read without alpha that is indistinguishable
    /// from the artwork itself, so the background merges into the shape and the trace becomes a
    /// filled canvas. Transparent has to behave exactly like the white background it stands in for.
    #[test]
    fn a_transparent_background_traces_like_a_white_one() {
        let shape = |bg: image::Rgba<u8>| {
            RgbaImage::from_fn(128, 128, |x, y| {
                if (32..96).contains(&x) && (32..96).contains(&y) {
                    image::Rgba([0, 0, 0, 255])
                } else {
                    bg
                }
            })
        };
        let opts = TraceOptions { mode: TraceMode::Binary, ..TraceOptions::default() };
        let transparent = trace(&png_bytes(&shape(image::Rgba([0, 0, 0, 0]))), &opts).unwrap();
        let white = trace(&png_bytes(&shape(image::Rgba([255, 255, 255, 255]))), &opts).unwrap();
        assert_eq!(
            transparent.svg, white.svg,
            "a transparent background must trace like a white one, not merge into the artwork",
        );
    }

    #[test]
    fn strip_empty_paths_counts_only_real_geometry() {
        let svg = "<svg>\n<path d=\"M0 0 L1 1 Z\" fill=\"#000\"/>\n<path d=\"\" fill=\"#111\"/>\n</svg>";
        let (out, n) = strip_empty_paths(svg);
        assert_eq!(n, 1);
        assert!(!out.contains("d=\"\""));
        assert!(out.contains("M0 0 L1 1 Z"));
    }

    /// The thumbnail path must not become a way to read non-image files: anything that is not
    /// decodable image data has to fail rather than come back as content.
    #[test]
    fn preview_png_refuses_non_image_data() {
        assert!(matches!(preview_png(b"root:x:0:0:root:/root:/bin/sh\n"), Err(TraceError::Decode(_))));
        let ok = preview_png(&png_bytes(&black_square(64, 64, 16))).unwrap();
        assert_eq!(&ok[1..4], b"PNG");
    }

    /// A 20000×20000 PNG is 75 KB compressed but 1.6 GB as RGBA. Without a decode-time
    /// allocation limit this exhausts memory before `MAX_DIM` downscaling ever runs.
    #[test]
    fn decode_rejects_a_decompression_bomb() {
        let bytes = include_bytes!("../tests/fixtures/bomb-20000x20000.png");
        assert!(matches!(decode_and_downscale(bytes), Err(TraceError::Decode(_))));
    }

    /// visioncortex overflows on this input; the panic must surface as a typed error
    /// rather than unwinding out of the CLI or the Tauri command.
    #[test]
    fn trace_contains_tracer_panic_on_high_frequency_image() {
        let bytes = include_bytes!("../tests/fixtures/checker-512.png");
        let opts = TraceOptions { filter_speckle: 0, ..TraceOptions::default() };
        match trace(bytes, &opts) {
            Err(TraceError::Trace(_)) | Err(TraceError::EmptyResult) => {}
            other => panic!("expected a typed error, got {other:?}"),
        }
    }

    /// The ceiling has to come from the read itself. A separate size check describes whatever the
    /// pathname pointed at when it ran, so a file that grows between the check and the read is
    /// read in full despite having just passed the limit — the cap is advisory rather than a
    /// bound. Deliberately exercised through a plain reader, with no file and no metadata call,
    /// because that is the property under test.
    #[test]
    fn read_capped_refuses_a_stream_longer_than_the_cap() {
        use std::io::Read as _;
        let over = std::io::repeat(b'x').take(9);
        assert!(read_capped(over, 8).unwrap().is_none(), "9 bytes must be refused against a cap of 8");
    }

    /// Covers the glue the helper tests cannot: opening the path, threading the real ceiling
    /// through, and handing back the bytes. Both trace entry points read through here, so a
    /// mistake in this wiring breaks every trace.
    #[test]
    fn read_image_returns_the_contents_of_a_small_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.bin");
        std::fs::write(&path, b"not an image, but bytes are bytes").unwrap();
        assert_eq!(read_image(&path).unwrap(), b"not an image, but bytes are bytes");
    }

    /// Exercises the real `MAX_INPUT_FILE_BYTES`, which the `read_capped` tests deliberately do
    /// not. The file is extended rather than written, so no quarter gigabyte ever moves through
    /// this process. What it costs on disk is the filesystem's business — usually nothing, since
    /// the range can be left unallocated, but `set_len` promises the size and never the storage.
    #[test]
    fn read_image_refuses_a_file_past_the_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.bin");
        std::fs::File::create(&path).unwrap().set_len(MAX_INPUT_FILE_BYTES + 1).unwrap();
        assert!(matches!(read_image(&path), Err(TraceError::Input(m)) if m.contains("too large")));
    }

    /// A path that does not exist is an input failure, not a decode failure: nothing was ever
    /// handed to the decoder. The distinction is what `code()` will make visible to the desktop.
    #[test]
    fn read_image_reports_a_missing_file_as_input() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(read_image(&dir.path().join("nope.png")), Err(TraceError::Input(_))));
    }

    #[test]
    fn read_capped_accepts_a_stream_exactly_at_the_cap() {
        use std::io::Read as _;
        let exact = std::io::repeat(b'x').take(8);
        assert_eq!(read_capped(exact, 8).unwrap().map(|b| b.len()), Some(8));
    }

    #[test]
    fn read_image_reports_a_missing_path_with_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.bin");
        assert!(matches!(read_image(&path), Err(TraceError::Input(m)) if m.contains("absent.bin")));
    }
}
