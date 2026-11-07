pub struct PwgPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

const HEADER_SIZE: usize = 64;

fn read_u32_be(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

type PwgHeader = (u32, u32, u32, u32, u32, u32, u32, u32);

fn page_header(data: &[u8], offset: usize) -> Option<PwgHeader> {
    if data.len() < offset + HEADER_SIZE {
        return None;
    }
    let res_x = read_u32_be(data, offset)?;
    let res_y = read_u32_be(data, offset + 4)?;
    let width = read_u32_be(data, offset + 8)?;
    let height = read_u32_be(data, offset + 12)?;
    let bits_per_pixel = read_u32_be(data, offset + 16)?;
    let color_space = read_u32_be(data, offset + 20)?;
    let channels = read_u32_be(data, offset + 24)?;
    let bits_per_channel = read_u32_be(data, offset + 28)?;
    Some((
        res_x,
        res_y,
        width,
        height,
        bits_per_pixel,
        color_space,
        channels,
        bits_per_channel,
    ))
}

fn looks_like_pwg_header(
    res_x: u32,
    res_y: u32,
    width: u32,
    height: u32,
    bits_per_pixel: u32,
    channels: u32,
) -> bool {
    (25..=4800).contains(&res_x)
        && (25..=4800).contains(&res_y)
        && width > 0
        && width <= 20_000
        && height > 0
        && height <= 200_000
        && (bits_per_pixel == 8 || bits_per_pixel == 1)
        && (channels == 1 || channels == 3 || channels == 4)
}

fn inflate_bytes(raw: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut zlib_out = Vec::new();
    if flate2::read::ZlibDecoder::new(raw)
        .read_to_end(&mut zlib_out)
        .is_ok()
        && !zlib_out.is_empty()
    {
        return Ok(zlib_out);
    }
    let mut decoder = flate2::read::DeflateDecoder::new(raw);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| format!("Không thể giải nén dữ liệu PWG (flate): {}", e))?;
    Ok(out)
}

