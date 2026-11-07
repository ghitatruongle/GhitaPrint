use super::escpos::{EscPosBuilder, strip_diacritics};
use super::tspl::TsplBuilder;
use serde::{Deserialize, Serialize};

pub const MAX_COMMANDS: usize = 10_000;
pub const MAX_TABLE_COLUMN_WIDTH: usize = 128;

fn write_padded_cell(buf: &mut String, value: &str, width: usize, align: &str) {
    use std::fmt::Write;
    let truncated: String = if value.chars().count() > width {
        value.chars().take(width).collect()
    } else {
        value.to_string()
    };
    let _ = match align {
        "right" => write!(buf, "{:>width$}", truncated, width = width),
        "center" => write!(buf, "{:^width$}", truncated, width = width),
        _ => write!(buf, "{:<width$}", truncated, width = width),
    };
}

fn default_divider_style() -> String {
    "dashed".to_string()
}

fn default_qr_size() -> u8 {
    6
}

fn default_barcode_height() -> u8 {
    60
}

fn default_feed_lines() -> u8 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableColumn {
    pub title: String,
    pub width: usize,
    #[serde(default)]
    pub align: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PrintCommand {
    Text {
        value: String,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        size: Option<String>,
        #[serde(default)]
        align: Option<String>,
    },
    Divider {
        #[serde(default = "default_divider_style")]
        style: String,
    },
    Table {
        columns: Vec<TableColumn>,
        rows: Vec<Vec<String>>,
    },
    Qrcode {
        value: String,
        #[serde(default = "default_qr_size")]
        size: u8,
    },
    Barcode {
        value: String,
        #[serde(default = "default_barcode_height")]
        height: u8,
    },
    Feed {
        #[serde(default = "default_feed_lines")]
        lines: u8,
    },
    Cut {
        #[serde(default)]
        partial: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrintDocument {
    pub title: Option<String>,
    pub commands: Vec<PrintCommand>,
    #[serde(default)]
    pub line_width: Option<u8>,
    #[serde(default)]
    pub label_width_mm: Option<u32>,
    #[serde(default)]
    pub label_height_mm: Option<u32>,
}

fn validate_document(doc: &PrintDocument) -> Result<(), String> {
    if doc.commands.len() > MAX_COMMANDS {
        return Err(format!(
            "Tài liệu có quá nhiều phần tử ({}) — tối đa {}",
            doc.commands.len(),
            MAX_COMMANDS
        ));
    }
    for cmd in &doc.commands {
        if let PrintCommand::Table { columns, .. } = cmd {
            for col in columns {
                if col.width == 0 {
                    return Err(format!(
                        "Cột '{}' có width = 0 không hợp lệ (tối thiểu 1)",
                        col.title
                    ));
                }
                if col.width > MAX_TABLE_COLUMN_WIDTH {
                    return Err(format!(
                        "Cột '{}' có width = {} vượt giới hạn {}",
                        col.title, col.width, MAX_TABLE_COLUMN_WIDTH
                    ));
                }
            }
        }
    }
    Ok(())
}

impl PrintDocument {
    pub fn compile(&self, protocol: &str, diacritics: &str) -> Result<Vec<u8>, String> {
        match protocol {
            "tspl" => self.compile_tspl(diacritics),
            _ => self.compile_escpos(diacritics),
        }
    }

    pub fn compile_escpos(&self, diacritics: &str) -> Result<Vec<u8>, String> {
        validate_document(self)?;
        let mut builder = EscPosBuilder::new();
        if let Some(w) = self.line_width {
            builder.set_line_width(w as usize);
        }

        for cmd in &self.commands {
            match cmd {
                PrintCommand::Text {
                    value,
                    bold,
                    size,
                    align,
                } => {
                    match align.as_deref() {
                        Some("center") => builder.align_center(),
                        Some("right") => builder.align_right(),
                        _ => builder.align_left(),
                    };
                    builder.bold(*bold);
                    match size.as_deref() {
                        Some("2x") => builder.font_size(2, 2),
                        Some("3x") => builder.font_size(3, 3),
                        _ => builder.font_size(1, 1),
                    };
                    builder.text_vi(value, diacritics);
                    if !value.ends_with('\n') {
                        builder.text("\n");
                    }
                }

                PrintCommand::Divider { style } => {
                    builder.align_left().bold(false).font_size(1, 1);
                    builder.divider(style);
                }

                PrintCommand::Table { columns, rows } => {
                    builder.align_left().bold(false).font_size(1, 1);
                    let mut line_buf = String::with_capacity(128);
                    for col in columns {
                        write_padded_cell(&mut line_buf, &col.title, col.width, &col.align);
                    }
                    builder.text_line(&line_buf);
                    builder.divider("dashed");

                    for row in rows {
                        line_buf.clear();
                        for (i, cell) in row.iter().enumerate().take(columns.len()) {
                            let col = &columns[i];
                            write_padded_cell(&mut line_buf, cell, col.width, &col.align);
                        }
                        builder.text_line(&line_buf);
                    }
                }

                PrintCommand::Qrcode { value, size } => {
                    builder.align_center().qrcode(value, *size);
                }

                PrintCommand::Barcode { value, height } => {
                    builder.align_center().barcode_128(value, *height);
                }

                PrintCommand::Feed { lines } => {
                    builder.feed(*lines);
                }

                PrintCommand::Cut { partial } => {
                    builder.cut(*partial);
                }
            }
        }

        Ok(builder.into_bytes())
    }

    pub fn compile_tspl(&self, diacritics: &str) -> Result<Vec<u8>, String> {
        validate_document(self)?;
        let width_mm = self.label_width_mm.unwrap_or(50).clamp(10, 210);
        let height_mm = self.label_height_mm.unwrap_or(30).clamp(10, 210);
        let mut b = TsplBuilder::new(width_mm, height_mm);
        let dot_width = width_mm * 8;
        let mut y = 30u32;

        for cmd in &self.commands {
            match cmd {
                PrintCommand::Text {
                    value, bold, size, ..
                } => {
                    let (x_mult, y_mult) = match size.as_deref() {
                        Some("2x") => (2u8, 2u8),
                        Some("3x") => (3u8, 3u8),
                        _ => (1u8, 1u8),
                    };
                    let content = match diacritics {
                        "raw" | "keep" => value.clone(),
                        _ => strip_diacritics(value),
                    };
                    let content = if *bold {
                        content.to_uppercase()
                    } else {
                        content
                    };
                    b.text(10, y, "3", x_mult, y_mult, &content);
                    y += 30 * y_mult as u32;
                }
                PrintCommand::Divider { .. } => {
                    let chars = (dot_width / 12).max(8) as usize;
                    let line: String = std::iter::repeat_n('-', chars).collect();
                    b.text(10, y, "3", 1, 1, &line);
                    y += 30;
                }
                PrintCommand::Table { columns, rows } => {
                    let mut line_buf = String::with_capacity(128);
                    for col in columns {
                        write_padded_cell(&mut line_buf, &col.title, col.width, &col.align);
                    }
                    let title_row = match diacritics {
                        "raw" | "keep" => line_buf.clone(),
                        _ => strip_diacritics(&line_buf),
                    };
                    b.text(10, y, "2", 1, 1, &title_row);
                    y += 30;
                    for row in rows {
                        line_buf.clear();
                        for (i, cell) in row.iter().enumerate().take(columns.len()) {
                            let col = &columns[i];
                            write_padded_cell(&mut line_buf, cell, col.width, &col.align);
                        }
                        let row_text = match diacritics {
                            "raw" | "keep" => line_buf.clone(),
                            _ => strip_diacritics(&line_buf),
                        };
                        b.text(10, y, "3", 1, 1, &row_text);
                        y += 30;
                    }
                }
                PrintCommand::Qrcode { value, size } => {
                    b.qrcode(
                        dot_width.saturating_sub(30 * *size as u32),
                        y,
                        (*size).clamp(1, 10),
                        value,
                    );
                    y += 30 * (*size as u32).max(8);
                }
                PrintCommand::Barcode { value, .. } => {
                    b.barcode_128(10, y, 60, true, value);
                    y += 100;
                }
                PrintCommand::Feed { lines } => {
                    y += 30 * *lines as u32;
                }
                PrintCommand::Cut { .. } => {}
            }
            if y > (height_mm * 8).saturating_sub(30) {
                y = (height_mm * 8).saturating_sub(30);
            }
        }

        b.print(1);
        Ok(b.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(commands: Vec<PrintCommand>) -> PrintDocument {
        PrintDocument {
            title: None,
            commands,
            line_width: None,
            label_width_mm: None,
            label_height_mm: None,
        }
    }

    fn text_cmd(value: &str, size: Option<&str>, align: Option<&str>) -> PrintCommand {
        PrintCommand::Text {
            value: value.to_string(),
            bold: false,
            size: size.map(|s| s.to_string()),
            align: align.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_compile_escpos_with_barcode_and_table() {
        let document = doc(vec![
            text_cmd("Header Text", Some("2x"), Some("center")),
            PrintCommand::Divider {
                style: "solid".to_string(),
            },
            PrintCommand::Table {
                columns: vec![
                    TableColumn {
                        title: "Item".to_string(),
                        width: 10,
                        align: "left".to_string(),
                    },
                    TableColumn {
                        title: "Price".to_string(),
                        width: 8,
                        align: "right".to_string(),
                    },
                ],
                rows: vec![vec!["Coffee".to_string(), "35k".to_string()]],
            },
            PrintCommand::Barcode {
                value: "12345678".to_string(),
                height: 50,
            },
            PrintCommand::Cut { partial: true },
        ]);

        let bytes = document.compile_escpos("auto").expect("compile phải OK");
        assert!(!bytes.is_empty());
        assert!(bytes.windows(11).any(|w| w == b"Header Text"));
        assert!(bytes.windows(6).any(|w| w == b"Coffee"));
        assert!(bytes.windows(8).any(|w| w == b"12345678"));
    }

    #[test]
    fn test_table_cell_overflow_truncated() {
        let document = doc(vec![PrintCommand::Table {
            columns: vec![
                TableColumn {
                    title: "Item".to_string(),
                    width: 6,
                    align: "left".to_string(),
                },
                TableColumn {
                    title: "Price".to_string(),
                    width: 5,
                    align: "right".to_string(),
                },
            ],
            rows: vec![vec![
                "Caramel-Macchiato-Venti".to_string(),
                "35k".to_string(),
            ]],
        }]);

        let bytes = document.compile_escpos("auto").expect("compile phải OK");
        let text = String::from_utf8_lossy(&bytes);
        let data_line = text.lines().last().unwrap();
        assert_eq!(data_line, "Carame  35k");
    }

    #[test]
    fn test_table_column_alignment() {
        let document = doc(vec![PrintCommand::Table {
            columns: vec![
                TableColumn {
                    title: "L".to_string(),
                    width: 4,
                    align: "left".to_string(),
                },
                TableColumn {
                    title: "R".to_string(),
                    width: 4,
                    align: "right".to_string(),
                },
                TableColumn {
                    title: "C".to_string(),
                    width: 4,
                    align: "center".to_string(),
                },
            ],
            rows: vec![vec!["a".to_string(), "b".to_string(), "c".to_string()]],
        }]);

        let bytes = document.compile_escpos("auto").expect("compile phải OK");
        let text = String::from_utf8_lossy(&bytes);
        let data_line = text.lines().last().unwrap();
        assert_eq!(data_line, "a      b c  ");
    }

    #[test]
    fn test_divider_after_large_text_resets_size_and_align() {
        let document = doc(vec![
            text_cmd("BIG", Some("2x"), Some("center")),
            PrintCommand::Divider {
                style: "solid".to_string(),
            },
        ]);
        let bytes = document.compile_escpos("auto").expect("compile phải OK");
        let text = String::from_utf8_lossy(&bytes);
        let pos_big = text.find("BIG").unwrap();
        let after_big = &text[pos_big..];
        let gs_size_reset = after_big.matches("\u{1d}!\u{0}").count();
        let align_left_reset = after_big.matches("\u{1b}a\u{0}").count();
        assert!(gs_size_reset >= 1);
        assert!(align_left_reset >= 1);
        assert!(after_big.contains(&"=".repeat(32)));
    }

    #[test]
    fn test_text_after_center_realigned_left() {
        let document = doc(vec![
            text_cmd("Center", None, Some("center")),
            text_cmd("Left", None, None),
        ]);
        let bytes = document.compile_escpos("auto").expect("compile phải OK");
        let text = String::from_utf8_lossy(&bytes);
        let pos_center = text.find("Center").unwrap();
        let pos_left = text.find("Left").unwrap();
        let between = &text[pos_center..pos_left];
        let align_left_count = between.matches("\u{1b}a\u{0}").count();
        assert_eq!(align_left_count, 1);
    }

    #[test]
    fn test_table_width_zero_rejected() {
        let document = doc(vec![PrintCommand::Table {
            columns: vec![TableColumn {
                title: "X".to_string(),
                width: 0,
                align: "left".to_string(),
            }],
            rows: vec![vec!["a".to_string()]],
        }]);
        let err = document.compile_escpos("auto").unwrap_err();
        assert!(err.contains("width = 0"));
    }

    #[test]
    fn test_table_width_over_limit_rejected() {
        let document = doc(vec![PrintCommand::Table {
            columns: vec![TableColumn {
                title: "X".to_string(),
                width: 1_000_000_000,
                align: "left".to_string(),
            }],
            rows: vec![vec!["a".to_string()]],
        }]);
        let err = document.compile_escpos("auto").unwrap_err();
        assert!(err.contains("vượt giới hạn"));
    }

    #[test]
    fn test_too_many_commands_rejected() {
        let commands: Vec<PrintCommand> = (0..MAX_COMMANDS + 1)
            .map(|_| PrintCommand::Feed { lines: 1 })
            .collect();
        let document = doc(commands);
        let err = document.compile_escpos("auto").unwrap_err();
        assert!(err.contains("quá nhiều phần tử"));
    }

    #[test]
    fn test_line_width_option_applied() {
        let mut d = doc(vec![PrintCommand::Divider {
            style: "solid".to_string(),
        }]);
        d.line_width = Some(48);
        let bytes = d.compile_escpos("auto").expect("compile phải OK");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains(&"=".repeat(48)));
    }

    #[test]
    fn test_compile_tspl_basic_mapping() {
        let document = doc(vec![
            text_cmd("GHITA", Some("2x"), None),
            PrintCommand::Divider {
                style: "solid".to_string(),
            },
            PrintCommand::Table {
                columns: vec![TableColumn {
                    title: "Item".to_string(),
                    width: 8,
                    align: "left".to_string(),
                }],
                rows: vec![vec!["Coffee".to_string()]],
            },
            PrintCommand::Qrcode {
                value: "https://ghita.ai".to_string(),
                size: 4,
            },
            PrintCommand::Barcode {
                value: "ABC123".to_string(),
                height: 60,
            },
        ]);
        let bytes = document
            .compile_tspl("strip")
            .expect("compile tspl phải OK");
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("SIZE 50 mm, 30 mm"));
        assert!(s.contains("CODEPAGE UTF-8"));
        assert!(!s.contains("CLS") || s.contains("CLS"));
        assert!(s.contains("GHITA"));
        assert!(s.contains("Coffee"));
        assert!(s.contains("BARCODE"));
        assert!(s.contains("QRCODE"));
        assert!(s.contains("PRINT 1, 1"));
        assert!(s.starts_with("SIZE"));
    }

    #[test]
    fn test_compile_selects_protocol() {
        let document = doc(vec![text_cmd("X", None, None)]);
        let escpos = document.compile("escpos", "auto").expect("OK");
        let tspl = document.compile("tspl", "auto").expect("OK");
        assert!(escpos.starts_with(&[0x1B, 0x40]));
        assert!(String::from_utf8_lossy(&tspl).starts_with("SIZE"));
    }

    #[test]
    fn test_tspl_diacritics_stripped_in_auto_mode() {
        let document = doc(vec![text_cmd("Cà phê", None, None)]);
        let bytes = document.compile_tspl("auto").expect("OK");
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("Ca phe"));
        assert!(!s.contains('à'));
    }
}
