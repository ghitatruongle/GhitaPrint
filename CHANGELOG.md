# 📋 Changelog

Toàn bộ thay đổi đáng chú ý của **GhitaPrint** được ghi nhận trong file này.
Định dạng dựa trên [Keep a Changelog](https://keepachangelog.com/), phiên bản tuân theo [Semantic Versioning](https://semver.org/).

---

## [0.0.0] — 2026-10-11

### 🎉 Phát hành chính thức đầu tiên

Bản phát hành chính thức đầu tiên của **GhitaPrint — Universal, Driverless & Ultra-fast Print Engine** viết bằng Rust, sau chu kỳ `0.0.0-preview` với toàn bộ 4 Track phát triển đã hoàn thành.

#### Track 1 — CLI Framework, Configuration & Daemon Lifecycle
- Khung lệnh CLI phân cấp với Clap v4: `scan`, `test`, `print`, `config`, `startup`, `daemon`, `virtual-printer`, `status`.
- Quản lý cấu hình tự động qua `ghitaprint.toml` (`%APPDATA%\GhitaPrint\ghitaprint.toml`).
- Đăng ký/gỡ tự khởi động cùng Windows qua Registry `HKCU\...\Run`.
- Giao diện Terminal bo góc UTF-8, cờ `--json` toàn cục.

#### Track 2 — Driverless Hardware Transports & Device Discovery
- Network JetDirect Transport: raw TCP cổng 9100, tắt Nagle, port ping an toàn.
- Driverless USB Direct Engine: Bulk OUT Endpoint qua `nusb`, không cần cài driver hãng.
- Định danh URI thống nhất: `usb://<VID>:<PID>`, `tcp://<IP>:<PORT>`.
- Bộ quét thiết bị: `ghitaprint scan`, `ghitaprint scan --all` (chẩn đoán toàn bộ cổng USB bo mạch chủ).

#### Track 3 — Universal Protocol Encoders & Graphics CLI
- Universal Document Schema (UIR): tài liệu in JSON chuẩn hóa (`text`, `table`, `divider`, `qrcode`, `feed`, `cut`).
- ESC/POS Engine: mã nhị phân đầy đủ (font size, bold, align, QR code, cắt giấy).
- TSPL Label Engine: tem nhãn mã vạch (kích thước mm, Code 128, QR, khung `BOX`).
- Floyd-Steinberg Monochrome Rasterizer: ảnh màu → 1-bit siêu mịn (`GS v 0`).
- Lệnh in thực tế: `test --receipt`, `test --label`, `print -f bill.json`, `print --image logo.png`.

#### Track 4 — Virtual Printer, Local REST/WebSocket Server & Ecosystem
- Local HTTP REST & WebSocket Server (axum): `/api/v1/health`, `/api/v1/printers`, `/api/v1/print`, `/api/v1/ws`.
- Virtual Printer Bridge: máy in ảo "GhitaPrint" trên Windows Spooler (Microsoft IPP Class Driver).
- Điều khiển nền: `ghitaprint daemon --start --port 9123`.
- Single Binary tối ưu: ~1.92 MB (LTO, strip, abort panic), RAM 5–8 MB.

#### 🛠️ Sửa lỗi trong đợt review mã nguồn phát hành
- Bộ cài setup.exe nhúng nhầm bản `ghitaprint.exe` cũ khi build song song (`include_bytes!` đọc file chưa được ghi xong) — bổ sung `build.rs` theo dõi `target/release/ghitaprint.exe`, buộc build 2 bước đúng thứ tự.
- `startup install` ghi Registry lệnh `daemon --silent` (flag không tồn tại) khiến daemon không thể tự chạy khi bật máy — đổi thành `daemon --start`.
- ESC/POS `barcode_128` tràn `u8` khi dữ liệu > 253 byte và `qrcode` tràn `u16` khi > 65.532 byte — giới hạn dữ liệu an toàn.
- Bảng trong JSON Document bỏ qua thuộc tính `align` của cột — hỗ trợ đầy đủ left/center/right.
- Gửi dữ liệu qua TCP không có timeout — request daemon có thể treo vĩnh viễn nếu máy in ngắt giữa chừng.
- Lệnh `status` hiển thị hardcode cổng 9123 thay vì `api_port` cấu hình thực tế.
- Scheme URI `tcp://<IP>:<PORT>` (chuẩn trong tài liệu kiến trúc) không được xử lý khi kết nối TCP — kết nối luôn thất bại; giờ được tự động strip prefix.
- Luồng `print --image` thiếu lệnh khởi tạo `ESC @` đầu dữ liệu — bổ sung để máy in luôn bắt đầu ở trạng thái mặc định.

#### 📦 Tải xuống
- Bộ cài 1-Click: `GhitaPrint_0.0.0.exe` — cài thường / `--silent` / `--uninstall`, tự thêm PATH, đăng ký tự khởi động, cài máy in ảo và chạy daemon.
- SHA-256: `2531dc5c39b74ada297aedae6f45ed00ddda02b988a0479e4e942d373eabb97d`

#### ✅ Chất lượng
- 23 automated tests: 23 passed, 0 failed, 0 warnings.
- File bộ cài phát hành qua **GitHub Releases** và không còn được track trong mã nguồn git.
