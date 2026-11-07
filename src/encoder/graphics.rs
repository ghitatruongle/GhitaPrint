use image::{DynamicImage, GenericImageView};

pub struct FloydSteinbergRasterizer;

pub const DEFAULT_HEAD_WIDTH_DOTS: u32 = 576;
pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;
pub const MAX_RASTER_HEIGHT_DOTS: u32 = 65_535;

impl FloydSteinbergRasterizer {
    pub fn dither(
        img: &DynamicImage,
        target_width: Option<u32>,
    ) -> Result<(Vec<u8>, u32, u32), String> {
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return Err("Ảnh rỗng hoặc không hợp lệ".to_string());
        }
        if (w as u64) * (h as u64) > MAX_IMAGE_PIXELS {
            return Err(format!(
                "Ảnh quá lớn ({} điểm ảnh) — tối đa {} triệu điểm ảnh",
                (w as u64) * (h as u64),
                MAX_IMAGE_PIXELS / 1_000_000
            ));
        }

        let max_width = target_width
            .unwrap_or(DEFAULT_HEAD_WIDTH_DOTS)
            .clamp(1, 2048);
        let eff_w = w.min(max_width);

        let resized_owned;
        let source: &DynamicImage = if eff_w != w {
            let target_h = (((h as f64) * (eff_w as f64 / w as f64)).round() as u32).max(1);
            resized_owned =
                img.resize_exact(eff_w, target_h, image::imageops::FilterType::Triangle);
            &resized_owned
        } else {
            img
        };

        let (raw_w, height) = source.dimensions();
        if raw_w == 0 || height == 0 {
            return Err("Ảnh sau khi co giãn rỗng".to_string());
        }
        if height > MAX_RASTER_HEIGHT_DOTS {
            return Err(format!(
                "Ảnh cao {} dots vượt giới hạn máy in {} dots — hãy cắt ảnh ngắn lại hoặc giảm --image-width",
                height, MAX_RASTER_HEIGHT_DOTS
            ));
        }

        let width = raw_w.div_ceil(8) * 8;
        let width_usize = width as usize;
        let height_usize = height as usize;

        let rgba = source.to_rgba8();
        let raw_bytes = rgba.as_raw();
        let mut pixels = vec![255.0f32; width_usize * height_usize];

        for y in 0..height_usize {
            let src_row_start = y * raw_w as usize * 4;
            let dst_row_start = y * width_usize;
            let src_row = &raw_bytes[src_row_start..src_row_start + (raw_w as usize * 4)];
            let (chunks, _) = src_row.as_chunks::<4>();

            for (x, chunk) in chunks.iter().enumerate() {
                let a = chunk[3];
                let blended = if a == 255 {
                    0.299 * (chunk[0] as f32)
                        + 0.587 * (chunk[1] as f32)
                        + 0.114 * (chunk[2] as f32)
                } else if a == 0 {
                    255.0
                } else {
                    let alpha = a as f32 / 255.0;
                    let luma = 0.299 * (chunk[0] as f32)
                        + 0.587 * (chunk[1] as f32)
                        + 0.114 * (chunk[2] as f32);
                    luma * alpha + 255.0 * (1.0 - alpha)
                };
                pixels[dst_row_start + x] = blended;
            }
        }

        let width_bytes = width_usize / 8;
        let mut bitmap = vec![0u8; width_bytes * height_usize];

        for y in 0..height_usize {
            let row_offset = y * width_usize;
            let next_row_offset = (y + 1) * width_usize;
            let has_next_row = (y + 1) < height_usize;
            let row_byte_offset = y * width_bytes;

            let mut current_byte = 0u8;

            for x in 0..width_usize {
                let idx = row_offset + x;
                let old_val = pixels[idx];
                let is_black = old_val < 128.0;
                let new_val = if is_black { 0.0 } else { 255.0 };
                let err = old_val - new_val;

                current_byte = (current_byte << 1) | if is_black { 1 } else { 0 };
                if (x & 7) == 7 {
                    bitmap[row_byte_offset + (x >> 3)] = current_byte;
                    current_byte = 0;
                }

                if x + 1 < width_usize {
                    pixels[idx + 1] += err * (7.0 / 16.0);
                }
                if has_next_row {
                    if x > 0 {
                        pixels[next_row_offset + (x - 1)] += err * (3.0 / 16.0);
                    }
                    pixels[next_row_offset + x] += err * (5.0 / 16.0);
                    if x + 1 < width_usize {
                        pixels[next_row_offset + (x + 1)] += err * (1.0 / 16.0);
                    }
                }
            }
        }

