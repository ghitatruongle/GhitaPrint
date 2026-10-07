use super::EscPosBuilder;
use serde::{Deserialize, Serialize};

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
}

impl PrintDocument {
    pub fn compile_escpos(&self) -> Vec<u8> {
        let mut builder = EscPosBuilder::new();

        for cmd in &self.commands {
            match cmd {
                PrintCommand::Text {
                    value,
                    bold,
                    size,
                    align,
                } => {
                    if let Some(a) = align {
                        match a.as_str() {
                            "center" => builder.align_center(),
                            "right" => builder.align_right(),
                            _ => builder.align_left(),
                        };
                    }

                    builder.bold(*bold);

                    if let Some(s) = size {
                        if s == "2x" {
                            builder.font_size(2, 2);
                        } else if s == "3x" {
                            builder.font_size(3, 3);
                        } else {
                            builder.font_size(1, 1);
                        }
                    } else {
                        builder.font_size(1, 1);
                    }

                    builder.text(value);
                    if !value.ends_with('\n') {
                        builder.text("\n");
                    }
                }

                PrintCommand::Divider { style } => {
                    builder.divider(style);
                }

                PrintCommand::Table { columns, rows } => {
                    use std::fmt::Write;
                    builder.align_left().bold(false).font_size(1, 1);
                    let mut line_buf = String::with_capacity(128);
                    for col in columns {
                        let _ = write!(line_buf, "{:<width$}", col.title, width = col.width);
                    }
                    builder.text_line(&line_buf);
                    builder.divider("dashed");

                    for row in rows {
                        line_buf.clear();
                        for (i, cell) in row.iter().enumerate() {
                            let width = columns.get(i).map(|c| c.width).unwrap_or(10);
                            let _ = write!(line_buf, "{:<width$}", cell, width = width);
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

        builder.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_escpos_with_barcode_and_table() {
        let doc = PrintDocument {
            title: Some("Test Doc".to_string()),
            commands: vec![
                PrintCommand::Text {
                    value: "Header Text".to_string(),
                    bold: true,
                    size: Some("2x".to_string()),
                    align: Some("center".to_string()),
                },
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
            ],
        };

        let bytes = doc.compile_escpos();
        assert!(!bytes.is_empty());
        assert!(
            bytes
                .windows(b"Header Text".len())
                .any(|w| w == b"Header Text")
        );
        assert!(bytes.windows(b"Coffee".len()).any(|w| w == b"Coffee"));
        assert!(bytes.windows(b"12345678".len()).any(|w| w == b"12345678"));
    }
}
