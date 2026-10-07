pub mod routes;
pub mod virtual_printer;

pub use routes::{AppState, create_router};
pub use virtual_printer::{
    install_virtual_printer, is_virtual_printer_installed, uninstall_virtual_printer,
};

use std::net::SocketAddr;
use tokio::time::Instant;

pub async fn start_server(
    port: u16,
    default_printer: String,
    timeout_secs: u64,
) -> Result<(), std::io::Error> {
    let state = AppState {
        start_time: Instant::now(),
        default_printer,
        timeout_secs,
    };

    let app = create_router(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
