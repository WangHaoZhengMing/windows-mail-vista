//! Classic Windows Mail control creation and painting.
use super::controls::control;
use super::icons::{IconSet, ToolbarIcon};
use super::{dpi, icons, strings::wide};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{Controls::*, Shell::*, WindowsAndMessaging::*},
};

pub const TOOLBAR: i32 = 2101;

/// Initialize the common control classes used by the application's windows.
///
/// # Safety
/// Call on the UI thread before creating controls using these classes.
pub unsafe fn init() {
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES | ICC_TREEVIEW_CLASSES | ICC_BAR_CLASSES,
        });
    }
}
/// Replace a toolbar's image list and return the newly owned list.
/// The caller retains responsibility for releasing the previous list.
///
/// # Safety
/// `bar` must be a live toolbar on the calling UI thread. Allow synchronous
/// window message reentry, and keep the returned list alive while attached.
pub unsafe fn resize_toolbar(bar: HWND, dpi: u32) -> HIMAGELIST {
    unsafe {
        let size = dpi::scale(20, dpi);
        let button = dpi::scale(28, dpi);
        let icons = icons::images_for_dpi(IconSet::Toolbar, dpi);
        SendMessageW(bar, TB_SETIMAGELIST, 0, icons);
        SendMessageW(bar, TB_SETBITMAPSIZE, 0, (size | (size << 16)) as isize);
        SendMessageW(bar, TB_SETBUTTONSIZE, 0, (button | (button << 16)) as isize);
        SendMessageW(bar, TB_AUTOSIZE, 0, 0);
        icons
    }
}
/// Create a child toolbar and its image list. The parent owns the window;
/// the caller owns the image list and releases it after detaching or destruction.
///
/// # Safety
/// `owner` must be a live window on the calling UI thread. Allow synchronous
/// window message reentry while creating and configuring the child.
pub unsafe fn toolbar(
    owner: HWND,
    entries: &[(&str, u16, ToolbarIcon, bool)],
) -> (HWND, HIMAGELIST) {
    unsafe {
        let bar = control(
            owner,
            "ToolbarWindow32",
            "Toolbar",
            TBSTYLE_FLAT
                | TBSTYLE_TOOLTIPS
                | TBSTYLE_TRANSPARENT
                | TBSTYLE_LIST
                | CCS_NORESIZE as u32
                | CCS_NOPARENTALIGN as u32
                | CCS_NODIVIDER as u32,
            TOOLBAR as u16,
        );
        // Keep the classic gradient while retaining v6 image-list alpha support.
        // The themed toolbar otherwise paints over our custom background.
        SetWindowTheme(bar, wide("").as_ptr(), wide("").as_ptr());
        SendMessageW(bar, TB_BUTTONSTRUCTSIZE, std::mem::size_of::<TBBUTTON>(), 0);
        let toolbar_dpi = dpi::window_dpi(owner);
        let size = dpi::scale(20, toolbar_dpi);
        SendMessageW(bar, TB_SETBITMAPSIZE, 0, (size | (size << 16)) as isize);
        let icons = icons::images_for_dpi(IconSet::Toolbar, toolbar_dpi);
        SendMessageW(bar, TB_SETIMAGELIST, 0, icons as isize);
        for (label, id, image, enabled) in entries {
            let text = wide(&format!("{label}\0"));
            let string = SendMessageW(bar, TB_ADDSTRINGW, 0, text.as_ptr() as isize);
            let button = TBBUTTON {
                iBitmap: *image as i32,
                idCommand: *id as i32,
                fsState: if *enabled { TBSTATE_ENABLED as u8 } else { 0 },
                fsStyle: (BTNS_AUTOSIZE | BTNS_SHOWTEXT) as u8,
                iString: string,
                ..Default::default()
            };
            SendMessageW(bar, TB_ADDBUTTONSW, 1, &button as *const _ as isize);
        }
        let button = dpi::scale(28, toolbar_dpi);
        SendMessageW(bar, TB_SETBUTTONSIZE, 0, (button | (button << 16)) as isize);
        SetWindowSubclass(bar, Some(toolbar_proc), 1, 0);
        (bar, icons)
    }
}
unsafe extern "system" fn toolbar_proc(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _id: usize,
    _data: usize,
) -> isize {
    unsafe {
        if msg == WM_ERASEBKGND {
            let mut rect = RECT::default();
            GetClientRect(hwnd, &mut rect);
            paint_toolbar(wp as HDC, rect);
            return 1;
        }
        if msg == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(toolbar_proc), 1);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}
