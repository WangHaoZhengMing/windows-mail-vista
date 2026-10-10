//! Per-window DPI resources. Dimensions elsewhere are in 96-DPI logical pixels.
use super::strings::wide;
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

pub fn scale(value: i32, dpi: u32) -> i32 {
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}
pub fn logical(value: i32, dpi: u32) -> i32 {
    ((value as i64 * 96 + dpi as i64 / 2) / dpi.max(1) as i64) as i32
}
/// Read a window's DPI, falling back to the 96-DPI logical coordinate scale.
///
/// # Safety
/// `hwnd` must be a live window on the calling UI thread.
pub unsafe fn window_dpi(hwnd: HWND) -> u32 {
    unsafe { GetDpiForWindow(hwnd).max(96) }
}
pub struct ThreadContext(DPI_AWARENESS_CONTEXT);
impl ThreadContext {
    pub fn enter() -> Self {
        unsafe {
            Self(SetThreadDpiAwarenessContext(
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            ))
        }
    }
}
impl Drop for ThreadContext {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                SetThreadDpiAwarenessContext(self.0);
            }
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FontWeight {
    #[default]
    Regular,
    Bold,
}
pub struct Resources {
    pub dpi: u32,
    font: HFONT,
    bold_font: HFONT,
    small: HICON,
    large: HICON,
}
impl Default for Resources {
    fn default() -> Self {
        Self {
            dpi: 96,
            font: null_mut(),
            bold_font: null_mut(),
            small: null_mut(),
            large: null_mut(),
        }
    }
}
impl Resources {
    /// Apply this DPI resource set's regular or bold font to one control.
    /// Bold uses the same face and logical size and is allocated only on demand.
    ///
    /// # Safety
    /// `control` must be a live child of the window last passed to `update`, on
    /// the calling UI thread. Keep these resources alive until the control is
    /// destroyed or another font is applied. Allow synchronous message reentry.
    pub unsafe fn apply_font(&mut self, control: HWND, weight: FontWeight) {
        unsafe {
            let font = match weight {
                FontWeight::Regular => self.font,
                FontWeight::Bold => {
                    if self.bold_font.is_null() {
                        self.bold_font = create_font(self.dpi, FontWeight::Bold);
                    }
                    if self.bold_font.is_null() {
                        self.font
                    } else {
                        self.bold_font
                    }
                }
            };
            if !font.is_null() {
                SendMessageW(control, WM_SETFONT, font as usize, 1);
            }
        }
    }
    /// Replace owned fonts and icons and apply them to this window's controls.
    ///
    /// # Safety
    /// `hwnd` and its children must be live windows on the calling UI thread.
    /// Allow synchronous message reentry during the operation; keep this resource
    /// set alive until those windows no longer use its fonts and icons.
    pub unsafe fn update(&mut self, hwnd: HWND, dpi: u32) {
        unsafe {
            self.dpi = dpi.max(96);
            let font = create_font(self.dpi, FontWeight::Regular);
            if !font.is_null() {
                // Detach every child from both old fonts before deleting them.
                // Callers can then opt individual labels back into bold.
                EnumChildWindows(hwnd, Some(apply_regular_font), font as isize);
                if !self.font.is_null() {
                    DeleteObject(self.font);
                }
                self.font = font;
                if !self.bold_font.is_null() {
                    DeleteObject(self.bold_font);
                    self.bold_font = null_mut();
                }
            }
            let small = super::icons::icon(scale(16, self.dpi));
            let large = super::icons::icon(scale(32, self.dpi));
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small as isize);
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, large as isize);
            if !self.small.is_null() {
                DestroyIcon(self.small);
            }
            if !self.large.is_null() {
                DestroyIcon(self.large);
            }
            self.small = small;
            self.large = large;
        }
    }
}
unsafe fn create_font(dpi: u32, weight: FontWeight) -> HFONT {
    unsafe {
        CreateFontW(
            -scale(12, dpi),
            0,
            0,
            0,
            match weight {
                FontWeight::Regular => FW_NORMAL,
                FontWeight::Bold => FW_BOLD,
            } as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            wide("Segoe UI").as_ptr(),
        )
    }
}
unsafe extern "system" fn apply_regular_font(hwnd: HWND, font: isize) -> i32 {
    unsafe {
        SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
    }
    1
}
impl Drop for Resources {
    fn drop(&mut self) {
        unsafe {
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
            if !self.bold_font.is_null() {
                DeleteObject(self.bold_font);
            }
            if !self.small.is_null() {
                DestroyIcon(self.small);
            }
            if !self.large.is_null() {
                DestroyIcon(self.large);
            }
        }
    }
}
/// Apply the suggested rectangle from `WM_DPICHANGED`.
///
/// # Safety
/// `hwnd` must be a live window on the calling UI thread, and `lp` must point to
/// the valid `RECT` supplied by the current `WM_DPICHANGED` message. Allow
/// synchronous window message reentry during the position update.
pub unsafe fn suggested(hwnd: HWND, lp: isize) {
    unsafe {
        let rect = &*(lp as *const RECT);
        SetWindowPos(
            hwnd,
            null_mut(),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}
/// Move a borrowed child control using 96-DPI logical coordinates.
///
/// # Safety
/// `hwnd` must be a live window on the calling UI thread. Allow synchronous
/// window message reentry while moving or repainting the control.
pub unsafe fn move_control(
    dpi: u32,
    hwnd: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    repaint: i32,
) {
    unsafe {
        MoveWindow(
            hwnd,
            scale(x, dpi),
            scale(y, dpi),
            scale(width, dpi),
            scale(height, dpi),
            repaint,
        );
    }
}

#[cfg(test)]
mod tests;
