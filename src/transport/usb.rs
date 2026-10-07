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
        format!("usb://{:04x}:{:04x}", self.vendor_id, self.product_id)
    }
}

const KNOWN_PRINTER_VIDS: &[u16] = &[
    0x0416, 0x1fc9, 0x04b8, 0x0a5f, 0x04a9, 0x03f0, 0x04f9, 0x1203, 0x0dd4, 0x0fe6, 0x1504, 0x0519,
    0x20d1, 0x0684,
];

pub fn scan_usb_printers() -> Result<Vec<UsbPrinterDevice>, TransportError> {
    let mut printers = Vec::new();

    let devices = nusb::list_devices()
        .map_err(|e| TransportError::Usb(format!("Không thể liệt kê thiết bị USB: {}", e)))?;

    for dev in devices {
        let vid = dev.vendor_id();
        let pid = dev.product_id();

        let has_printer_class = dev.interfaces().any(|i| i.class() == 0x07);
        let is_known_vendor = KNOWN_PRINTER_VIDS.contains(&vid);

        let name_matches = if !has_printer_class && !is_known_vendor {
            let prod_raw = dev.product_string().unwrap_or("");
            let mfg_raw = dev.manufacturer_string().unwrap_or("");
            let prod_lower = prod_raw.to_lowercase();
            let mfg_lower = mfg_raw.to_lowercase();
            prod_lower.contains("print")
                || prod_lower.contains("pos")
                || prod_lower.contains("receipt")
                || mfg_lower.contains("print")
                || mfg_lower.contains("epson")
                || mfg_lower.contains("xprinter")
                || mfg_lower.contains("zebra")
        } else {
            false
        };

        if has_printer_class || is_known_vendor || name_matches {
            let mfg = dev.manufacturer_string().unwrap_or("Không rõ").to_string();
            let prod = dev.product_string().unwrap_or("Thiết bị USB").to_string();
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
}

impl UsbTransport {
    pub fn parse_uri(uri: &str) -> Option<(u16, u16)> {
        let clean = uri.strip_prefix("usb://").unwrap_or(uri);
        let parts: Vec<&str> = clean.split(':').collect();
        if parts.len() == 2 {
            let vid = u16::from_str_radix(parts[0], 16).ok()?;
            let pid = u16::from_str_radix(parts[1], 16).ok()?;
            Some((vid, pid))
        } else {
            None
        }
    }

    pub fn open(vendor_id: u16, product_id: u16) -> Result<Self, TransportError> {
        let dev_info = nusb::list_devices()
            .map_err(|e| TransportError::Usb(e.to_string()))?
            .find(|d| d.vendor_id() == vendor_id && d.product_id() == product_id)
            .ok_or_else(|| {
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
            TransportError::Usb(format!(
                "Không thể claim interface {}: {}",
                target_interface, e
            ))
        })?;

        let out_endpoint = 0x01;

        Ok(Self {
            interface,
            out_endpoint,
        })
    }

    pub fn open_uri(uri: &str) -> Result<Self, TransportError> {
        let (vid, pid) = Self::parse_uri(uri).ok_or_else(|| {
            TransportError::Usb(format!(
                "Định dạng URI USB không hợp lệ (mẫu: usb://0416:5011): {}",
                uri
            ))
        })?;
        Self::open(vid, pid)
    }
}

impl PrinterTransport for UsbTransport {
    async fn send_raw(&mut self, data: &[u8]) -> Result<(), TransportError> {
        const CHUNK_SIZE: usize = 16 * 1024;
        for chunk in data.chunks(CHUNK_SIZE) {
            let completion = self
                .interface
                .bulk_out(self.out_endpoint, chunk.to_vec())
                .await;
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
            Some((0x0416, 0x5011))
        );
        assert_eq!(UsbTransport::parse_uri("04b8:0202"), Some((0x04b8, 0x0202)));
        assert_eq!(UsbTransport::parse_uri("usb://invalid"), None);
        assert_eq!(UsbTransport::parse_uri("123"), None);
    }
}
