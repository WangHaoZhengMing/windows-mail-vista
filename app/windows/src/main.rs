#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    #[cfg(windows)]
    {
        let result = mail_windows::startup::native::run(
            &mail_windows::startup::native::raw_tail(),
            &mail_windows::startup::native::runtime_dir(),
            mail_windows::startup::native::show_command(),
        );
        let code = match result {
            Ok(code) => code,
            Err(error) => {
                let message: Vec<u16> =
                    format!("{}\n\nExit code: {}", error.detail, error.exit_code)
                        .encode_utf16()
                        .chain(Some(0))
                        .collect();
                let title: Vec<u16> = "Windows Mail Vista".encode_utf16().chain(Some(0)).collect();
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                        std::ptr::null_mut(),
                        message.as_ptr(),
                        title.as_ptr(),
                        windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
                    );
                }
                error.exit_code
            }
        };
        std::process::exit(code);
    }
    #[cfg(not(windows))]
    {
        eprintln!("WinMail requires Windows; use the entry point for your platform in app/.");
        std::process::exit(2);
    }
}
