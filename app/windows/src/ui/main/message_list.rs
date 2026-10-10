//! Compact list presentation; original sender/date headers remain unchanged.
use crate::platform::strings::wide;
use std::{
    cell::Cell,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{
        Controls::*,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

pub(super) struct Row {
    pub unread: bool,
    pub priority: i8,
    pub attachment: bool,
    pub flagged: bool,
    pub from_tip: Vec<u16>,
    pub date_tip: Vec<u16>,
}
/// Vista's first three cells are narrow status lanes. The message envelope is
/// a native subitem image in From, leaving these cells empty for normal mail.
pub(super) unsafe fn draw_status(list: HWND, draw: &NMLVCUSTOMDRAW, row: &Row) {
    unsafe {
        let mut rect = RECT {
            top: draw.iSubItem,
            left: LVIR_BOUNDS as i32,
            ..Default::default()
        };
        SendMessageW(
            list,
            LVM_GETSUBITEMRECT,
            draw.nmcd.dwItemSpec,
            &mut rect as *mut _ as isize,
        );
        if draw.iSubItem == 0 {
            rect.right = rect.left + SendMessageW(list, LVM_GETCOLUMNWIDTH, 0, 0) as i32;
        }
        // ListView subitem custom-draw flags do not reliably reflect row
        // selection. Query the native item state instead of trusting CDIS.
        let selected = SendMessageW(
            list,
            LVM_GETITEMSTATE,
            draw.nmcd.dwItemSpec,
            LVIS_SELECTED as isize,
        ) & LVIS_SELECTED as isize
            != 0;
        let focused = windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() == list;
        let background = if selected {
            GetSysColor(if focused {
                COLOR_HIGHLIGHT
            } else {
                COLOR_BTNFACE
            })
        } else if draw.iSubItem == 2 {
            0xfaf8f6
        } else {
            0xffffff
        };
        let brush = CreateSolidBrush(if selected {
            background
        } else {
            crate::platform::theme::surface(background)
        });
        FillRect(draw.nmcd.hdc, &rect, brush);
        DeleteObject(brush);
        let divider = CreateSolidBrush(crate::platform::theme::panel(0xf7eee3));
        let edge = RECT {
            left: rect.right - 1,
            ..rect
        };
        FillRect(draw.nmcd.hdc, &edge, divider);
        DeleteObject(divider);
        let (symbol, color) = match draw.iSubItem {
            0 if row.priority > 0 => ("!", 0x3030c0),
            0 if row.priority < 0 => ("↓", 0xcb8d35),
            1 if row.attachment => ("📎", 0x329fce),
            2 if row.flagged => ("⚑", 0x3030c0),
            _ => return,
        };
        SetBkMode(draw.nmcd.hdc, TRANSPARENT as i32);
        SetTextColor(
            draw.nmcd.hdc,
            if selected && focused {
                GetSysColor(COLOR_HIGHLIGHTTEXT)
            } else {
                color
            },
        );
        DrawTextW(
            draw.nmcd.hdc,
            wide(symbol).as_ptr(),
            -1,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
    }
}
impl Row {
    pub fn new(from: &str, date: &str, read: bool) -> Self {
        Self {
            unread: !read,
            priority: 0,
            attachment: false,
            flagged: false,
            from_tip: wide(from),
            date_tip: wide(date),
        }
    }
}
/// A native tooltip attached to the entire list. Hit testing selects the exact
/// cell; changing rows/columns dismisses stale text before the next hover.
pub(super) struct Tooltip {
    pub window: HWND,
    list: HWND,
    cell: Cell<(i32, i32)>,
}
impl Tooltip {
    pub unsafe fn install(owner: HWND, list: HWND) -> Option<Box<Self>> {
        unsafe {
            let window = CreateWindowExW(
                WS_EX_TOPMOST,
                wide("tooltips_class32").as_ptr(),
                null(),
                WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                owner,
                null_mut(),
                null_mut(),
                null(),
            );
            if window.is_null() {
                return None;
            }
            let mut tooltip = Box::new(Self {
                window,
                list,
                cell: Cell::new((-1, -1)),
            });
            let info = TTTOOLINFOW {
                cbSize: std::mem::offset_of!(TTTOOLINFOW, lpReserved) as u32,
                uFlags: TTF_IDISHWND | TTF_SUBCLASS | TTF_TRANSPARENT,
                hwnd: owner,
                uId: list as usize,
                lpszText: (-1isize) as *mut u16, // LPSTR_TEXTCALLBACKW
                ..Default::default()
            };
            if SendMessageW(window, TTM_ADDTOOLW, 0, &info as *const _ as isize) == 0
                || SetWindowSubclass(list, Some(mouse), 4, (&mut *tooltip as *mut Self) as usize)
                    == 0
            {
                DestroyWindow(window);
                return None;
            }
            SendMessageW(window, TTM_SETMAXTIPWIDTH, 0, 600);
            Some(tooltip)
        }
    }
    pub unsafe fn hide(&self) {
        unsafe {
            SendMessageW(self.window, TTM_POP, 0, 0);
        }
    }
}
impl Drop for Tooltip {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.list) != 0 {
                RemoveWindowSubclass(self.list, Some(mouse), 4);
            }
            if IsWindow(self.window) != 0 {
                DestroyWindow(self.window);
            }
        }
    }
}
unsafe extern "system" fn mouse(
    window: HWND,
    message: u32,
    wp: usize,
    lp: isize,
    id: usize,
    reference: usize,
) -> LRESULT {
    unsafe {
        if message == WM_MOUSEMOVE {
            let tooltip = &*(reference as *const Tooltip);
            let mut hit = LVHITTESTINFO {
                pt: POINT {
                    x: lp as i16 as i32,
                    y: (lp >> 16) as i16 as i32,
                },
                ..Default::default()
            };
            SendMessageW(window, LVM_SUBITEMHITTEST, 0, &mut hit as *mut _ as isize);
            let cell = (hit.iItem, hit.iSubItem);
            if tooltip.cell.replace(cell) != cell {
                tooltip.hide();
            }
        } else if matches!(message, WM_VSCROLL | WM_HSCROLL | WM_MOUSEWHEEL | WM_SIZE) {
            (&*(reference as *const Tooltip)).hide();
        } else if message == WM_NCDESTROY {
            RemoveWindowSubclass(window, Some(mouse), id);
        }
        DefSubclassProc(window, message, wp, lp)
    }
}
pub(super) unsafe fn hovered(list: HWND) -> Option<(usize, i32)> {
    unsafe {
        let mut hit = LVHITTESTINFO::default();
        GetCursorPos(&mut hit.pt);
        ScreenToClient(list, &mut hit.pt);
        SendMessageW(list, LVM_SUBITEMHITTEST, 0, &mut hit as *mut _ as isize);
        usize::try_from(hit.iItem)
            .ok()
            .map(|index| (index, hit.iSubItem))
    }
}
