pub mod process;
pub mod startup;

#[allow(unused_imports)]
pub use process::{probe_health_sync, spawn_detached};
#[allow(unused_imports)]
pub use startup::{
    get_startup_command, get_watch_startup_command, install_startup, install_watch_startup,
    is_startup_enabled, is_watch_startup_enabled, uninstall_startup, uninstall_watch_startup,
};
