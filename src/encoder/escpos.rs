pub struct EscPosBuilder {
    buffer: Vec<u8>,
    line_width: usize,
}

#[allow(dead_code)]
impl EscPosBuilder {
    pub const DEFAULT_LINE_WIDTH: usize = 32;

    pub fn new() -> Self {
        Self::with_capacity(512)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let mut builder = Self {
            buffer: Vec::with_capacity(capacity),
            line_width: Self::DEFAULT_LINE_WIDTH,
        };
        builder.init();
        builder
    }

    pub fn init(&mut self) -> &mut Self {
        self.buffer.extend_from_slice(&[0x1B, 0x40]);
        self
    }

    pub fn align(&mut self, alignment: u8) -> &mut Self {
        self.buffer.extend_from_slice(&[0x1B, 0x61, alignment]);
        self
    }

    pub fn align_left(&mut self) -> &mut Self {
        self.align(0)
    }

    pub fn align_center(&mut self) -> &mut Self {
        self.align(1)
    }

    pub fn align_right(&mut self) -> &mut Self {
        self.align(2)
    }

    pub fn bold(&mut self, enable: bool) -> &mut Self {
        self.buffer
            .extend_from_slice(&[0x1B, 0x45, if enable { 1 } else { 0 }]);
        self
    }

    pub fn underline(&mut self, mode: u8) -> &mut Self {
        self.buffer.extend_from_slice(&[0x1B, 0x2D, mode]);
        self
    }

    pub fn font_size(&mut self, width_mult: u8, height_mult: u8) -> &mut Self {
        let w = (width_mult.clamp(1, 8) - 1) << 4;
        let h = height_mult.clamp(1, 8) - 1;
        self.buffer.extend_from_slice(&[0x1D, 0x21, w | h]);
        self
    }

    pub fn text(&mut self, text: &str) -> &mut Self {
        for ch in text.chars() {
            if ch == '\n' || (!ch.is_control() && ch != '\u{7f}') {
                let mut byte_buf = [0u8; 4];
                self.buffer
                    .extend_from_slice(ch.encode_utf8(&mut byte_buf).as_bytes());
            }
        }
        self
    }

    pub fn text_line(&mut self, text: &str) -> &mut Self {
        self.text(text);
        self.buffer.push(b'\n');
        self
    }

    pub fn set_line_width(&mut self, columns: usize) -> &mut Self {
        self.line_width = columns.clamp(8, 128);
        self
    }

    pub fn divider(&mut self, style: &str) -> &mut Self {
        let line: String = std::iter::repeat_n(
            if style == "solid" || style == "double" {
                '='
            } else {
                '-'
            },
            self.line_width,
        )
        .collect();
        self.text_line(&line)
    }

    pub fn codepage(&mut self, table: u8) -> &mut Self {
        self.buffer.extend_from_slice(&[0x1B, 0x74, table]);
        self
    }

    pub fn feed(&mut self, lines: u8) -> &mut Self {
        self.buffer.extend_from_slice(&[0x1B, 0x64, lines]);
        self
    }

    pub fn cut(&mut self, partial: bool) -> &mut Self {
        self.feed(3);
        let mode = if partial { 0x01 } else { 0x00 };
        self.buffer.extend_from_slice(&[0x1D, 0x56, mode]);
        self
    }

    pub fn barcode_128(&mut self, content: &str, height: u8) -> &mut Self {
        let data_all = content.as_bytes();
        let mut usable = data_all.len().min(253);
        while usable > 0 && !content.is_char_boundary(usable) {
            usable -= 1;
        }
        let data = &data_all[..usable];
        let len = (usable + 2) as u8;

        self.buffer.reserve(12 + data.len());
        self.buffer
            .extend_from_slice(&[0x1D, 0x68, height.clamp(20, 200)]);
        self.buffer.extend_from_slice(&[0x1D, 0x77, 2]);
        self.buffer.extend_from_slice(&[0x1D, 0x48, 2]);
        self.buffer
            .extend_from_slice(&[0x1D, 0x6B, 73, len, b'{', b'B']);
        self.buffer.extend_from_slice(data);
        self
    }

