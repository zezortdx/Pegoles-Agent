//! Screenshots from the host: base64 PNG → checked, cropped, resized RGB.
//! Bounded before decoding (bytes and pixel dimensions).

use base64::Engine;
use image::{imageops::FilterType, ImageFormat, ImageReader, Limits, RgbImage};

use crate::protocol::{BadRequest, ImagePrep, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE};

pub fn decode(b64: &str, prep: &ImagePrep) -> Result<RgbImage, BadRequest> {
    if b64.len() > MAX_IMAGE_BYTES * 4 / 3 + 8 {
        return Err(BadRequest("image too large".into()));
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| BadRequest("image is not valid base64".into()))?;
    if image::guess_format(&raw).ok() != Some(ImageFormat::Png) {
        return Err(BadRequest("images must be PNG".into()));
    }
    let mut reader = ImageReader::with_format(std::io::Cursor::new(raw), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(u64::from(MAX_IMAGE_SIDE) * u64::from(MAX_IMAGE_SIDE) * 8);
    reader.limits(limits);
    let mut image = reader
        .decode()
        .map_err(|_| BadRequest("image dimensions out of range or unreadable".into()))?
        .to_rgb8();
    if let Some([x0, y0, x1, y1]) = prep.crop {
        let (w, h) = image.dimensions();
        if !(x0 < x1 && x1 <= w && y0 < y1 && y1 <= h) {
            return Err(BadRequest("crop outside the image".into()));
        }
        image = image::imageops::crop_imm(&image, x0, y0, x1 - x0, y1 - y0).to_image();
    }
    if let Some((rw, rh)) = prep.resize {
        if (rw, rh) != image.dimensions() {
            // Lanczos, as the MLX worker (PIL LANCZOS), so both backends
            // see the same pixels.
            image = image::imageops::resize(&image, rw, rh, FilterType::Lanczos3);
        }
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> String {
        let img = RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 7])
        });
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(out)
    }

    #[test]
    fn crops_and_resizes_exactly() {
        let prep = ImagePrep {
            crop: Some([10, 20, 110, 120]),
            resize: Some((64, 32)),
        };
        let img = decode(&png(200, 150), &prep).unwrap();
        assert_eq!(img.dimensions(), (64, 32));
    }

    #[test]
    fn refuses_non_png_bad_crops_and_huge_images() {
        let jpeg_magic =
            base64::engine::general_purpose::STANDARD.encode([0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0]);
        assert!(decode(&jpeg_magic, &ImagePrep::default()).is_err());
        assert!(decode("%%%", &ImagePrep::default()).is_err());
        assert!(decode(
            &png(20, 20),
            &ImagePrep {
                crop: Some([0, 0, 30, 10]),
                resize: None
            }
        )
        .is_err());
        assert!(decode(&png(MAX_IMAGE_SIDE + 1, 2), &ImagePrep::default()).is_err());
    }
}
