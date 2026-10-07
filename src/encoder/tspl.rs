use std::io::Write;

pub struct TsplBuilder {
    buffer: Vec<u8>,
}

#[inline]
fn write_escaped_tspl_str(buf: &mut Vec<u8>, s: &str) {
    if !s.contains('"') {
        buf.extend_from_slice(s.as_bytes());
    } else {
        for &b in s.as_bytes() {
            if b == b'"' {
                buf.extend_from_slice(b"\\\"");
            } else {
                buf.push(b);
            }
        }
    }
}

#[inline]
fn write_unquoted_tspl_str(buf: &mut Vec<u8>, s: &str) {
    if !s.contains('"') {
        buf.extend_from_slice(s.as_bytes());
    } else {
        for &b in s.as_bytes() {
            if b != b'"' {
                buf.push(b);
            }
        }
    }
}

#[allow(dead_code)]
impl TsplBuilder {
    pub fn new(width_mm: u32, height_mm: u32) -> Self {
        let mut b = Self {
            buffer: Vec::with_capacity(512),
        };
        let _ = write!(
            b.buffer,
            "SIZE {} mm, {} mm\r\nGAP 2 mm, 0 mm\r\nDIRECTION 1\r\nCLS\r\n",
            width_mm, height_mm
        );
        b
    }

    pub fn gap(&mut self, gap_mm: u32) -> &mut Self {
        let _ = write!(self.buffer, "GAP {} mm, 0 mm\r\n", gap_mm);
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
        let _ = write!(
            self.buffer,
            "TEXT {},{},\"{}\",0,{},{},\"",
            x, y, font, x_mult, y_mult
        );
        write_escaped_tspl_str(&mut self.buffer, text);
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
        let human_readable = if readable { 1 } else { 0 };
        let _ = write!(
            self.buffer,
            "BARCODE {},{},\"128\",{},{},0,2,4,\"",
            x, y, height, human_readable
        );
        write_unquoted_tspl_str(&mut self.buffer, data);
        self.buffer.extend_from_slice(b"\"\r\n");
        self
    }

    pub fn qrcode(&mut self, x: u32, y: u32, cell_width: u8, data: &str) -> &mut Self {
        let _ = write!(self.buffer, "QRCODE {},{},L,{},A,0,\"", x, y, cell_width);
        write_escaped_tspl_str(&mut self.buffer, data);
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
        let _ = write!(
            self.buffer,
            "BOX {},{},{},{},{}\r\n",
            x, y, x_end, y_end, thickness
        );
        self
    }

    pub fn print(&mut self, copies: u32) -> &mut Self {
        let _ = write!(self.buffer, "PRINT 1, {}\r\n", copies);
        self
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

    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
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
        let bytes = b.into_bytes();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("Product \\\"Special\\\""));
    }
}
