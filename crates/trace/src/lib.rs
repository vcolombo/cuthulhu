// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Deserialize, Serialize};

pub const MAX_DIM: u32 = 2048;

/// Ceiling on what the decoder may allocate for one image. Applied during decode, so a
/// compressed bomb is rejected before its raw buffer exists — `MAX_DIM` only bounds what
/// vtracer sees, which is far too late. Sized to clear an ordinary large photo (a 6000×4000
/// RGBA frame is ~96 MiB) while refusing the pathological cases.
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;

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
pub enum TraceError { InvalidOption(String), Decode(String), Trace(String), EmptyResult }
impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
    let needed = w0 as u64 * h0 as u64 * 8;
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

pub fn trace(image_bytes: &[u8], opts: &TraceOptions) -> Result<TraceResult, TraceError> {
    validate(opts)?;
    let (rgba, downscaled) = decode_and_downscale(image_bytes)?;
    let (width, height) = (rgba.width(), rgba.height());

    let mut img = vtracer::ColorImage::new();
    img.pixels = rgba.into_raw();
    img.width = width as usize;
    img.height = height as usize;

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
    if svg_file.paths.is_empty() {
        return Err(TraceError::EmptyResult);
    }
    Ok(TraceResult {
        path_count: svg_file.paths.len(),
        svg: svg_file.to_string(),
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
}