        Ok((bitmap, width, height))
    }

    pub fn to_escpos_raster(
        img: &DynamicImage,
        target_width: Option<u32>,
    ) -> Result<Vec<u8>, String> {
        let (bitmap, width, height) = Self::dither(img, target_width)?;
        let width_bytes = (width / 8) as u16;
        let height_dots = height as u16;

        let mut cmd = Vec::with_capacity(8 + bitmap.len());
        cmd.extend_from_slice(&[
            0x1D,
            0x76,
            0x30,
            0x00,
            (width_bytes & 0xFF) as u8,
            ((width_bytes >> 8) & 0xFF) as u8,
            (height_dots & 0xFF) as u8,
            ((height_dots >> 8) & 0xFF) as u8,
        ]);
        cmd.extend_from_slice(&bitmap);
        Ok(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb, Rgba};

    #[test]
    fn test_floyd_steinberg_dithering() {
        let img = ImageBuffer::from_pixel(16, 16, Rgb([128u8, 128u8, 128u8]));
        let dyn_img = DynamicImage::ImageRgb8(img);

        let (bitmap, w, h) = FloydSteinbergRasterizer::dither(&dyn_img, None).expect("dither OK");
        assert_eq!(w, 16);
        assert_eq!(h, 16);
        assert_eq!(bitmap.len(), 32);

        let has_black = bitmap.iter().any(|&b| b > 0);
        let has_white = bitmap.iter().any(|&b| b < 255);
        assert!(has_black);
        assert!(has_white);
    }

    #[test]
    fn test_escpos_raster_command() {
        let img = ImageBuffer::from_pixel(8, 8, Rgb([0u8, 0u8, 0u8]));
        let dyn_img = DynamicImage::ImageRgb8(img);

        let cmd = FloydSteinbergRasterizer::to_escpos_raster(&dyn_img, None).expect("raster OK");
        assert_eq!(&cmd[0..4], &[0x1D, 0x76, 0x30, 0x00]);
        assert_eq!(cmd[4], 1);
        assert_eq!(cmd[5], 0);
        assert_eq!(cmd[6], 8);
        assert_eq!(cmd[7], 0);
        assert_eq!(&cmd[8..], &[0xFF; 8]);
    }

    #[test]
    fn test_transparent_png_blends_to_white() {
        let img = ImageBuffer::from_pixel(8, 8, Rgba([0u8, 0u8, 0u8, 0u8]));
        let dyn_img = DynamicImage::ImageRgba8(img);

        let (bitmap, w, h) = FloydSteinbergRasterizer::dither(&dyn_img, None).expect("dither OK");
        assert_eq!(w, 8);
        assert_eq!(h, 8);
        assert_eq!(bitmap, vec![0u8; 8]);
    }

    #[test]
    fn test_escpos_raster_clamps_extreme_width() {
        let img = ImageBuffer::from_pixel(8, 8, Rgb([0u8, 0u8, 0u8]));
        let dyn_img = DynamicImage::ImageRgb8(img);

        let cmd =
            FloydSteinbergRasterizer::to_escpos_raster(&dyn_img, Some(999_999)).expect("raster OK");
        let width_bytes = u16::from_le_bytes([cmd[4], cmd[5]]);
        assert!(width_bytes <= 256);
        assert!(cmd.len() >= 8);

        let zero_cmd = FloydSteinbergRasterizer::to_escpos_raster(&dyn_img, Some(0)).expect("OK");
        let zero_width_bytes = u16::from_le_bytes([zero_cmd[4], zero_cmd[5]]);
        assert!(zero_width_bytes >= 1);
    }

    #[test]
    fn test_default_width_clamps_wide_image() {
        let img = ImageBuffer::from_pixel(2000, 10, Rgb([128u8, 128u8, 128u8]));
        let dyn_img = DynamicImage::ImageRgb8(img);
        let (bitmap, w, h) = FloydSteinbergRasterizer::dither(&dyn_img, None).expect("dither OK");
        assert_eq!(w, 576);
        assert_eq!(h, 3);
        assert_eq!(bitmap.len(), (576 / 8) * 3);
    }

    #[test]
    fn test_small_image_not_upscaled_by_default() {
        let img = ImageBuffer::from_pixel(64, 64, Rgb([128u8, 128u8, 128u8]));
        let dyn_img = DynamicImage::ImageRgb8(img);
        let (_, w, h) = FloydSteinbergRasterizer::dither(&dyn_img, None).expect("dither OK");
        assert_eq!(w, 64);
        assert_eq!(h, 64);
    }

    #[test]
    fn test_tall_image_rejected() {
        let img = ImageBuffer::from_pixel(10, 70_000, Rgb([0u8, 0u8, 0u8]));
        let dyn_img = DynamicImage::ImageRgb8(img);
        let err = FloydSteinbergRasterizer::dither(&dyn_img, None).unwrap_err();
        assert!(err.contains("vượt giới hạn"));
    }

    #[test]
    fn test_empty_image_rejected() {
        let img = ImageBuffer::from_pixel(0, 0, Rgb([0u8, 0u8, 0u8]));
        let dyn_img = DynamicImage::ImageRgb8(img);
        let err = FloydSteinbergRasterizer::dither(&dyn_img, None).unwrap_err();
        assert!(err.contains("rỗng"));
    }

    #[test]
    fn test_huge_pixel_count_rejected() {
        let w: u32 = 12_000;
        let h: u32 = 10_000;
        let img = ImageBuffer::from_fn(w, h, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 128u8]));
        let dyn_img = DynamicImage::ImageRgb8(img);
        let err = FloydSteinbergRasterizer::dither(&dyn_img, None).unwrap_err();
        assert!(err.contains("quá lớn"));
    }

    #[test]
    fn test_degenerate_aspect_ratio_not_empty() {
        let img = ImageBuffer::from_pixel(10_000, 1, Rgb([0u8, 0u8, 0u8]));
        let dyn_img = DynamicImage::ImageRgb8(img);
        let cmd = FloydSteinbergRasterizer::to_escpos_raster(&dyn_img, Some(2048)).expect("OK");
        let height = u16::from_le_bytes([cmd[6], cmd[7]]);
        assert!(height >= 1);
    }
}
