//! Shared native light/dark palette. All handles remain owned by Windows.
use super::strings::wide;
use std::{
    ptr::null,
    sync::atomic::{AtomicBool, Ordering},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::Threading::GetCurrentThreadId,
    UI::{Controls::*, Shell::*, WindowsAndMessaging::*},
};
static DARK: AtomicBool = AtomicBool::new(false);
pub fn is_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}
pub fn set_dark(value: bool) {
    DARK.store(value, Ordering::Relaxed);
}
pub fn surface(light: u32) -> u32 {
    if is_dark() { 0x242424 } else { light }
}
pub fn panel(light: u32) -> u32 {
    if is_dark() { 0x303030 } else { light }
}
pub fn text(light: u32) -> u32 {
    if is_dark() { 0xe8e8e8 } else { light }
}
pub fn edge(light: u32) -> u32 {
    if is_dark() { 0x505050 } else { light }
}
unsafe fn class(window: HWND) -> String {
    unsafe {
        let mut b = [0u16; 128];
        let n = GetClassNameW(window, b.as_mut_ptr(), 128);
        String::from_utf16_lossy(&b[..n.max(0) as usize])
    }
}
/// Apply to a window created on the calling UI thread.
pub unsafe fn apply(window: HWND) {
    unsafe {
        if window.is_null() {
            return;
        }
        let name = class(window);
        // Do not subclass browser-owned windows or change a mail document's colors.
        if !(name.starts_with("WinMail")
            || matches!(
                name.as_str(),
                "STATIC"
                    | "EDIT"
                    | "BUTTON"
                    | "LISTBOX"
                    | "COMBOBOX"
                    | "SysListView32"
                    | "SysTreeView32"
                    | "SysHeader32"
                    | "msctls_statusbar32"
                    | "RICHEDIT50W"
            ))
        {
            return;
        }
        SetWindowSubclass(window, Some(procedure), 0x574d5448, 0);
        let dark = is_dark();
        if matches!(
            name.as_str(),
            "BUTTON" | "SysListView32" | "SysTreeView32" | "SysHeader32" | "COMBOBOX"
        ) {
            SetWindowTheme(
                window,
                wide(if dark {
                    "DarkMode_Explorer"
                } else {
                    "Explorer"
                })
                .as_ptr(),
                null(),
            );
        }
        let bg = surface(GetSysColor(COLOR_WINDOW));
        let fg = text(GetSysColor(COLOR_WINDOWTEXT));
        if name == "SysListView32" {
            SendMessageW(window, LVM_SETBKCOLOR, 0, bg as isize);
            SendMessageW(window, LVM_SETTEXTBKCOLOR, 0, bg as isize);
            SendMessageW(window, LVM_SETTEXTCOLOR, 0, fg as isize);
        }
        if name == "SysTreeView32" {
            SendMessageW(window, TVM_SETBKCOLOR, 0, bg as isize);
            SendMessageW(window, TVM_SETTEXTCOLOR, 0, fg as isize);
        }
        // The composer is a document canvas: recoloring its default font would
        // change outgoing HTML. Keep the mail's authored colors intact.
        if GetAncestor(window, GA_ROOT) == window {
            let value = i32::from(dark);
            DwmSetWindowAttribute(window, 20, &value as *const _ as *const _, 4);
        }
        RedrawWindow(
            window,
            null(),
            std::ptr::null_mut(),
            RDW_INVALIDATE | RDW_ERASE | RDW_FRAME,
        );
    }
}
#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(
        window: HWND,
        attribute: u32,
        value: *const std::ffi::c_void,
        size: u32,
    ) -> i32;
}
unsafe extern "system" fn visit(window: HWND, _: isize) -> i32 {
    unsafe {
        apply(window);
    }
    1
}
unsafe extern "system" fn root(window: HWND, _: isize) -> i32 {
    unsafe {
        apply(window);
        EnumChildWindows(window, Some(visit), 0);
    }
    1
}
/// Refresh all app windows belonging to the UI thread, including open readers.
pub unsafe fn refresh() {
    unsafe {
        EnumThreadWindows(GetCurrentThreadId(), Some(root), 0);
    }
}
unsafe extern "system" fn procedure(
    window: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    _: usize,
) -> isize {
    unsafe {
        if is_dark() {
            match msg {
                WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
                    let dc = wp as HDC;
                    SetTextColor(dc, text(0));
                    SetBkColor(dc, surface(0));
                    SetDCBrushColor(dc, surface(0));
                    return GetStockObject(DC_BRUSH) as isize;
                }
                WM_ERASEBKGND => {
                    let mut r = RECT::default();
                    GetClientRect(window, &mut r);
                    SetDCBrushColor(wp as HDC, surface(0));
                    FillRect(wp as HDC, &r, GetStockObject(DC_BRUSH) as HBRUSH);
                    return 1;
                }
                _ => {}
            }
        }
        if msg == WM_NCDESTROY {
            RemoveWindowSubclass(window, Some(procedure), 0x574d5448);
        }
        DefSubclassProc(window, msg, wp, lp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_list_dark_colors_restore_to_system_colors() {
        unsafe {
            let previous = is_dark();
            let common = INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            InitCommonControlsEx(&common);
            let window = CreateWindowExW(
                0,
                wide("SysListView32").as_ptr(),
                wide("").as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                200,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                null(),
            );
            assert!(!window.is_null());
            set_dark(true);
            apply(window);
            assert_eq!(SendMessageW(window, LVM_GETBKCOLOR, 0, 0) as u32, 0x242424);
            assert_eq!(
                SendMessageW(window, LVM_GETTEXTCOLOR, 0, 0) as u32,
                0xe8e8e8
            );
            set_dark(false);
            apply(window);
            assert_eq!(
                SendMessageW(window, LVM_GETBKCOLOR, 0, 0) as u32,
                GetSysColor(COLOR_WINDOW)
            );
            DestroyWindow(window);
            set_dark(previous);
        }
    }
}
