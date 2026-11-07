use super::{PrinterTransport, TransportError};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct UsbPrinterDevice {
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: String,
    pub product: String,
    pub serial_number: Option<String>,
    pub is_printer: bool,
}

impl UsbPrinterDevice {
    pub fn uri(&self) -> String {
        let base = format!("usb://{:04x}:{:04x}", self.vendor_id, self.product_id);
        match &self.serial_number {
            Some(serial) if !serial.is_empty() => format!("{}?serial={}", base, serial),
            _ => base,
        }
    }
}

const KNOWN_PRINTER_VIDS: &[u16] = &[
    0x0416, 0x1fc9, 0x04b8, 0x0a5f, 0x04a9, 0x03f0, 0x04f9, 0x1203, 0x0dd4, 0x0fe6, 0x1504, 0x0519,
    0x20d1, 0x0684,
];

pub fn classify_device(
    vendor_id: u16,
    has_printer_class: bool,
    product: Option<&str>,
    manufacturer: Option<&str>,
) -> bool {
    if has_printer_class {
        return true;
    }

    let name_hints = || {
        let prod = product.unwrap_or("").to_lowercase();
        let mfg = manufacturer.unwrap_or("").to_lowercase();
        prod.contains("print")
            || prod.contains("pos")
            || prod.contains("receipt")
            || mfg.contains("print")
            || mfg.contains("epson")
            || mfg.contains("xprinter")
            || mfg.contains("zebra")
    };

    if KNOWN_PRINTER_VIDS.contains(&vendor_id) {
        return name_hints();
    }
    name_hints()
}

pub fn scan_usb_printers() -> Result<Vec<UsbPrinterDevice>, TransportError> {
    let mut printers = Vec::new();

    let devices = nusb::list_devices()
        .map_err(|e| TransportError::Usb(format!("Không thể liệt kê thiết bị USB: {}", e)))?;

    for dev in devices {
        let vid = dev.vendor_id();
        let pid = dev.product_id();

        let has_printer_class = dev.interfaces().any(|i| i.class() == 0x07);
        let product = dev.product_string();
        let manufacturer = dev.manufacturer_string();

        if classify_device(vid, has_printer_class, product, manufacturer) {
            let mfg = manufacturer.unwrap_or("Không rõ").to_string();
            let prod = product.unwrap_or("Thiết bị USB").to_string();
            let serial = dev.serial_number().map(|s| s.to_string());

            printers.push(UsbPrinterDevice {
                vendor_id: vid,
                product_id: pid,
                manufacturer: mfg,
                product: prod,
                serial_number: serial,
                is_printer: true,
            });
        }
    }

    Ok(printers)
}

#[allow(dead_code)]
pub fn list_all_usb_devices() -> Result<Vec<UsbPrinterDevice>, TransportError> {
    let mut all = Vec::new();

    let devices = nusb::list_devices()
        .map_err(|e| TransportError::Usb(format!("Không thể liệt kê thiết bị USB: {}", e)))?;

    for dev in devices {
        let vid = dev.vendor_id();
        let pid = dev.product_id();
        let mfg = dev.manufacturer_string().unwrap_or("Unknown").to_string();
        let prod = dev.product_string().unwrap_or("USB Device").to_string();
        let serial = dev.serial_number().map(|s| s.to_string());

        all.push(UsbPrinterDevice {
            vendor_id: vid,
            product_id: pid,
            manufacturer: mfg,
            product: prod,
            serial_number: serial,
            is_printer: false,
        });
    }

    Ok(all)
}

pub struct UsbTransport {
    interface: nusb::Interface,
    out_endpoint: u8,
    pub timeout_secs: u64,
}

