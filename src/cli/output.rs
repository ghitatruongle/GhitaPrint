use colored::Colorize;
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, ContentArrangement, Row, Table};
use serde::Serialize;

pub fn print_banner() {
    println!(
        "{}",
        "============================================================".bright_blue()
    );
    println!(
        "  🖨️  {} - Universal Driverless Print Engine",
        "GhitaPrint".bold().bright_green()
    );
    println!(
        "  Phiên bản: {} | Chế độ CLI",
        env!("CARGO_PKG_VERSION").bright_yellow()
    );
    println!(
        "{}",
        "============================================================".bright_blue()
    );
}

pub fn print_success(msg: &str) {
    println!("{} {}", "✔".bright_green().bold(), msg);
}

pub fn print_error(msg: &str) {
    eprintln!("{} {}", "✖".bright_red().bold(), msg.bright_red());
}

pub fn print_info(msg: &str) {
    println!("{} {}", "ℹ".bright_cyan().bold(), msg);
}

pub fn print_warning(msg: &str) {
    println!("{} {}", "⚠".bright_yellow().bold(), msg);
}

#[derive(Serialize)]
pub struct PrinterInfo {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub address: String,
    pub status: String,
}

pub fn render_printers(printers: &[PrinterInfo], as_json: bool) {
    if as_json {
        if let Ok(json) = serde_json::to_string_pretty(printers) {
            println!("{}", json);
        }
        return;
    }

    if printers.is_empty() {
        print_warning("Không tìm thấy máy in nào đang kết nối.");
        println!("  • Kiểm tra lại dây cáp USB máy in đã cắm chưa.");
        println!("  • Hoặc kiểm tra máy in mạng đã bật nguồn và cùng mạng LAN chưa.");
        return;
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic);

    table.set_header(vec![
        Cell::new("Loại").fg(Color::Cyan),
        Cell::new("Tên Thiết Bị").fg(Color::Cyan),
        Cell::new("Địa Chỉ / Cổng").fg(Color::Cyan),
        Cell::new("Trạng Thái").fg(Color::Cyan),
    ]);

    for p in printers {
        let status_cell = if p.status == "Sẵn sàng" {
            Cell::new(&p.status).fg(Color::Green)
        } else {
            Cell::new(&p.status).fg(Color::Yellow)
        };

        table.add_row(Row::from(vec![
            Cell::new(&p.transport),
            Cell::new(&p.name),
            Cell::new(&p.address),
            status_cell,
        ]));
    }

    println!("{}", table);
}

#[allow(dead_code)]
pub fn render_json<T: Serialize>(data: &T) {
    match serde_json::to_string_pretty(data) {
        Ok(json) => println!("{}", json),
        Err(e) => print_error(&format!("Lỗi serialize JSON: {}", e)),
    }
}
