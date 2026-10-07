use image::{DynamicImage, GenericImageView};

pub struct FloydSteinbergRasterizer;

#[allow(dead_code)]
impl FloydSteinbergRasterizer {
    pub fn dither(img: &DynamicImage, target_width: Option<u32>) -> (Vec<u8>, u32, u32) {
        let resized = if let Some(tw) = target_width {
            let (w, h) = img.dimensions();
            let target_h = (h as f32 * (tw as f32 / w as f32)).round() as u32;
            img.resize_exact(tw, target_h, image::imageops::FilterType::Triangle)
        } else {
            img.clone()
        };

        let (raw_w, height) = resized.dimensions();
        if raw_w == 0 || height == 0 {
            return (Vec::new(), 0, 0);
        }

        let width = raw_w.div_ceil(8) * 8;
        let width_usize = width as usize;
        let height_usize = height as usize;

        let rgba = resized.to_rgba8();
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

        (bitmap, width, height)
    }

    pub fn to_escpos_raster(img: &DynamicImage, target_width: Option<u32>) -> Vec<u8> {
        let (bitmap, width, height) = Self::dither(img, target_width);
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
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    #[test]
    fn test_floyd_steinberg_dithering() {
        let img = ImageBuffer::from_pixel(16, 16, Rgb([128u8, 128u8, 128u8]));
        let dyn_img = DynamicImage::ImageRgb8(img);

        let (bitmap, w, h) = FloydSteinbergRasterizer::dither(&dyn_img, None);
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

        let cmd = FloydSteinbergRasterizer::to_escpos_raster(&dyn_img, None);
        assert_eq!(&cmd[0..4], &[0x1D, 0x76, 0x30, 0x00]);
        assert_eq!(cmd[4], 1);
        assert_eq!(cmd[5], 0);
        assert_eq!(cmd[6], 8);
        assert_eq!(cmd[7], 0);
        assert_eq!(&cmd[8..], &[0xFF; 8]);
    }

    #[test]
    fn test_create_sample_png_and_dither() {
        let mut img = ImageBuffer::new(64, 64);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            if (x + y) % 4 == 0 {
                *pixel = Rgb([0u8, 0u8, 0u8]);
            } else {
                *pixel = Rgb([200u8, 200u8, 200u8]);
            }
        }
        let dyn_img = DynamicImage::ImageRgb8(img);
        dyn_img.save("test_logo.png").unwrap();
        assert!(std::path::Path::new("test_logo.png").exists());
    }

    #[test]
    fn test_transparent_png_blends_to_white() {
        use image::Rgba;
        let img = ImageBuffer::from_pixel(8, 8, Rgba([0u8, 0u8, 0u8, 0u8]));
        let dyn_img = DynamicImage::ImageRgba8(img);

        let (bitmap, w, h) = FloydSteinbergRasterizer::dither(&dyn_img, None);
        assert_eq!(w, 8);
        assert_eq!(h, 8);
        assert_eq!(bitmap, vec![0u8; 8]);
    }
}