impl UsbTransport {
    pub fn parse_uri(uri: &str) -> Option<(u16, u16, Option<String>)> {
        let clean = uri.strip_prefix("usb://").unwrap_or(uri);
        let (device_part, serial) = match clean.split_once('?') {
            Some((dev, query)) => {
                let serial = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("serial="))
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                (dev, serial)
            }
            None => (clean, None),
        };
        let parts: Vec<&str> = device_part.split(':').collect();
        if parts.len() == 2 {
            let vid = u16::from_str_radix(parts[0], 16).ok()?;
            let pid = u16::from_str_radix(parts[1], 16).ok()?;
            Some((vid, pid, serial))
        } else {
            None
        }
    }

    pub fn open(
        vendor_id: u16,
        product_id: u16,
        serial: Option<&str>,
    ) -> Result<Self, TransportError> {
        let mut matches: Vec<_> = nusb::list_devices()
            .map_err(|e| TransportError::Usb(e.to_string()))?
            .filter(|d| d.vendor_id() == vendor_id && d.product_id() == product_id)
            .collect();

        if let Some(expected) = serial {
            matches.retain(|d| d.serial_number().map(|s| s == expected).unwrap_or(false));
            if matches.is_empty() {
                return Err(TransportError::NotFound(format!(
                    "usb://{:04x}:{:04x}?serial={} — không tìm thấy thiết bị khớp serial này",
                    vendor_id, product_id, expected
                )));
            }
        } else if matches.len() > 1 {
            let serials: Vec<String> = matches
                .iter()
                .map(|d| {
                    d.serial_number()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "(không có)".to_string())
                })
                .collect();
            return Err(TransportError::Usb(format!(
                "Phát hiện {} máy in cùng VID:PID {:04x}:{:04x} — hãy chỉ định serial: usb://{:04x}:{:04x}?serial=<serial>. Serial hiện có: {}",
                matches.len(),
                vendor_id,
                product_id,
                vendor_id,
                product_id,
                serials.join(", ")
            )));
        }

        let dev_info = matches.into_iter().next().ok_or_else(|| {
            TransportError::NotFound(format!("usb://{:04x}:{:04x}", vendor_id, product_id))
        })?;

        let mut target_interface = 0u8;
        for iface in dev_info.interfaces() {
            if iface.class() == 0x07 {
                target_interface = iface.interface_number();
                break;
            }
        }

        let device = dev_info
            .open()
            .map_err(|e| TransportError::Usb(format!("Không thể mở thiết bị USB: {}", e)))?;

        let interface = device.claim_interface(target_interface).map_err(|e| {
            let detail = e.to_string();
            if detail.to_lowercase().contains("access")
                || detail.to_lowercase().contains("denied")
                || detail.to_lowercase().contains("busy")
            {
                TransportError::Usb(format!(
                    "Không claim được interface {} — máy in đang bị driver khác của Windows chiếm (usbprint.sys). GhitaPrint cần driver WinUSB: cài qua Zadig (zadig.akeo.ie) chọn máy in rồi chọn 'WinUSB', hoặc dùng kết nối mạng TCP 9100. Chi tiết: {}",
                    target_interface, detail
                ))
            } else {
                TransportError::Usb(format!(
                    "Không thể claim interface {}: {}",
                    target_interface, detail
                ))
            }
        })?;

        let out_endpoint = interface.descriptors().find_map(|alt| {
            alt.endpoints()
                .find(|ep| {
                    ep.direction() == nusb::transfer::Direction::Out
                        && ep.transfer_type() == nusb::transfer::EndpointType::Bulk
                })
                .map(|ep| ep.address())
        });

        let Some(out_endpoint) = out_endpoint else {
            return Err(TransportError::Usb(
                "Thiết bị USB không có endpoint Bulk OUT — không thể in qua chế độ USB Direct. Hãy kiểm tra lại máy in hoặc dùng kết nối mạng TCP 9100".to_string(),
            ));
        };

        Ok(Self {
            interface,
            out_endpoint,
            timeout_secs: 60,
        })
    }

    pub fn open_uri(uri: &str) -> Result<Self, TransportError> {
        let (vid, pid, serial) = Self::parse_uri(uri).ok_or_else(|| {
            TransportError::Usb(format!(
                "Định dạng URI USB không hợp lệ (mẫu: usb://0416:5011 hoặc usb://0416:5011?serial=ABC): {}",
                uri
            ))
        })?;
        Self::open(vid, pid, serial.as_deref())
    }
}

impl PrinterTransport for UsbTransport {
    async fn send_raw(&mut self, data: &[u8]) -> Result<(), TransportError> {
        const CHUNK_SIZE: usize = 16 * 1024;
        let per_chunk_timeout = std::time::Duration::from_secs(self.timeout_secs.max(30));
        for chunk in data.chunks(CHUNK_SIZE) {
            let transfer = self.interface.bulk_out(self.out_endpoint, chunk.to_vec());
            let completion = tokio::time::timeout(per_chunk_timeout, transfer)
                .await
                .map_err(|_| {
                    TransportError::Usb(format!(
                        "Máy in USB không phản hồi sau {} giây (chunk {} bytes) — máy có thể đang lỗi, hết giấy hoặc cáp lỏng",
                        per_chunk_timeout.as_secs(),
                        chunk.len()
                    ))
                })?;
            if let Err(e) = completion.status {
                return Err(TransportError::Usb(format!("Lỗi ghi USB Bulk Out: {}", e)));
            }
        }
        Ok(())
    }

    async fn is_alive(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_uri() {
        assert_eq!(
            UsbTransport::parse_uri("usb://0416:5011"),
            Some((0x0416, 0x5011, None))
        );
        assert_eq!(
            UsbTransport::parse_uri("04b8:0202"),
            Some((0x04b8, 0x0202, None))
        );
        assert_eq!(
            UsbTransport::parse_uri("usb://0416:5011?serial=ABC123"),
            Some((0x0416, 0x5011, Some("ABC123".to_string())))
        );
        assert_eq!(UsbTransport::parse_uri("usb://invalid"), None);
        assert_eq!(UsbTransport::parse_uri("123"), None);
    }

    #[test]
    fn test_classify_device_requires_class_or_name_hint() {
        assert!(classify_device(
            0x0416,
            true,
            Some("Thermal"),
            Some("Xprinter")
        ));
        assert!(classify_device(0x0416, false, Some("POS Printer"), None));
        assert!(!classify_device(
            0x0416,
            false,
            Some("Mouse"),
            Some("Logitech")
        ));
        assert!(!classify_device(
            0x03f0,
            false,
            Some("OfficeJet Mouse"),
            None
        ));
        assert!(classify_device(
            0x03f0,
            false,
            Some("HP LaserJet Printer"),
            None
        ));
        assert!(!classify_device(0x1234, false, None, None));
        assert!(classify_device(0x1234, false, Some("Receipt POS"), None));
    }

    #[test]
    fn test_uri_includes_serial_when_present() {
        let dev = UsbPrinterDevice {
            vendor_id: 0x0416,
            product_id: 0x5011,
            manufacturer: "X".to_string(),
            product: "Y".to_string(),
            serial_number: Some("SN001".to_string()),
            is_printer: true,
        };
        assert_eq!(dev.uri(), "usb://0416:5011?serial=SN001");
        let no_serial = UsbPrinterDevice {
            serial_number: None,
            ..dev
        };
        assert_eq!(no_serial.uri(), "usb://0416:5011");
    }
}
