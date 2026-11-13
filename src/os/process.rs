use std::path::Path;

pub fn quote_command_line_argument(argument: &str) -> String {
    if !argument.is_empty()
        && !argument.contains([' ', '\t', '\n', '"'])
        && !argument.ends_with('\\')
    {
        return argument.to_string();
    }
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    let mut pending_backslashes = 0usize;
    for character in argument.chars() {
        match character {
            '\\' => {
                pending_backslashes += 1;
            }
            '"' => {
                let escapes = pending_backslashes * 2 + 1;
                quoted.push_str(&"\\".repeat(escapes));
                pending_backslashes = 0;
                quoted.push('"');
            }
            _ => {
                quoted.push_str(&"\\".repeat(pending_backslashes));
                pending_backslashes = 0;
                quoted.push(character);
            }
        }
    }
    quoted.push_str(&"\\".repeat(pending_backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(windows)]
pub fn process_image_name(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        PROCESS_QUERY_LIMITED_INFORMATION, OpenProcess, QueryFullProcessImageNameW,
    };

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length)
    };
    unsafe {
        let _ = CloseHandle(handle);
    }
    if ok == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..length as usize]))
}

#[cfg(not(windows))]
pub fn process_image_name(_pid: u32) -> Option<String> {
    None
}

#[cfg(windows)]
pub fn spawn_detached(exe_path: &Path, arguments: &[&str]) -> Result<u32, String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CreateProcessW, PROCESS_INFORMATION,
        STARTUPINFOW,
    };

    let mut command_line: Vec<u16> = OsStr::new(&quote_command_line_argument(
        &exe_path.display().to_string(),
    ))
    .encode_wide()
    .collect();
    for argument in arguments {
        command_line.push(b' ' as u16);
        command_line.extend(OsStr::new(&quote_command_line_argument(argument)).encode_wide());
    }
    command_line.push(0);

    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut info,
        )
    };

    if created == 0 {
        return Err(format!(
            "Không thể khởi chạy '{}' (CreateProcessW thất bại, mã lỗi {})",
            exe_path.display(),
            std::io::Error::last_os_error()
        ));
    }

    let pid = info.dwProcessId;
    unsafe {
        let _ = CloseHandle(info.hProcess);
        let _ = CloseHandle(info.hThread);
    }
    Ok(pid)
}

#[cfg(not(windows))]
pub fn spawn_detached(_exe_path: &Path, _arguments: &[&str]) -> Result<u32, String> {
    Err("Tính năng spawn nền chỉ hỗ trợ trên Windows.".to_string())
}

pub fn probe_health_sync(port: u16, attempts: u32) -> Option<String> {
    use std::io::{Read, Write};
    use std::net::SocketAddr;
    use std::net::TcpStream;
    use std::time::Duration;

    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().ok()?;

    for _ in 0..attempts {
        if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
            let request = "GET /api/v1/health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n";
            if stream.write_all(request.as_bytes()).is_ok() {
                let mut buffer = [0u8; 2048];
                if let Ok(read_bytes) = stream.read(&mut buffer) {
                    let text = String::from_utf8_lossy(&buffer[..read_bytes]).to_string();
                    if text.starts_with("HTTP/1.1 200") || text.starts_with("HTTP/1.0 200") {
                        if let Some(body_start) = text.find('{')
                            && let Ok(json) =
                                serde_json::from_str::<serde_json::Value>(&text[body_start..])
                        {
                            return json["version"].as_str().map(str::to_string);
                        }
                        return Some(env!("CARGO_PKG_VERSION").to_string());
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_argument_roundtrip_simple_and_tricky() {
        assert_eq!(quote_command_line_argument("daemon"), "daemon");
        assert_eq!(
            quote_command_line_argument("C:\\đường dẫn có dấu cách\\x.exe"),
            "\"C:\\đường dẫn có dấu cách\\x.exe\""
        );
        assert_eq!(
            quote_command_line_argument("ghi\"chép"),
            "\"ghi\\\"chép\""
        );
        assert_eq!(quote_command_line_argument("C:\\folder\\"), "\"C:\\folder\\\\\"");
    }

    #[cfg(windows)]
    #[test]
    fn test_process_image_name_resolves_current_process() {
        let expected = std::env::current_exe()
            .unwrap()
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let actual = process_image_name(std::process::id())
            .map(|full| std::path::Path::new(&full)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default())
            .unwrap_or_default();
        assert_eq!(actual, expected, "tên image của tiến trình hiện tại phải khớp");
    }

    #[test]
    fn test_probe_health_sync_unreachable_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let free_port = listener.local_addr().unwrap().port();
        drop(listener);

        assert!(probe_health_sync(free_port, 1).is_none());
    }

    #[test]
    fn test_probe_health_sync_against_live_health_endpoint() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let version = runtime.block_on(async {
            let state = crate::server::AppState::new(
                String::new(),
                5,
                String::new(),
                "auto".to_string(),
                "escpos".to_string(),
            );
            let app = crate::server::create_router(state);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move { axum::serve(listener, app).await });
            tokio::task::spawn_blocking(move || probe_health_sync(port, 3))
                .await
                .expect("probe task phải chạy được")
        });

        assert_eq!(
            version.expect("health probe phải thành công"),
            env!("CARGO_PKG_VERSION")
        );
    }
}
