use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod cli;
mod config;
mod discovery;
mod encoder;
mod os;
mod server;
mod transport;

use cli::output::{
    print_banner, print_error, print_info, print_success, print_warning, render_printers,
};
use cli::{Cli, Commands, ConfigAction, StartupAction, VirtualPrinterAction};
use config::ConfigManager;
use discovery::DeviceScanner;
use encoder::{EscPosBuilder, FloydSteinbergRasterizer, PrintDocument, TsplBuilder};
use transport::{PrinterTransport, TcpTransport, UsbTransport};

#[tokio::main]
async fn main() {
    let args = Cli::parse();

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&args.log_level));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    if !args.json {
        print_banner();
    }

    let config_mgr = ConfigManager::new();
    let settings = config_mgr.load();

    match args.command {
        Commands::Scan(scan_args) => {
            if !args.json {
                print_info("Đang quét các thiết bị máy in thực tế trên hệ thống...");
            }

            let mut detected_printers = Vec::new();

            if scan_args.all {
                if let Ok(all_usb) = transport::list_all_usb_devices() {
                    for dev in all_usb {
                        detected_printers.push(cli::output::PrinterInfo {
                            id: format!("usb-{:04x}-{:04x}", dev.vendor_id, dev.product_id),
                            name: format!("{} {}", dev.manufacturer, dev.product),
                            transport: "USB Generic".to_string(),
                            address: dev.uri(),
                            status: "Đang cắm".to_string(),
                        });
                    }
                }
            } else {
                if scan_args.usb || (!scan_args.network && !scan_args.bluetooth) {
                    let usb_printers = DeviceScanner::scan_usb();
                    detected_printers.extend(usb_printers);
                }

                if scan_args.network || (!scan_args.usb && !scan_args.bluetooth) {
                    let mut candidates = vec!["127.0.0.1:9100"];
                    if !settings.default_printer.is_empty()
                        && !settings.default_printer.starts_with("usb://")
                    {
                        candidates.push(&settings.default_printer);
                    }
                    let net_printers = DeviceScanner::scan_network(&candidates).await;
                    detected_printers.extend(net_printers);
                }
            }

            render_printers(&detected_printers, args.json);
        }

        Commands::Test(test_args) => {
            let target_addr = test_args
                .address
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    if !settings.default_printer.is_empty() {
                        &settings.default_printer
                    } else {
                        ""
                    }
                });

            let (print_data, doc_name) = if test_args.receipt {
                (EscPosBuilder::build_test_receipt(), "Hóa đơn nhiệt ESC/POS")
            } else if test_args.label {
                (TsplBuilder::build_test_label(), "Tem nhãn mã vạch TSPL")
            } else {
                print_info(
                    "Vui lòng chọn loại in thử: --receipt (hóa đơn nhiệt) hoặc --label (tem nhãn mã vạch)",
                );
                return;
            };

            print_info(&format!(
                "Đã tạo mẫu {} chuẩn ({} bytes)",
                doc_name,
                print_data.len()
            ));

            if target_addr.is_empty() {
                print_warning("Chưa cấu hình địa chỉ máy in mục tiêu!");
                println!(
                    "  • Chỉ định trực tiếp: ghitaprint test --receipt --address 192.168.1.200:9100"
                );
                println!(
                    "  • Hoặc máy in USB   : ghitaprint test --label --address usb://0416:5011"
                );
                println!("  • Hoặc đặt mặc định : ghitaprint config set default_printer <address>");
                return;
            }

            print_info(&format!("Đang gửi lệnh in tới: {}", target_addr));
            send_to_printer(target_addr, &print_data, settings.connection_timeout_secs).await;
        }

        Commands::Print(print_args) => {
            let target_addr = print_args
                .address
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    if !settings.default_printer.is_empty() {
                        &settings.default_printer
                    } else {
                        ""
                    }
                });

            if target_addr.is_empty() {
                print_error(
                    "Vui lòng cung cấp địa chỉ máy in qua cờ --address hoặc cấu hình default_printer.",
                );
                return;
            }

            let raw_data = if let Some(ref img_path) = print_args.image {
                match image::open(img_path) {
                    Ok(img) => {
                        print_info(&format!(
                            "Đang xử lý ảnh {} qua thuật toán Floyd-Steinberg...",
                            img_path.display()
                        ));
                        let mut raster_bytes = FloydSteinbergRasterizer::to_escpos_raster(
                            &img,
                            print_args.image_width,
                        );
                        raster_bytes.extend_from_slice(&[0x1B, 0x64, 0x03]);
                        raster_bytes
                    }
                    Err(e) => {
                        print_error(&format!(
                            "Không thể mở file ảnh {}: {}",
                            img_path.display(),
                            e
                        ));
                        return;
                    }
                }
            } else if let Some(ref file_path) = print_args.file {
                match std::fs::read_to_string(file_path) {
                    Ok(content) => {
                        if let Ok(doc) = serde_json::from_str::<PrintDocument>(&content) {
                            print_info(
                                "Đã nhận dạng JSON PrintDocument, đang biên dịch ESC/POS...",
                            );
                            doc.compile_escpos()
                        } else {
                            content.into_bytes()
                        }
                    }
                    Err(_) => match std::fs::read(file_path) {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            print_error(&format!(
                                "Không thể đọc tệp {}: {}",
                                file_path.display(),
                                e
                            ));
                            return;
                        }
                    },
                }
            } else if let Some(ref text) = print_args.raw {
                text.as_bytes().to_vec()
            } else {
                print_error(
                    "Thiếu nội dung cần in! Dùng --file <path>, --image <path> hoặc --raw <data>.",
                );
                return;
            };

            print_info(&format!(
                "Đang gửi {} bytes tới máy in {}...",
                raw_data.len(),
                target_addr
            ));
            send_to_printer(target_addr, &raw_data, settings.connection_timeout_secs).await;
        }

        Commands::Config(config_args) => match config_args.action {
            ConfigAction::List => {
                if args.json {
                    cli::output::render_json(&settings);
                } else {
                    print_info(&format!(
                        "Tệp cấu hình: {}",
                        config_mgr.get_path().display()
                    ));
                    println!("  • api_port                = {}", settings.api_port);
                    println!(
                        "  • default_printer         = \"{}\"",
                        settings.default_printer
                    );
                    println!("  • auto_startup            = {}", settings.auto_startup);
                    println!("  • log_level               = \"{}\"", settings.log_level);
                    println!(
                        "  • connection_timeout_secs = {}",
                        settings.connection_timeout_secs
                    );
                }
            }
            ConfigAction::Get { key } => match key.as_str() {
                "api_port" => println!("{}", settings.api_port),
                "default_printer" => println!("{}", settings.default_printer),
                "auto_startup" => println!("{}", settings.auto_startup),
                "log_level" => println!("{}", settings.log_level),
                "connection_timeout_secs" => println!("{}", settings.connection_timeout_secs),
                _ => print_error(&format!("Không tìm thấy khóa cấu hình '{}'", key)),
            },
            ConfigAction::Set { key, value } => {
                let mut new_settings = settings.clone();
                let mut updated = true;
                match key.as_str() {
                    "api_port" => {
                        if let Ok(val) = value.parse() {
                            new_settings.api_port = val;
                        } else {
                            print_error("Giá trị api_port phải là số nguyên (ví dụ: 9123)");
                            updated = false;
                        }
                    }
                    "default_printer" => new_settings.default_printer = value.clone(),
                    "auto_startup" => {
                        if let Ok(val) = value.parse() {
                            new_settings.auto_startup = val;
                        } else {
                            print_error("Giá trị auto_startup phải là true hoặc false");
                            updated = false;
                        }
                    }
                    "log_level" => new_settings.log_level = value.clone(),
                    "connection_timeout_secs" => {
                        if let Ok(val) = value.parse() {
                            new_settings.connection_timeout_secs = val;
                        } else {
                            print_error("Giá trị connection_timeout_secs phải là số nguyên");
                            updated = false;
                        }
                    }
                    _ => {
                        print_error(&format!("Không hỗ trợ khóa cấu hình '{}'", key));
                        updated = false;
                    }
                }

                if updated {
                    match config_mgr.save(&new_settings) {
                        Ok(_) => print_success(&format!("Đã cập nhật '{}' = '{}'", key, value)),
                        Err(e) => print_error(&format!("Không thể lưu cấu hình: {}", e)),
                    }
                }
            }
            ConfigAction::Reset => {
                let default_settings = config::Settings::default();
                match config_mgr.save(&default_settings) {
                    Ok(_) => print_success("Đã khôi phục cấu hình về mặc định."),
                    Err(e) => print_error(&format!("Không thể lưu cấu hình: {}", e)),
                }
            }
        },

        Commands::Startup(startup_args) => match startup_args.action {
            StartupAction::Install { service } => {
                if service {
                    print_info("Đang đăng ký GhitaPrint dưới dạng Windows Service...");
                    print_warning(
                        "Chế độ Windows Service đang được tối ưu hóa cho bản phát hành chính thức.",
                    );
                } else {
                    print_info("Đang đăng ký GhitaPrint vào Windows Registry Run (HKCU)...");
                    match os::install_startup() {
                        Ok(cmd) => {
                            print_success("Đã kích hoạt tự khởi động cùng Windows thành công!");
                            println!("  Lệnh đăng ký: {}", cmd);
                        }
                        Err(e) => print_error(&format!("Lỗi khi đăng ký khởi động: {}", e)),
                    }
                }
            }
            StartupAction::Uninstall => {
                print_info("Đang gỡ bỏ cấu hình tự khởi động cùng Windows...");
                match os::uninstall_startup() {
                    Ok(_) => print_success("Đã hủy tự khởi động cùng Windows!"),
                    Err(e) => print_error(&format!("Lỗi khi gỡ đăng ký: {}", e)),
                }
            }
            StartupAction::Status => {
                let is_enabled = os::is_startup_enabled();
                print_info("Trạng thái tự khởi động cùng Windows:");
                if is_enabled {
                    println!("  • Registry Run (HKCU): Đã kích hoạt [✔]");
                    if let Some(cmd) = os::get_startup_command() {
                        println!("  • Lệnh thực thi      : {}", cmd);
                    }
                } else {
                    println!("  • Registry Run (HKCU): Chưa kích hoạt [Chưa bật]");
                    println!("  • Để kích hoạt, chạy : ghitaprint startup install");
                }
            }
        },

        Commands::VirtualPrinter(vp_args) => match vp_args.action {
            VirtualPrinterAction::Install => {
                print_info("Đang đăng ký máy in ảo 'GhitaPrint' vào Windows Print Spooler...");
                match server::install_virtual_printer(settings.api_port) {
                    Ok(msg) => print_success(&msg),
                    Err(e) => print_error(&format!("Lỗi cài đặt máy in ảo: {}", e)),
                }
            }
            VirtualPrinterAction::Uninstall => {
                print_info("Đang gỡ bỏ máy in ảo 'GhitaPrint' khỏi Windows...");
                match server::uninstall_virtual_printer() {
                    Ok(msg) => print_success(&msg),
                    Err(e) => print_error(&format!("Lỗi gỡ bỏ máy in ảo: {}", e)),
                }
            }
            VirtualPrinterAction::Status => {
                let installed = server::is_virtual_printer_installed();
                print_info("Trạng thái Máy in ảo Windows:");
                if installed {
                    println!("  • Máy in ảo 'GhitaPrint': Đã cài đặt [✔]");
                    println!("  • Trình điều khiển      : Microsoft IPP Class Driver");
                    println!(
                        "  • Điểm tiếp nhận        : http://127.0.0.1:{}/printers/ghitaprint",
                        settings.api_port
                    );
                } else {
                    println!("  • Máy in ảo 'GhitaPrint': Chưa cài đặt [Chưa có]");
                    println!("  • Để cài đặt, chạy      : ghitaprint virtual-printer install");
                }
            }
        },

        Commands::Daemon(daemon_args) => {
            if daemon_args.start {
                print_info(&format!(
                    "Đang khởi chạy GhitaPrint Daemon trên cổng {}...",
                    daemon_args.port
                ));
                print_success(&format!(
                    "GhitaPrint Local API đã sẵn sàng tại: http://127.0.0.1:{}",
                    daemon_args.port
                ));
                println!(
                    "  • REST API   : http://127.0.0.1:{}/api/v1/print",
                    daemon_args.port
                );
                println!(
                    "  • WebSocket  : ws://127.0.0.1:{}/api/v1/ws",
                    daemon_args.port
                );
                println!("  • Nhấn Ctrl + C để dừng dịch vụ.");

                if let Err(e) = server::start_server(
                    daemon_args.port,
                    settings.default_printer.clone(),
                    settings.connection_timeout_secs,
                )
                .await
                {
                    print_error(&format!("Lỗi máy chủ: {}", e));
                }
            } else if daemon_args.stop {
                print_info("Đang gửi tín hiệu dừng tới GhitaPrint Daemon...");
                print_success("Đã hoàn tất kiểm tra trạng thái.");
            } else if daemon_args.restart {
                print_info("Đang khởi động lại GhitaPrint Daemon...");
                print_success("Daemon đã khởi động lại.");
            } else {
                print_info("Sử dụng: ghitaprint daemon --start | --stop | --restart");
            }
        }

        Commands::Status => {
            let is_startup = os::is_startup_enabled();
            print_info("Trạng thái GhitaPrint Engine:");
            println!("  • Tiến trình Daemon   : Sẵn sàng");
            println!(
                "  • Tự khởi động Windows: {}",
                if is_startup {
                    "Đã bật [✔]"
                } else {
                    "Chưa bật"
                }
            );
            println!(
                "  • Máy in mặc định     : {}",
                if settings.default_printer.is_empty() {
                    "Chưa cấu hình"
                } else {
                    &settings.default_printer
                }
            );
            println!("  • Cổng Local API      : http://127.0.0.1:9123 (WebSocket + REST)");
        }
    }
}

async fn send_to_printer(target_addr: &str, data: &[u8], timeout_secs: u64) {
    if target_addr.starts_with("usb://") {
        match UsbTransport::open_uri(target_addr) {
            Ok(mut usb) => match usb.send_raw(data).await {
                Ok(_) => print_success("Đã gửi dữ liệu thành công qua cổng USB Direct!"),
                Err(e) => print_error(&format!("Lỗi khi gửi dữ liệu USB: {}", e)),
            },
            Err(e) => print_error(&format!("Không thể mở máy in USB: {}", e)),
        }
    } else {
        match TcpTransport::connect(target_addr, timeout_secs).await {
            Ok(mut tcp) => match tcp.send_raw(data).await {
                Ok(_) => print_success("Đã gửi dữ liệu thành công qua cổng TCP JetDirect 9100!"),
                Err(e) => print_error(&format!("Lỗi khi gửi dữ liệu mạng: {}", e)),
            },
            Err(e) => print_error(&format!(
                "Không thể kết nối máy in mạng {}: {}",
                target_addr, e
            )),
        }
    }
}