    pub fn qrcode(&mut self, content: &str, size: u8) -> &mut Self {
        let data_all = content.as_bytes();
        const QR_PAYLOAD_LIMIT: usize = 2900;
        let mut usable = data_all.len().min(QR_PAYLOAD_LIMIT);
        while usable > 0 && !content.is_char_boundary(usable) {
            usable -= 1;
        }
        let data = &data_all[..usable];
        let len = (usable + 3) as u16;
        let p_l = (len & 0xFF) as u8;
        let p_h = ((len >> 8) & 0xFF) as u8;

        self.buffer.reserve(32 + data.len());
        self.buffer
            .extend_from_slice(&[0x1D, 0x28, 0x6B, 0x04, 0x00, 0x31, 0x41, 0x32, 0x00]);

        let qr_size = size.clamp(1, 16);
        self.buffer
            .extend_from_slice(&[0x1D, 0x28, 0x6B, 0x03, 0x00, 0x31, 0x43, qr_size]);

        self.buffer
            .extend_from_slice(&[0x1D, 0x28, 0x6B, 0x03, 0x00, 0x31, 0x45, 0x31]);

        self.buffer
            .extend_from_slice(&[0x1D, 0x28, 0x6B, p_l, p_h, 0x31, 0x50, 0x30]);
        self.buffer.extend_from_slice(data);

        self.buffer
            .extend_from_slice(&[0x1D, 0x28, 0x6B, 0x03, 0x00, 0x31, 0x51, 0x30]);
        self
    }

    pub fn text_vi(&mut self, text: &str, mode: &str) -> &mut Self {
        let (codepage, bytes) = encode_text_bytes(text, mode);
        if let Some(cp) = codepage {
            self.codepage(cp);
        }
        self.buffer.extend_from_slice(&bytes);
        self
    }

    pub fn build_test_receipt() -> Vec<u8> {
        let mut b = Self::new();
        b.align_center()
            .font_size(2, 2)
            .bold(true)
            .text_line("GHITAPRINT")
            .font_size(1, 1)
            .bold(false)
            .text_line("Universal Driverless Print Engine")
            .text_line("Hotline: 1900-GHITA | www.ghita.ai")
            .divider("double")
            .align_center()
            .bold(true)
            .text_line("PHIEU KIEM TRA MAY IN (TEST)")
            .bold(false)
            .align_left()
            .text_line("Thoi gian : 2026-10-07 12:00")
            .text_line("Giao thuc : ESC/POS Standard")
            .text_line("Driverless: Direct Hardware Bytecode")
            .divider("solid")
            .text_line("Mat hang              SL   Thanh tien")
            .divider("dashed")
            .text_line("1. Ca phe sua da       1      35.000d")
            .text_line("2. Tra dao cam sa      2      90.000d")
            .divider("solid")
            .align_right()
            .bold(true)
            .font_size(1, 2)
            .text_line("TONG TIEN: 125.000d")
            .font_size(1, 1)
            .bold(false)
            .align_center()
            .feed(1)
            .text_line("Quet ma QR de thanh toan:")
            .qrcode("https://ghita.ai/pay/demo-receipt-101", 6)
            .feed(1)
            .text_line("Cam on Quy khach & Hen gap lai!")
            .cut(true);

        b.into_bytes()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }
}

impl Default for EscPosBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn sanitize_to_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\n' || (!ch.is_control() && ch != '\u{7f}') {
            let mut byte_buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut byte_buf).as_bytes());
        }
    }
    out
}

pub(crate) fn strip_diacritics(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match vn_base_char(ch) {
            Some(base) => out.push(base),
            None => {
                let cp = ch as u32;
                if (0x0300..=0x036F).contains(&cp) || cp == 0x031B {
                    continue;
                }
                out.push(ch);
            }
        }
    }
    out
}

