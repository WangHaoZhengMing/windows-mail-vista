//! Shared control creation and text access for native windows.
//!
//! These handles are borrowed from their parent window, which owns their lifetime.
use super::strings::wide;
use std::ptr::null;
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::*, UI::WindowsAndMessaging::*,
};

/// Read the UTF-16 caption or edit contents of a live window.
///
/// # Safety
/// `window` must be a live window owned by the calling UI thread.
pub unsafe fn window_text(window: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(window).max(0) as usize;
        let mut buffer = vec![0; length + 1];
        let copied = GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }
}

/// Replace the UTF-16 caption or edit contents of a live window.
///
/// # Safety
/// `window` must be a live window owned by the calling UI thread. The caller must
/// allow for synchronous window messages triggered by changing the contents.
pub unsafe fn set_window_text(window: HWND, value: &str) {
    unsafe {
        SetWindowTextW(window, wide(value).as_ptr());
    }
}

/// Create a classic mail control, borrowed from `owner`.
///
/// # Safety
/// `owner` must be a live window owned by the calling UI thread.
pub unsafe fn control(owner: HWND, class: &str, caption: &str, style: u32, id: u16) -> HWND {
    let client_edge = matches!(class, "EDIT" | "SysTreeView32" | "SysListView32");
    unsafe { create_control(owner, class, caption, style, id, client_edge, 24) }
}

/// Create an account form control, preserving the form's original border policy.
///
/// # Safety
/// `owner` must be a live window owned by the calling UI thread.
pub unsafe fn account_control(
    owner: HWND,
    class: &str,
    caption: &str,
    style: u32,
    id: u16,
) -> HWND {
    let client_edge = matches!(class, "EDIT" | "LISTBOX");
    unsafe { create_control(owner, class, caption, style, id, client_edge, 25) }
}

unsafe fn create_control(
    owner: HWND,
    class: &str,
    caption: &str,
    style: u32,
    id: u16,
    client_edge: bool,
    height: i32,
) -> HWND {
    unsafe {
        let window = CreateWindowExW(
            if client_edge { WS_EX_CLIENTEDGE } else { 0 },
            wide(class).as_ptr(),
            wide(caption).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            100,
            height,
            owner,
            id as usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        SendMessageW(
            window,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT) as usize,
            1,
        );
        window
    }
}
