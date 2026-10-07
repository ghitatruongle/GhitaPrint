# 🖨️ GhitaPrint (Universal Driverless Rust Print Engine)

> **Universal, Driverless & Ultra-fast Print Engine written in Rust.**  
> Một giải pháp in ấn hợp nhất, kết nối mọi loại máy in (POS nhiệt, tem nhãn mã vạch, văn phòng) mà không cần cài đặt driver thủ công, file thực thi siêu nhỏ gọn (< 2MB) và tiêu tốn cực ít RAM (5–8MB).

---

## 🎯 1. Bối cảnh & Vấn đề giải quyết

1. **Không cần cài driver hãng:** Kết nối trực tiếp máy in USB (qua Bulk OUT Endpoint với `nusb`), máy in mạng (TCP 9100 JetDirect) và máy in ảo Windows (Microsoft IPP Class Driver) mà không cần cài driver từ đĩa/hãng (Xprinter, Epson, Zebra, Brother, Canon...).
2. **Siêu nhẹ & Tốc độ cao:** 
   * Viết bằng Rust thuần, Single Binary **chỉ 1.89 MB** (nhỏ gấp 50 lần so với Java/Electron).
   * Tiêu thụ chỉ **5–8 MB RAM** khi chạy nền.
   * Khởi động tức thì trong **0.05 giây**.
3. **100% Điều khiển qua CLI (CLI-First):** Quét cổng, cấu hình, in thử nghiệm, bật tự khởi động cùng Windows, và quản lý máy in ảo hoàn toàn bằng dòng lệnh terminal.

---

## ⌨️ 2. Sổ tay câu lệnh CLI (`ghitaprint.exe`)

```bash
# 1. Quét thiết bị máy in thực tế (USB & Mạng LAN)
ghitaprint scan                 # Quét máy in USB và máy in mạng
ghitaprint scan --all           # Chẩn đoán toàn bộ cổng USB cắm trên bo mạch chủ
ghitaprint --json scan          # Xuất kết quả dạng JSON chuẩn

# 2. In thử nghiệm nhanh (Testing)
ghitaprint test --receipt       # In hóa đơn nhiệt mẫu test cắt giấy & mã QR (ESC/POS)
ghitaprint test --label         # In tem nhãn mã vạch mẫu test (TSPL / ZPL)
ghitaprint test --receipt --address 192.168.1.200:9100 # In tới máy in mạng cụ thể

# 3. In tài liệu thực tế (Document / Image / Raw)
ghitaprint print -f bill.json   # In tài liệu từ file cấu trúc JSON
ghitaprint print --image logo.png # In ảnh với thuật toán Floyd-Steinberg dithering 1-bit
ghitaprint print --raw "ESC @..." # Gửi chuỗi byte thô ra máy in

# 4. Quản lý cấu hình (Config)
ghitaprint config list          # Xem cấu hình hiện tại (tại %APPDATA%\GhitaPrint\ghitaprint.toml)
ghitaprint config set default_printer "usb://0416:5011" # Đặt máy in mặc định
ghitaprint config set api_port 9123 # Đổi cổng Local REST & WebSocket API

# 5. Tự khởi động cùng Windows (Startup)
ghitaprint startup install      # Tự động đăng ký ghitaprint.exe chạy ngầm khi Windows bật máy
ghitaprint startup status       # Kiểm tra trạng thái đăng ký Registry Run
ghitaprint startup uninstall    # Gỡ bỏ tự khởi động

# 6. Quản lý Máy in ảo Windows (Virtual Printer Bridge)
ghitaprint virtual-printer install   # Đăng ký máy in ảo "GhitaPrint" vào Windows Spooler
ghitaprint virtual-printer status    # Kiểm tra trạng thái máy in ảo
ghitaprint virtual-printer uninstall # Gỡ bỏ máy in ảo

# 7. Khởi chạy Local REST & WebSocket Server (Daemon)
ghitaprint daemon --start --port 9123
```

---

## 🏛️ 3. Kiến trúc phân tầng & Giao thức

```
┌────────────────────────────────────────────────────────┐
│     Client Layer (Web POS, Word/Chrome, CLI, AI)       │
└───────────────────────────┬────────────────────────────┘
                            │ (JSON / HTTP / WebSocket / Ctrl+P)
┌───────────────────────────▼────────────────────────────┐
│         GhitaPrint Core Engine & Axum Server           │
├──────────────────────────┬─────────────────────────────┤
│      Encoders & Parser   │     Connection Transport    │
│  • ESC/POS Builder       │  • Raw TCP (Socket 9100)    │
│  • TSPL / ZPL Builder    │  • Driverless USB (nusb)    │
│  • Floyd-Steinberg Dither│  • Virtual IPP Print Spooler│
│  • Document JSON Schema  │  • Bluetooth SPP / BLE      │
└──────────────────────────┴─────────────────────────────┘
```

---

## 🧪 4. Kiểm thử tự động (Unit Tests)

Dự án bao gồm bộ 21 automated tests kiểm tra toàn bộ luồng truyền nhận, mã hóa và máy chủ API:

```bash
cargo test
```

Kết quả: **21 passed, 0 failed, 0 warnings**.

---

## 📦 5. Bộ Cài Đặt 1-Click Duy Nhất (`release/GhitaPrint-Setup.exe`)

Thư mục `release/` chứa duy nhất file cài đặt độc lập:

* **File cài đặt:** `release/GhitaPrint-Setup.exe` (~2.1 MB)
* **Tự động hóa hoàn toàn:**
  1. Trích xuất file thực thi `ghitaprint.exe` vào `%LocalAppData%\Programs\GhitaPrint`.
  2. Tự động thêm vào biến môi trường **PATH** để gõ lệnh `ghitaprint` ở mọi terminal.
  3. Đăng ký **Tự khởi động cùng Windows** (`HKCU\...\Run`).
  4. Cài đặt **Máy in ảo Windows** (Microsoft IPP Class Driver).
  5. Khởi chạy **Daemon nền** kết nối sẵn sàng.

```bash
# Cài đặt thông thường (hiện giao diện CLI đẹp mắt)
.\release\GhitaPrint-Setup.exe

# Cài đặt ẩn tự động (Silent / Unattended mode)
.\release\GhitaPrint-Setup.exe --silent

# Gỡ cài đặt sạch sẽ khỏi máy tính
.\release\GhitaPrint-Setup.exe --uninstall
```

### Cách build lại bộ cài đặt:
```bash
cargo build --release --bin ghitaprint
cargo build --release --bin setup
Copy-Item target\release\setup.exe release\GhitaPrint-Setup.exe -Force
```

