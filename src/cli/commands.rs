use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "ghitaprint",
    author = "Ghita Team",
    version = "0.0.0-preview",
    about = "🖨️ Universal, Driverless & Ultra-fast Print Engine written in Rust",
    long_about = "GhitaPrint là bộ driver và công cụ điều khiển in ấn đa năng, kết nối mọi loại máy in mà không cần cài đặt driver thủ công."
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        help = "Xuất kết quả dưới định dạng JSON cho ứng dụng hoặc script"
    )]
    pub json: bool,

    #[arg(
        short,
        long,
        global = true,
        default_value = "info",
        help = "Mức độ chi tiết log (trace, debug, info, warn, error)"
    )]
    pub log_level: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(about = "Quét và phát hiện các máy in đang kết nối (USB, Mạng LAN, Bluetooth)")]
    Scan(ScanArgs),

    #[command(about = "In thử nghiệm nhanh để kiểm tra máy in (Hóa đơn nhiệt, Tem nhãn)")]
    Test(TestArgs),

    #[command(about = "Gửi lệnh in tài liệu từ file JSON, text hoặc dữ liệu thô")]
    Print(PrintArgs),

    #[command(about = "Quản lý cấu hình hệ thống (xem, sửa file ghitaprint.toml)")]
    Config(ConfigArgs),

    #[command(about = "Quản lý đăng ký tự khởi động cùng Windows")]
    Startup(StartupArgs),

    #[command(about = "Điều khiển tiến trình chạy nền (Background Daemon)")]
    Daemon(DaemonArgs),

    #[command(about = "Quản lý máy in ảo Windows (hỗ trợ in từ Word, Chrome qua Ctrl+P)")]
    VirtualPrinter(VirtualPrinterArgs),

    #[command(about = "Kiểm tra trạng thái hoạt động của daemon và máy in")]
    Status,
}

#[derive(Args, Debug)]
pub struct VirtualPrinterArgs {
    #[command(subcommand)]
    pub action: VirtualPrinterAction,
}

#[derive(Subcommand, Debug)]
pub enum VirtualPrinterAction {
    #[command(about = "Cài đặt máy in ảo \"GhitaPrint\" vào Windows")]
    Install,
    #[command(about = "Gỡ bỏ máy in ảo \"GhitaPrint\" khỏi Windows")]
    Uninstall,
    #[command(about = "Kiểm tra trạng thái máy in ảo")]
    Status,
}

#[derive(Args, Debug)]
pub struct ScanArgs {
    #[arg(long, help = "Chỉ quét thiết bị qua cổng USB")]
    pub usb: bool,

    #[arg(long, help = "Liệt kê toàn bộ thiết bị USB cắm trên máy để chẩn đoán")]
    pub all: bool,

    #[arg(long, help = "Chỉ quét máy in qua mạng LAN (mDNS & TCP 9100)")]
    pub network: bool,

    #[arg(long, help = "Chỉ quét máy in Bluetooth")]
    pub bluetooth: bool,
}

#[derive(Args, Debug)]
pub struct TestArgs {
    #[arg(long, help = "In hóa đơn nhiệt mẫu test cắt giấy & QR code (ESC/POS)")]
    pub receipt: bool,

    #[arg(long, help = "In tem mã vạch mẫu test (TSPL / ZPL)")]
    pub label: bool,

    #[arg(
        short,
        long,
        default_value = "network",
        help = "Loại kết nối mục tiêu (network, usb, bluetooth)"
    )]
    pub target: String,

    #[arg(
        short,
        long,
        help = "Địa chỉ máy in (ví dụ: 192.168.1.200:9100 hoặc usb://0416:5011)"
    )]
    pub address: Option<String>,
}

#[derive(Args, Debug)]
pub struct PrintArgs {
    #[arg(
        short,
        long,
        help = "Đường dẫn file JSON chứa cấu trúc tài liệu cần in"
    )]
    pub file: Option<PathBuf>,

    #[arg(
        long,
        help = "Đường dẫn file ảnh PNG/JPEG cần in (tự động xử lý dithering đơn sắc)"
    )]
    pub image: Option<PathBuf>,

    #[arg(
        long,
        help = "Chiều rộng ảnh mục tiêu (dots, ví dụ 384 cho K58, 576 cho K80)"
    )]
    pub image_width: Option<u32>,

    #[arg(long, help = "Chuỗi byte thô cần gửi trực tiếp tới máy in")]
    pub raw: Option<String>,

    #[arg(short, long, help = "Địa chỉ máy in mục tiêu")]
    pub address: Option<String>,
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: ConfigAction,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    #[command(about = "Hiển thị toàn bộ cấu hình hiện tại")]
    List,
    #[command(about = "Lấy giá trị của một khóa cấu hình")]
    Get { key: String },
    #[command(about = "Đặt giá trị cho một khóa cấu hình")]
    Set { key: String, value: String },
    #[command(about = "Đặt lại cấu hình về mặc định")]
    Reset,
}

#[derive(Args, Debug)]
pub struct StartupArgs {
    #[command(subcommand)]
    pub action: StartupAction,
}

#[derive(Subcommand, Debug)]
pub enum StartupAction {
    #[command(about = "Đăng ký ghitaprint.exe tự chạy khi khởi động Windows")]
    Install {
        #[arg(
            long,
            help = "Cài đặt dưới dạng Windows Service hệ thống thay vì Registry Run"
        )]
        service: bool,
    },
    #[command(about = "Hủy đăng ký tự khởi động cùng Windows")]
    Uninstall,
    #[command(about = "Kiểm tra trạng thái đăng ký tự khởi động")]
    Status,
}

#[derive(Args, Debug)]
pub struct DaemonArgs {
    #[arg(long, help = "Khởi chạy daemon chạy ngầm")]
    pub start: bool,

    #[arg(long, help = "Dừng daemon đang chạy ngầm")]
    pub stop: bool,

    #[arg(long, help = "Khởi động lại daemon")]
    pub restart: bool,

    #[arg(
        short,
        long,
        default_value_t = 9123,
        help = "Cổng lắng nghe của Local REST & WebSocket API (mặc định 9123)"
    )]
    pub port: u16,
}