fn vn_base_char(ch: char) -> Option<char> {
    let base = match ch {
        'à' | 'á' | 'ả' | 'ã' | 'ạ' | 'ă' | 'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' | 'â' | 'ầ' | 'ấ' | 'ẩ'
        | 'ẫ' | 'ậ' => 'a',
        'đ' => 'd',
        'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' | 'ê' | 'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' => 'e',
        'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' => 'i',
        'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' | 'ô' | 'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' | 'ơ' | 'ờ' | 'ớ' | 'ở'
        | 'ỡ' | 'ợ' => 'o',
        'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' | 'ư' | 'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' => 'u',
        'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' => 'y',
        'À' | 'Á' | 'Ả' | 'Ã' | 'Ạ' | 'Ă' | 'Ằ' | 'Ắ' | 'Ẳ' | 'Ẵ' | 'Ặ' | 'Â' | 'Ầ' | 'Ấ' | 'Ẩ'
        | 'Ẫ' | 'Ậ' => 'A',
        'Đ' => 'D',
        'È' | 'É' | 'Ẻ' | 'Ẽ' | 'Ẹ' | 'Ê' | 'Ề' | 'Ế' | 'Ể' | 'Ễ' | 'Ệ' => 'E',
        'Ì' | 'Í' | 'Ỉ' | 'Ĩ' | 'Ị' => 'I',
        'Ò' | 'Ó' | 'Ỏ' | 'Õ' | 'Ọ' | 'Ô' | 'Ồ' | 'Ố' | 'Ổ' | 'Ỗ' | 'Ộ' | 'Ơ' | 'Ờ' | 'Ớ' | 'Ở'
        | 'Ỡ' | 'Ợ' => 'O',
        'Ù' | 'Ú' | 'Ủ' | 'Ũ' | 'Ụ' | 'Ư' | 'Ừ' | 'Ứ' | 'Ử' | 'Ữ' | 'Ự' => 'U',
        'Ỳ' | 'Ý' | 'Ỷ' | 'Ỹ' | 'Ỵ' => 'Y',
        _ => return None,
    };
    Some(base)
}

const ESC_T_CODEPAGE_VIETNAMESE_TCVN3: u8 = 34;

fn tcvn3_byte(ch: char) -> Option<u8> {
    let byte = match ch {
        'à' => 0xB5,
        'á' => 0xB8,
        'ả' => 0xB6,
        'ã' => 0xB7,
        'ạ' => 0xB9,
        'ă' => 0xB0,
        'â' => 0xB4,
        'đ' => 0xD0,
        'Đ' => 0x90,
        _ => return None,
    };
    Some(byte)
}

