// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Deserialize, Serialize};

pub const MAX_DIM: u32 = 2048;

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
    let img = image::load_from_memory(bytes).map_err(|e| TraceError::Decode(e.to_string()))?;
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= MAX_DIM {
        return Ok((img.to_rgba8(), false));
    }
    let resized = img.resize(MAX_DIM, MAX_DIM, image::imageops::FilterType::Triangle);
    Ok((resized.to_rgba8(), true))
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
}
