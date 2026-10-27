use super::EscPosBuilder;
use serde::{Deserialize, Serialize};

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

    #[test]
    fn test_table_cell_overflow_truncated() {
        let doc = PrintDocument {
            title: None,
            commands: vec![PrintCommand::Table {
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
            }],
        };

        let bytes = doc.compile_escpos();
        let text = String::from_utf8_lossy(&bytes);
        let data_line = text.lines().last().unwrap();
        assert_eq!(data_line, "Carame  35k");
        assert_eq!(data_line.chars().count(), 11);
    }

    #[test]
    fn test_table_column_alignment() {
        let doc = PrintDocument {
            title: None,
            commands: vec![PrintCommand::Table {
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
            }],
        };

        let bytes = doc.compile_escpos();
        let text = String::from_utf8_lossy(&bytes);
        let data_line = text.lines().last().unwrap();
        assert_eq!(data_line, "a      b c  ");
    }
}
