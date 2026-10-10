//! String conversions required at the Windows API boundary.

/// Encode a string as a zero-terminated UTF-16 buffer for a Windows API call.
pub fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

pub fn command_words(tail: &str) -> Vec<String> {
    if tail.trim().is_empty() {
        return Vec::new();
    }
    unsafe {
        let mut count = 0;
        let pointers = windows_sys::Win32::UI::Shell::CommandLineToArgvW(
            wide(&format!("WinMailRust.exe {tail}")).as_ptr(),
            &mut count,
        );
        if pointers.is_null() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for i in 1..count as usize {
            let pointer = *pointers.add(i);
            let mut length = 0;
            while *pointer.add(length) != 0 {
                length += 1;
            }
            out.push(String::from_utf16_lossy(std::slice::from_raw_parts(
                pointer, length,
            )));
        }
        windows_sys::Win32::Foundation::LocalFree(pointers.cast());
        out
    }
}
