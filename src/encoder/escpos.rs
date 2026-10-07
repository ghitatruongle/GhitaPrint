pub struct EscPosBuilder {
    buffer: Vec<u8>,
}

#[allow(dead_code)]
impl EscPosBuilder {
    pub fn new() -> Self {
        Self::with_capacity(512)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let mut builder = Self {
            buffer: Vec::with_capacity(capacity),
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
        let w = (width_mult.saturating_sub(1).min(7)) << 4;
        let h = height_mult.saturating_sub(1).min(7);
        self.buffer.extend_from_slice(&[0x1D, 0x21, w | h]);
        self
    }

    pub fn text(&mut self, text: &str) -> &mut Self {
        self.buffer.extend_from_slice(text.as_bytes());
        self
    }

    pub fn text_line(&mut self, text: &str) -> &mut Self {
        self.text(text);
        self.buffer.push(b'\n');
        self
    }

    pub fn divider(&mut self, style: &str) -> &mut Self {
        let line = match style {
            "solid" => "════════════════════════════════\n",
            "double" => "================================\n",
            _ => "--------------------------------\n",
        };
        self.text(line)
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
        let data = content.as_bytes();
        let len = (data.len() + 2) as u8;

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
        let data = content.as_bytes();
        let len = (data.len() + 3) as u16;
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
}