pub fn decode_pwg_document(data: &[u8], compression: Option<&str>) -> Result<Vec<PwgPage>, String> {
    let flate_requested = compression
        .map(|c| c.eq_ignore_ascii_case("flate") || c.eq_ignore_ascii_case("gzip"))
        .unwrap_or(false);

    let mut pages = Vec::new();
    let mut offset = 0usize;

    while offset + HEADER_SIZE <= data.len() {
        let Some((
            res_x,
            res_y,
            width,
            height,
            bits_per_pixel,
            color_space,
            channels,
            bits_per_channel,
        )) = page_header(data, offset)
        else {
            break;
        };

        if !looks_like_pwg_header(res_x, res_y, width, height, bits_per_pixel, channels) {
            break;
        }

        let image_data_size = read_u32_be(data, offset + 36).unwrap_or(0) as usize;
        let expected_raw = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(channels as usize)
            .saturating_mul((bits_per_channel.max(1)) as usize / 8.max(bits_per_channel as usize));

        let raw_start = offset + HEADER_SIZE;
        let raw_end = (raw_start + image_data_size).min(data.len());
        if raw_end <= raw_start {
            return Err("PWG: ImageDataSize rỗng hoặc vượt quá dữ liệu nhận được".to_string());
        }

        let compressed = &data[raw_start..raw_end];
        let payload = if flate_requested {
            inflate_bytes(compressed)?
        } else {
            compressed.to_vec()
        };

        let is_one_bit = bits_per_pixel == 1 || bits_per_channel == 1;
        let (row_stride, expected_len) = if is_one_bit {
            let rb = (width as usize).div_ceil(8);
            (rb, rb * height as usize)
        } else {
            let bytes_per_pixel = channels as usize * (bits_per_channel as usize / 8).max(1);
            let rb = width as usize * bytes_per_pixel;
            (rb, rb * height as usize)
        };
        if payload.len() < expected_len {
            return Err(format!(
                "PWG: dữ liệu trang ngắn ({} bytes, cần {})",
                payload.len(),
                expected_len
            ));
        }

        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);

        for y in 0..height as usize {
            let row = &payload[y * row_stride..(y + 1) * row_stride];
            if is_one_bit {
                for x in 0..width as usize {
                    let byte = row.get(x / 8).copied().unwrap_or(0xFF);
                    let bit_set = (byte >> (7 - (x % 8))) & 1 == 1;
                    let value = if bit_set { 0u8 } else { 255u8 };
                    rgba.extend_from_slice(&[value, value, value, 255]);
                }
            } else if channels == 1 {
                for &gray in row.iter().take(width as usize) {
                    rgba.extend_from_slice(&[gray, gray, gray, 255]);
                }
            } else {
                for pixel in row.as_chunks::<3>().0.iter().take(width as usize) {
                    rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
            }
        }

        pages.push(PwgPage {
            width,
            height,
            rgba,
        });

        offset = raw_start + image_data_size;
        if flate_requested {
            break;
        }
        let _ = expected_raw;
        if color_space == 0 {
            break;
        }
    }

    if pages.is_empty() {
        return Err("PWG: không đọc được trang raster nào từ dữ liệu".to_string());
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn make_header(
        res_x: u32,
        width: u32,
        height: u32,
        bits: u32,
        cs: u32,
        ch: u32,
        bpc: u32,
        data_size: u32,
    ) -> Vec<u8> {
        let mut header = Vec::with_capacity(64);
        for value in [
            res_x, res_x, width, height, bits, cs, ch, bpc, 0, data_size, 0, 0, 0, 0, 0, 0,
        ] {
            header.extend_from_slice(&value.to_be_bytes());
        }
        header
    }

    #[test]
    fn test_decode_uncompressed_gray_page() {
        let width = 8u32;
        let height = 2u32;
        let mut data = make_header(600, width, height, 8, 19, 1, 8, width * height);
        data.extend_from_slice(&[
            0u8, 255u8, 0u8, 255u8, 0u8, 255u8, 0u8, 255u8, 128u8, 128u8, 128u8, 128u8, 128u8,
            128u8, 128u8, 128u8,
        ]);

        let pages = decode_pwg_document(&data, None).expect("decode OK");
        assert_eq!(pages.len(), 1);
        let page = &pages[0];
        assert_eq!(page.width, 8);
        assert_eq!(page.height, 2);
        assert_eq!(page.rgba.len(), (8 * 2 * 4) as usize);
        assert_eq!(&page.rgba[0..4], &[0, 0, 0, 255]);
        assert_eq!(&page.rgba[4..8], &[255, 255, 255, 255]);
    }

    #[test]
    fn test_decode_one_bit_page_black_bit_is_ink() {
        let width = 8u32;
        let height = 1u32;
        let mut data = make_header(600, width, height, 1, 6, 1, 1, 1);
        data.push(0b1010_1010);

        let pages = decode_pwg_document(&data, None).expect("decode OK");
        let page = &pages[0];
        assert_eq!(page.rgba[0], 0);
        assert_eq!(page.rgba[4], 255);
        assert_eq!(page.rgba[8], 0);
    }

    #[test]
    fn test_decode_rejects_garbage() {
        let garbage = vec![0u8; 100];
        assert!(decode_pwg_document(&garbage, None).is_err());
        assert!(decode_pwg_document(&[], None).is_err());
    }

    #[test]
    fn test_decode_flate_page() {
        use flate2::Compression;
        use flate2::write::ZlibEncoder;
        use std::io::Write;

        let width = 4u32;
        let height = 1u32;
        let payload: Vec<u8> = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&payload).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut data = make_header(600, width, height, 8, 18, 3, 8, compressed.len() as u32);
        data.extend_from_slice(&compressed);

        let pages = decode_pwg_document(&data, Some("flate")).expect("decode OK");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].rgba[0], 10);
        assert_eq!(pages[0].rgba[3], 255);
    }

    #[test]
    fn test_decode_multi_page_uncompressed() {
        let width = 2u32;
        let height = 1u32;
        let mut data = make_header(600, width, height, 8, 19, 1, 8, 2);
        data.extend_from_slice(&[0u8, 255u8]);
        data.extend_from_slice(&make_header(600, width, height, 8, 19, 1, 8, 2));
        data.extend_from_slice(&[255u8, 0u8]);

        let pages = decode_pwg_document(&data, None).expect("decode OK");
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].rgba[0], 0);
        assert_eq!(pages[1].rgba[0], 255);
    }
}