fn encode_text_bytes(text: &str, mode: &str) -> (Option<u8>, Vec<u8>) {
    match mode {
        "raw" => (None, sanitize_to_bytes(text)),
        "keep" => {
            if text.is_ascii() {
                return (None, text.as_bytes().to_vec());
            }
            let mut bytes = Vec::with_capacity(text.len());
            let mut all_mapped = true;
            for ch in text.chars() {
                if ch.is_ascii() {
                    bytes.push(ch as u8);
                } else if let Some(b) = tcvn3_byte(ch) {
                    bytes.push(b);
                } else {
                    all_mapped = false;
                    break;
                }
            }
            if all_mapped {
                (Some(ESC_T_CODEPAGE_VIETNAMESE_TCVN3), bytes)
            } else {
                (None, strip_diacritics(text).into_bytes())
            }
        }
        _ => (None, strip_diacritics(text).into_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escpos_init_and_text() {
        let mut b = EscPosBuilder::new();
        b.text("Xin chao");
        let bytes = b.into_bytes();
        assert_eq!(&bytes[0..2], &[0x1B, 0x40]);
        assert!(bytes.ends_with(b"Xin chao"));
    }

    #[test]
    fn test_escpos_align_and_bold() {
        let mut b = EscPosBuilder::new();
        b.align_center().bold(true).text("Bold Center");
        let bytes = b.into_bytes();
        assert!(bytes.windows(3).any(|w| w == [0x1B, 0x61, 1]));
        assert!(bytes.windows(3).any(|w| w == [0x1B, 0x45, 1]));
    }

    #[test]
    fn test_escpos_qr_code() {
        let mut b = EscPosBuilder::new();
        b.qrcode("https://ghita.ai", 6);
        let bytes = b.into_bytes();
        assert!(bytes.windows(3).any(|w| w == [0x1D, 0x28, 0x6B]));
        assert!(
            bytes
                .windows(b"https://ghita.ai".len())
                .any(|w| w == b"https://ghita.ai")
        );
    }

    #[test]
    fn test_escpos_cut() {
        let mut b = EscPosBuilder::new();
        b.cut(true);
        let bytes = b.into_bytes();
        assert!(bytes.windows(3).any(|w| w == [0x1D, 0x56, 1]));
    }

    #[test]
    fn test_escpos_barcode_128() {
        let mut b = EscPosBuilder::new();
        b.barcode_128("ABC-12345", 60);
        let bytes = b.into_bytes();
        assert!(bytes.windows(3).any(|w| w == [0x1D, 0x68, 60]));
        assert!(bytes.windows(3).any(|w| w == [0x1D, 0x6B, 73]));
        assert!(bytes.windows(b"ABC-12345".len()).any(|w| w == b"ABC-12345"));
    }

    #[test]
    fn test_escpos_barcode_128_length_overflow() {
        let long_data = "A".repeat(500);
        let mut b = EscPosBuilder::new();
        b.barcode_128(&long_data, 60);
        let bytes = b.into_bytes();
        let pos = bytes
            .windows(4)
            .position(|w| w == [0x1D, 0x6B, 73, 255])
            .expect("len byte phải là 255 (= 253 + 2)");
        let emitted = &bytes[pos + 6..];
        assert_eq!(emitted.len(), 253);
        assert!(emitted.iter().all(|&b| b == b'A'));
    }

    #[test]
    fn test_build_test_receipt() {
        let receipt = EscPosBuilder::build_test_receipt();
        assert!(!receipt.is_empty());
        assert!(receipt.len() > 100);
        assert!(
            receipt
                .windows(b"GHITAPRINT".len())
                .any(|w| w == b"GHITAPRINT")
        );
    }

    #[test]
    fn test_strip_diacritics_common_words() {
        assert_eq!(strip_diacritics("Cà phê sữa đá"), "Ca phe sua da");
        assert_eq!(strip_diacritics("Hóa đơn"), "Hoa don");
        assert_eq!(strip_diacritics("ĐƯỜNG"), "DUONG");
        assert_eq!(strip_diacritics("Nguyễn Văn Ất"), "Nguyen Van At");
        assert_eq!(strip_diacritics("ổ"), "o");
    }

    #[test]
    fn test_strip_keeps_ascii_and_other_scripts() {
        assert_eq!(strip_diacritics("plain ascii"), "plain ascii");
        assert_eq!(strip_diacritics("混合 text"), "混合 text");
        assert_eq!(strip_diacritics("emoji 😀 ok"), "emoji 😀 ok");
    }

    #[test]
    fn test_text_filters_control_chars() {
        let mut b = EscPosBuilder::new();
        b.text("a\u{1b}b\u{7}c\u{0}d");
        let bytes = b.into_bytes();
        assert!(bytes.ends_with(b"abcd"));
    }

    #[test]
    fn test_text_vi_strip_mode() {
        let mut b = EscPosBuilder::new();
        b.text_vi("Cà phê", "strip");
        let bytes = b.into_bytes();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("Ca phe"));
    }

    #[test]
    fn test_text_vi_keep_mode_tcvn3_full_map() {
        let mut b = EscPosBuilder::new();
        b.text_vi("Cà đâ", "keep");
        let bytes = b.into_bytes();
        assert!(
            bytes
                .windows(3)
                .any(|w| w == [0x1B, 0x74, ESC_T_CODEPAGE_VIETNAMESE_TCVN3])
        );
        assert!(bytes.contains(&0xB5));
        assert!(bytes.contains(&0xD0));
        assert!(bytes.contains(&0xB4));
    }

    #[test]
    fn test_text_vi_keep_falls_back_to_strip_when_unmappable() {
        let mut b = EscPosBuilder::new();
        b.text_vi("Cà phê", "keep");
        let bytes = b.into_bytes();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("Ca phe"));
        assert!(!text.contains('\u{ea}'));
    }

    #[test]
    fn test_text_vi_auto_uses_strip() {
        let mut b = EscPosBuilder::new();
        b.text_vi("Hóa đơn", "auto");
        let bytes = b.into_bytes();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("Hoa don"));
    }

    #[test]
    fn test_divider_uses_line_width() {
        let mut b = EscPosBuilder::new();
        b.set_line_width(48);
        b.divider("solid");
        let bytes = b.into_bytes();
        let text = String::from_utf8_lossy(&bytes);
        let line = text.lines().last().unwrap();
        assert!(line.ends_with(&"=".repeat(48)));
    }

    #[test]
    fn test_qr_payload_char_boundary_truncation() {
        let payload = "Đ".repeat(2000);
        let mut b = EscPosBuilder::new();
        b.qrcode(&payload, 6);
        let bytes = b.into_bytes();
        let ascii_body = String::from_utf8_lossy(&bytes).to_string();
        let emitted: String = ascii_body
            .chars()
            .filter(|c| !c.is_ascii() || c.is_alphanumeric())
            .collect();
        assert!(!emitted.contains('\u{fffd}'));
    }
}
