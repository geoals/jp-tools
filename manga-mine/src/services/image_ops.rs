//! Image decoding, cropping, and compression.
//!
//! All functions are synchronous and CPU-bound — call them via
//! `tokio::task::spawn_blocking` from handlers.
//!
//! Photos from phones carry an EXIF orientation tag; browsers render it
//! applied, so user-drawn crop coordinates are in *oriented* space. Every
//! decode here applies the orientation first so pixel space matches what the
//! user saw.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader};

#[derive(Debug, thiserror::Error)]
pub enum ImageOpsError {
    #[error("image processing failed: {0}")]
    Failed(String),
}

/// Fractional crop rectangle (0.0–1.0, relative to the oriented image).
#[derive(Debug, Clone, Copy)]
pub struct CropRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl CropRect {
    /// Convert to pixel coordinates, clamped to the image bounds.
    /// Returns `None` when the resulting region is empty.
    pub fn to_pixels(self, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
        let clamp01 = |v: f64| v.clamp(0.0, 1.0);
        let x0 = clamp01(self.x);
        let y0 = clamp01(self.y);
        let x1 = clamp01(self.x + self.w);
        let y1 = clamp01(self.y + self.h);

        let px = (x0 * width as f64).floor() as u32;
        let py = (y0 * height as f64).floor() as u32;
        let pw = ((x1 - x0) * width as f64).ceil() as u32;
        let ph = ((y1 - y0) * height as f64).ceil() as u32;

        let pw = pw.min(width.saturating_sub(px));
        let ph = ph.min(height.saturating_sub(py));

        if pw == 0 || ph == 0 {
            return None;
        }
        Some((px, py, pw, ph))
    }
}

/// Decode an image and apply its EXIF orientation.
fn decode_oriented(bytes: &[u8]) -> Result<DynamicImage, ImageOpsError> {
    let mut decoder = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ImageOpsError::Failed(format!("unrecognized image format: {e}")))?
        .into_decoder()
        .map_err(|e| ImageOpsError::Failed(format!("failed to decode image: {e}")))?;

    let orientation = decoder
        .orientation()
        .unwrap_or(Orientation::NoTransforms);

    let mut img = DynamicImage::from_decoder(decoder)
        .map_err(|e| ImageOpsError::Failed(format!("failed to decode image: {e}")))?;
    img.apply_orientation(orientation);
    Ok(img)
}

fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>, ImageOpsError> {
    let mut out = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut out, quality);
    img.to_rgb8()
        .write_with_encoder(encoder)
        .map_err(|e| ImageOpsError::Failed(format!("failed to encode jpeg: {e}")))?;
    Ok(out)
}

/// Crop the user-drawn region out of a photo and encode it as a high-quality
/// JPEG for OCR.
pub fn crop_for_ocr(bytes: &[u8], rect: CropRect) -> Result<Vec<u8>, ImageOpsError> {
    let img = decode_oriented(bytes)?;
    let (px, py, pw, ph) = rect
        .to_pixels(img.width(), img.height())
        .ok_or_else(|| ImageOpsError::Failed("crop region is empty".into()))?;
    let crop = img.crop_imm(px, py, pw, ph);
    encode_jpeg(&crop, 92)
}

/// Compress a whole photo for the Anki card: downscale so the longest side is
/// at most `max_dim` pixels and re-encode as JPEG.
pub fn compress_photo(bytes: &[u8], max_dim: u32, quality: u8) -> Result<Vec<u8>, ImageOpsError> {
    let img = decode_oriented(bytes)?;
    let img = if img.width().max(img.height()) > max_dim {
        img.resize(max_dim, max_dim, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    encode_jpeg(&img, quality)
}

/// Downscale a photo to a small thumbnail JPEG for the queue view.
pub fn thumbnail(bytes: &[u8], max_dim: u32) -> Result<Vec<u8>, ImageOpsError> {
    let img = decode_oriented(bytes)?;
    let thumb = img.thumbnail(max_dim, max_dim);
    encode_jpeg(&thumb, 75)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn test_jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut img = RgbImage::new(width, height);
        // Left half red, right half blue — lets crop tests check content
        for (x, _y, p) in img.enumerate_pixels_mut() {
            *p = if x < width / 2 {
                Rgb([200, 30, 30])
            } else {
                Rgb([30, 30, 200])
            };
        }
        let mut out = Vec::new();
        let encoder = JpegEncoder::new_with_quality(&mut out, 90);
        img.write_with_encoder(encoder).unwrap();
        out
    }

    #[test]
    fn crop_rect_to_pixels_clamps_out_of_bounds() {
        let rect = CropRect { x: -0.5, y: 0.5, w: 2.0, h: 1.0 };
        let (px, py, pw, ph) = rect.to_pixels(100, 200).unwrap();
        assert_eq!((px, py), (0, 100));
        assert_eq!((pw, ph), (100, 100));
    }

    #[test]
    fn crop_rect_empty_region_is_none() {
        let rect = CropRect { x: 0.5, y: 0.5, w: 0.0, h: 0.5 };
        assert!(rect.to_pixels(100, 100).is_none());
        let rect = CropRect { x: 1.5, y: 0.0, w: 0.5, h: 0.5 };
        assert!(rect.to_pixels(100, 100).is_none());
    }

    #[test]
    fn crop_for_ocr_extracts_region() {
        let jpeg = test_jpeg(200, 100);
        let rect = CropRect { x: 0.0, y: 0.0, w: 0.4, h: 1.0 };
        let out = crop_for_ocr(&jpeg, rect).unwrap();

        let img = image::load_from_memory(&out).unwrap();
        assert_eq!(img.width(), 80);
        assert_eq!(img.height(), 100);
        // Left 40% of the test image is red
        let px = img.to_rgb8().get_pixel(10, 50).0;
        assert!(px[0] > 150 && px[2] < 100, "expected red region, got {px:?}");
    }

    #[test]
    fn compress_photo_downscales_large_images() {
        let jpeg = test_jpeg(2000, 1000);
        let out = compress_photo(&jpeg, 1600, 80).unwrap();
        let img = image::load_from_memory(&out).unwrap();
        assert_eq!(img.width(), 1600);
        assert_eq!(img.height(), 800);
    }

    #[test]
    fn compress_photo_keeps_small_images() {
        let jpeg = test_jpeg(400, 300);
        let out = compress_photo(&jpeg, 1600, 80).unwrap();
        let img = image::load_from_memory(&out).unwrap();
        assert_eq!((img.width(), img.height()), (400, 300));
    }
}
