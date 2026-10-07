use std::io;

#[cfg(windows)]
use winreg::RegKey;
#[cfg(windows)]
use winreg::enums::*;

const REG_RUN_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APP_NAME: &str = "GhitaPrint";

pub fn install_startup() -> io::Result<String> {
    #[cfg(windows)]
    {
        let exe_path = std::env::current_exe()?;
        let cmd = format!("\"{}\" daemon --silent", exe_path.display());

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(REG_RUN_PATH)?;
        key.set_value(APP_NAME, &cmd)?;

        Ok(cmd)
    }

    #[cfg(not(windows))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Tính năng tự khởi động chỉ hỗ trợ trên hệ điều hành Windows.",
        ))
    }
}

pub fn uninstall_startup() -> io::Result<()> {
    #[cfg(windows)]
    {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey_with_flags(REG_RUN_PATH, KEY_WRITE) {
            let _ = key.delete_value(APP_NAME);
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Tính năng tự khởi động chỉ hỗ trợ trên hệ điều hành Windows.",
        ))
    }
}

pub fn is_startup_enabled() -> bool {
    #[cfg(windows)]
    {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey_with_flags(REG_RUN_PATH, KEY_READ) {
            key.get_value::<String, _>(APP_NAME).is_ok()
        } else {
            false
        }
    }

    #[cfg(not(windows))]
    {
        false
    }
}

pub fn get_startup_command() -> Option<String> {
    #[cfg(windows)]
    {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey_with_flags(REG_RUN_PATH, KEY_READ) {
            key.get_value::<String, _>(APP_NAME).ok()
        } else {
            None
        }
    }

    #[cfg(not(windows))]
    {
        None
    }
}