pub(super) unsafe fn paint_toolbar(dc: HDC, rect: RECT) {
    unsafe {
        let height = (rect.bottom - rect.top).max(1);
        for y in rect.top..rect.bottom {
            let t = (y - rect.top) as f32 / height as f32;
            let rgb = if t < 0.25 {
                let k = t / 0.25;
                (112.0 - 83.0 * k, 159.0 - 79.0 * k, 211.0 - 82.0 * k)
            } else {
                let k = (t - 0.25) / 0.75;
                (29.0 - 15.0 * k, 80.0 - 42.0 * k, 129.0 - 56.0 * k)
            };
            let color = (rgb.0 as u32) | ((rgb.1 as u32) << 8) | ((rgb.2 as u32) << 16);
            let brush = CreateSolidBrush(color);
            FillRect(
                dc,
                &RECT {
                    left: rect.left,
                    top: y,
                    right: rect.right,
                    bottom: y + 1,
                },
                brush,
            );
            DeleteObject(brush);
        }
    }
}
/// Handle the toolbar's tooltip and custom-draw notifications, if applicable.
///
/// # Safety
/// `lp` must be zero or the valid `WM_NOTIFY` notification pointer from the
/// calling window procedure. A matching custom-draw notification must contain
/// a mutable `NMTBCUSTOMDRAW` for the duration of this call. A tooltip notification
/// must contain a valid `NMTBGETINFOTIPW` and its writable text buffer.
pub unsafe fn custom_draw(lp: isize) -> Option<isize> {
    unsafe {
        if lp == 0 {
            return None;
        }
        // Some notifications (including NMLVKEYDOWN) are packed. Read the
        // common header by value before deciding which payload is present.
        let hdr = (lp as *const NMHDR).read_unaligned();
        if hdr.idFrom == TOOLBAR as usize && hdr.code == TBN_GETINFOTIPW {
            let tip = (lp as *const NMTBGETINFOTIPW).read_unaligned();
            if !tip.pszText.is_null() && tip.cchTextMax > 0 {
                let length = SendMessageW(hdr.hwndFrom, TB_GETBUTTONTEXTW, tip.iItem as usize, 0);
                if length >= 0 {
                    // Read into an owned buffer first: the tooltip's buffer may
                    // be shorter than the full button label.
                    let mut text = vec![0u16; length as usize + 1];
                    SendMessageW(
                        hdr.hwndFrom,
                        TB_GETBUTTONTEXTW,
                        tip.iItem as usize,
                        text.as_mut_ptr() as isize,
                    );
                    let count = (length as usize).min(tip.cchTextMax as usize - 1);
                    std::ptr::copy_nonoverlapping(text.as_ptr(), tip.pszText, count);
                    *tip.pszText.add(count) = 0;
                } else {
                    *tip.pszText = 0;
                }
            }
            return Some(0);
        }
        if hdr.idFrom == TOOLBAR as usize && hdr.code == NM_CUSTOMDRAW {
            let pointer = lp as *mut NMTBCUSTOMDRAW;
            let draw = pointer.read_unaligned();
            if draw.nmcd.dwDrawStage == CDDS_PREPAINT {
                paint_toolbar(draw.nmcd.hdc, draw.nmcd.rc);
                return Some(CDRF_NOTIFYITEMDRAW as isize);
            }
            if draw.nmcd.dwDrawStage == CDDS_ITEMPREPAINT {
                std::ptr::addr_of_mut!((*pointer).clrText).write_unaligned(0xffffff);
                std::ptr::addr_of_mut!((*pointer).clrTextHighlight).write_unaligned(0xffffff);
                return Some(TBCDRF_USECDCOLORS as isize);
            }
            return Some(0);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_notifications_are_checked_before_reading_custom_draw_payloads() {
        let mut buffer = vec![0u8; std::mem::size_of::<NMLVKEYDOWN>() + 1];
        let pointer = unsafe { buffer.as_mut_ptr().add(1) };
        unsafe {
            pointer.cast::<NMLVKEYDOWN>().write_unaligned(NMLVKEYDOWN {
                hdr: NMHDR {
                    hwndFrom: std::ptr::null_mut(),
                    idFrom: TOOLBAR as usize,
                    code: LVN_KEYDOWN,
                },
                wVKey: 0x41,
                flags: 0,
            });
            assert_eq!(custom_draw(pointer as isize), None);
        }
    }
}
