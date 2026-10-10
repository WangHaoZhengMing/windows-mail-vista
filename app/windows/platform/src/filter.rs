//! Blue toolbar face for a native dropdown-list ComboBox.
//!
//! Selection, keyboard input, accessibility, and the popup remain owned by the
//! system ComboBox. This stateless subclass paints only its closed client face.
use super::{classic, dpi};
use std::{
    mem::size_of,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::Threading::GetCurrentThreadId,
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const SUBCLASS: usize = 0x574d_464c;
const WHITE: COLORREF = 0x00ff_ffff;
const BORDER: COLORREF = 0x00d5_b58e;
const MAX_CAPTION: usize = 32768;

/// Style an ordinary `CBS_DROPDOWNLIST` without changing its native behavior.
/// Calling this repeatedly updates the same subclass and allocates no resources.
/// Returns false for another control kind, another UI thread, or install failure.
///
/// # Safety
/// `window` must be a live control on the calling UI thread. The parent owns its
/// lifetime. Allow synchronous window message reentry while installing the style.
pub unsafe fn style_filter(window: HWND) -> bool {
    unsafe {
        if window.is_null() || GetWindowThreadProcessId(window, null_mut()) != GetCurrentThreadId()
        {
            return false;
        }
        let style = GetWindowLongPtrW(window, GWL_STYLE) as u32;
        let mut class = [0u16; 64];
        let copied = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
        if !String::from_utf16_lossy(&class[..copied.max(0) as usize])
            .eq_ignore_ascii_case("COMBOBOX")
            || style & 3 != CBS_DROPDOWNLIST as u32
            || style & (CBS_OWNERDRAWFIXED | CBS_OWNERDRAWVARIABLE) as u32 != 0
        {
            return false;
        }
        if SetWindowSubclass(window, Some(filter_proc), SUBCLASS, 0) == 0 {
            return false;
        }
        InvalidateRect(window, null(), 0);
        true
    }
}

unsafe extern "system" fn filter_proc(
    window: HWND,
    message: u32,
    wp: usize,
    lp: isize,
    _: usize,
    _: usize,
) -> isize {
    unsafe {
        match message {
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(window, &mut paint);
                paint_face(window, dc);
                EndPaint(window, &paint);
                return 0;
            }
            WM_PRINTCLIENT => {
                paint_face(window, wp as HDC);
                return 0;
            }
            WM_ERASEBKGND => return 1,
            WM_NCDESTROY => {
                RemoveWindowSubclass(window, Some(filter_proc), SUBCLASS);
            }
            _ => (),
        }
        // Native messages still perform selection, notification, dropdown, and
        // dialog keyboard handling. Paint afterwards using their current state.
        let result = DefSubclassProc(window, message, wp, lp);
        if matches!(
            message,
            WM_SETFONT
                | WM_SIZE
                | WM_ENABLE
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_THEMECHANGED
                | WM_SYSCOLORCHANGE
                | WM_DPICHANGED
                | WM_DPICHANGED_AFTERPARENT
                | WM_UPDATEUISTATE
                | WM_KEYDOWN
                | WM_CHAR
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | CB_SETCURSEL
                | CB_SELECTSTRING
                | CB_ADDSTRING
                | CB_INSERTSTRING
                | CB_DELETESTRING
                | CB_RESETCONTENT
                | CB_SHOWDROPDOWN
        ) && IsWindow(window) != 0
        {
            InvalidateRect(window, null(), 0);
        }
        result
    }
}

unsafe fn selected_caption(window: HWND) -> Vec<u16> {
    unsafe {
        let selected = SendMessageW(window, CB_GETCURSEL, 0, 0);
        if selected < 0 {
            return vec![0];
        }
        let length = SendMessageW(window, CB_GETLBTEXTLEN, selected as usize, 0);
        if !(0..=MAX_CAPTION as isize).contains(&length) {
            return vec![0];
        }
        let mut caption = vec![0u16; length as usize + 1];
        if SendMessageW(
            window,
            CB_GETLBTEXT,
            selected as usize,
            caption.as_mut_ptr() as isize,
        ) < 0
        {
            return vec![0];
        }
        caption
    }
}

unsafe fn paint_face(window: HWND, dc: HDC) {
    unsafe {
        if dc.is_null() {
            return;
        }
        let mut rect = RECT::default();
        if GetClientRect(window, &mut rect) == 0 || rect.right <= 0 || rect.bottom <= 0 {
            return;
        }
        let saved = SaveDC(dc);
        if saved == 0 {
            return;
        }
        classic::paint_toolbar(dc, rect);

        let mut info = COMBOBOXINFO {
            cbSize: size_of::<COMBOBOXINFO>() as u32,
            ..Default::default()
        };
        let have_info = GetComboBoxInfo(window, &mut info) != 0;
        let focused = GetFocus() == window
            || (have_info && !info.hwndItem.is_null() && GetFocus() == info.hwndItem);
        let border = CreateSolidBrush(if focused { WHITE } else { BORDER });
        if !border.is_null() {
            FrameRect(dc, &rect, border);
            DeleteObject(border);
        }
        let dpi = dpi::window_dpi(window);
        let button = if have_info {
            info.rcButton
        } else {
            RECT {
                left: (rect.right - dpi::scale(20, dpi)).max(0),
                top: 0,
                right: rect.right,
                bottom: rect.bottom,
            }
        };
        let font = SendMessageW(window, WM_GETFONT, 0, 0) as HFONT;
        SelectObject(
            dc,
            if font.is_null() {
                GetStockObject(DEFAULT_GUI_FONT)
            } else {
                font
            },
        );
        SetBkMode(dc, TRANSPARENT as i32);
        let color = if IsWindowEnabled(window) != 0 {
            WHITE
        } else {
            0x00dc_dcdc
        };
        SetTextColor(dc, color);
        let mut text_rect = RECT {
            left: dpi::scale(8, dpi),
            top: 1,
            right: (button.left - dpi::scale(4, dpi)).max(0),
            bottom: rect.bottom - 1,
        };
        let caption = selected_caption(window);
        DrawTextW(
            dc,
            caption.as_ptr(),
            -1,
            &mut text_rect,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );

        let cx = (button.left + button.right) / 2;
        let cy = (button.top + button.bottom) / 2;
        let radius = dpi::scale(4, dpi)
            .max(2)
            .min((button.right - button.left).max(2) / 2);
        let height = dpi::scale(3, dpi).max(2).min((rect.bottom / 3).max(2));
        let arrow = [
            POINT {
                x: cx - radius,
                y: cy - height / 2,
            },
            POINT {
                x: cx + radius,
                y: cy - height / 2,
            },
            POINT {
                x: cx,
                y: cy + height / 2,
            },
        ];
        let brush = CreateSolidBrush(color);
        if !brush.is_null() {
            SelectObject(dc, brush);
            SelectObject(dc, GetStockObject(NULL_PEN));
            Polygon(dc, arrow.as_ptr(), arrow.len() as i32);
            RestoreDC(dc, saved);
            DeleteObject(brush);
        } else {
            RestoreDC(dc, saved);
        }
    }
}

#[cfg(test)]
mod tests;
