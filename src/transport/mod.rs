pub mod tcp;
pub mod usb;

#[allow(unused_imports)]
pub use tcp::TcpTransport;
#[allow(unused_imports)]
pub use usb::{UsbPrinterDevice, UsbTransport, list_all_usb_devices, scan_usb_printers};

use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum TransportError {
    #[error("Lỗi I/O mạng hoặc thiết bị: {0}")]
    Io(#[from] std::io::Error),

    #[error("Hết thời gian chờ kết nối tới máy in ({0} giây)")]
    Timeout(u64),

    #[error("Không tìm thấy thiết bị: {0}")]
    NotFound(String),

    #[error("Thiết bị chưa được kết nối")]
    NotConnected,

    #[error("Lỗi giao tiếp USB: {0}")]
    Usb(String),
}

#[allow(dead_code)]
pub trait PrinterTransport: Send + Sync {
    fn send_raw(
        &mut self,
        data: &[u8],
    ) -> impl std::future::Future<Output = Result<(), TransportError>> + Send;
    fn is_alive(&self) -> impl std::future::Future<Output = bool> + Send;
}
