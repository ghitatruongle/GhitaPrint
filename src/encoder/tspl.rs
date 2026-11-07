use std::io::Write;

pub struct TsplBuilder {
    buffer: Vec<u8>,
    gap_mm: u32,
    header_emitted: bool,
}

fn sanitize_tspl_value(s: &str) -> String {
    s.replace('"', "'")
        .replace('\\', "/")
        .replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
}

#[allow(dead_code)]
impl TsplBuilder {
    pub fn new(width_mm: u32, height_mm: u32) -> Self {
        let mut b = Self {
            buffer: Vec::with_capacity(512),
            gap_mm: 2,
            header_emitted: false,
        };
        let w = width_mm.clamp(1, 210);
        let h = height_mm.clamp(1, 210);
        let _ = write!(b.buffer, "SIZE {} mm, {} mm\r\n", w, h);
        b
    }

    fn ensure_header(&mut self) {
        if !self.header_emitted {
            let _ = write!(
                self.buffer,
                "GAP {} mm, 0 mm\r\nDIRECTION 1\r\nCODEPAGE UTF-8\r\nCLS\r\n",
                self.gap_mm
            );
            self.header_emitted = true;
        }
    }

    pub fn gap(&mut self, gap_mm: u32) -> &mut Self {
        self.gap_mm = gap_mm.min(60);
        self
    }

    pub fn text(
        &mut self,
        x: u32,
        y: u32,
        font: &str,
        x_mult: u8,
        y_mult: u8,
        text: &str,
    ) -> &mut Self {
        self.ensure_header();
        let xm = x_mult.clamp(1, 10);
        let ym = y_mult.clamp(1, 10);
        let _ = write!(
            self.buffer,
            "TEXT {},{},\"{}\",0,{},{},\"",
            x, y, font, xm, ym
        );
        let sanitized = sanitize_tspl_value(text);
        self.buffer.extend_from_slice(sanitized.as_bytes());
        self.buffer.extend_from_slice(b"\"\r\n");
        self
    }

    pub fn barcode_128(
        &mut self,
        x: u32,
        y: u32,
        height: u32,
        readable: bool,
        data: &str,
    ) -> &mut Self {
        self.ensure_header();
        let human_readable = if readable { 1 } else { 0 };
        let _ = write!(
            self.buffer,
            "BARCODE {},{},\"128\",{},{},0,2,4,\"",
            x,
            y,
            height.clamp(10, 200),
            human_readable
        );
        let sanitized = sanitize_tspl_value(data);
        self.buffer.extend_from_slice(sanitized.as_bytes());
        self.buffer.extend_from_slice(b"\"\r\n");
        self
    }

    pub fn qrcode(&mut self, x: u32, y: u32, cell_width: u8, data: &str) -> &mut Self {
        self.ensure_header();
        let _ = write!(
            self.buffer,
            "QRCODE {},{},L,{},A,0,\"",
            x,
            y,
            cell_width.clamp(1, 10)
        );
        let sanitized = sanitize_tspl_value(data);
        self.buffer.extend_from_slice(sanitized.as_bytes());
        self.buffer.extend_from_slice(b"\"\r\n");
        self
    }

    pub fn box_rect(
        &mut self,
        x: u32,
        y: u32,
        x_end: u32,
        y_end: u32,
        thickness: u32,
    ) -> &mut Self {
        self.ensure_header();
        let _ = write!(
            self.buffer,
            "BOX {},{},{},{},{}\r\n",
            x,
            y,
            x_end,
            y_end,
            thickness.clamp(1, 60)
        );
        self
    }

    pub fn print(&mut self, copies: u32) -> &mut Self {
        self.ensure_header();
        let _ = write!(self.buffer, "PRINT 1, {}\r\n", copies.clamp(1, 999));
        self
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }

    pub fn build_test_label() -> Vec<u8> {
        let mut b = Self::new(50, 30);
        b.box_rect(10, 10, 390, 230, 2);
        b.text(20, 20, "3", 1, 1, "GHITA LOGISTICS");
        b.text(20, 50, "2", 1, 1, "Ma van don: GH-889922");
        b.barcode_128(20, 80, 60, true, "GH889922VN");
        b.text(20, 170, "2", 1, 1, "Gia tri: 250.000d");
        b.qrcode(280, 140, 4, "https://ghita.ai/track/GH889922VN");
        b.print(1);
        b.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tspl_builder_basics() {
        let mut b = TsplBuilder::new(50, 30);
        b.text(10, 10, "3", 1, 1, "Hello TSPL");
        b.print(1);
        let bytes = b.into_bytes();
        let s = String::from_utf8_lossy(&bytes);

        assert!(s.contains("SIZE 50 mm, 30 mm"));
        assert!(s.contains("CLS"));
        assert!(s.contains("TEXT 10,10,\"3\",0,1,1,\"Hello TSPL\""));
        assert!(s.contains("PRINT 1, 1"));
    }

    #[test]
    fn test_tspl_header_order_gap_before_cls() {
        let mut b = TsplBuilder::new(50, 30);
        b.gap(3);
        b.text(10, 10, "3", 1, 1, "X");
        let s = String::from_utf8_lossy(&b.into_bytes()).to_string();
        let gap_pos = s.find("GAP 3 mm").expect("phải có GAP");
        let cls_pos = s.find("CLS").expect("phải có CLS");
        assert!(gap_pos < cls_pos);
        assert!(s.contains("DIRECTION 1"));
        assert!(s.contains("CODEPAGE UTF-8"));
    }

    #[test]
    fn test_tspl_test_label() {
        let label = TsplBuilder::build_test_label();
        assert!(!label.is_empty());
        let s = String::from_utf8_lossy(&label);
        assert!(s.contains("GHITA LOGISTICS"));
        assert!(s.contains("BARCODE"));
        assert!(s.contains("QRCODE"));
    }

    #[test]
    fn test_tspl_quote_escaping() {
        let mut b = TsplBuilder::new(50, 30);
        b.text(10, 10, "3", 1, 1, "Product \"Special\"");
        let s = String::from_utf8_lossy(&b.into_bytes()).to_string();
        assert!(s.contains("Product 'Special'"));
    }

    #[test]
    fn test_tspl_crlf_injection_neutralized() {
        let mut b = TsplBuilder::new(50, 30);
        b.text(10, 10, "3", 1, 1, "Hi\r\nPRINT 9999,1");
        let s = String::from_utf8_lossy(&b.into_bytes()).to_string();
        assert_eq!(s.matches("PRINT").count(), 1);
        assert!(s.contains("Hi PRINT 9999,1"));
    }

    #[test]
    fn test_tspl_magnifier_and_copies_clamped() {
        let mut b = TsplBuilder::new(50, 30);
        b.text(10, 10, "3", 0, 255, "X");
        b.print(0);
        let s = String::from_utf8_lossy(&b.into_bytes()).to_string();
        assert!(s.contains("TEXT 10,10,\"3\",0,1,10,"));
        assert!(s.contains("PRINT 1, 1"));
    }

    #[test]
    fn test_tspl_dimensions_clamped() {
        let b = TsplBuilder::new(0, 9999);
        let s = String::from_utf8_lossy(&b.into_bytes()).to_string();
        assert!(s.contains("SIZE 1 mm, 210 mm"));
    }
}
