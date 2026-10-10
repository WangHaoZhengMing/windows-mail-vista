//! Classic Windows Mail About window with a painted identity banner.
use crate::platform::{dpi, icons, strings::wide};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, RECT},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
};

struct About {
    resources: dpi::Resources,
    button: HWND,
}

pub(super) unsafe fn show(owner: HWND) {
    unsafe {
        let class = wide("WinMail.About");
        let existing = FindWindowExW(null_mut(), null_mut(), class.as_ptr(), null());
        if !existing.is_null() {
            SetForegroundWindow(existing);
            return;
        }
        let definition = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: GetModuleHandleW(null()),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&definition) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return;
        }
        let value = dpi::window_dpi(owner);
        let mut bounds = RECT {
            left: 0,
            top: 0,
            right: dpi::scale(560, value),
            bottom: dpi::scale(420, value),
        };
        let style = WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
        AdjustWindowRectEx(&mut bounds, style, 0, WS_EX_DLGMODALFRAME);
        let mut parent = RECT::default();
        GetWindowRect(owner, &mut parent);
        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        let window = CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class.as_ptr(),
            wide("About Windows Mail Vista").as_ptr(),
            style,
            parent.left + (parent.right - parent.left - width) / 2,
            parent.top + (parent.bottom - parent.top - height) / 2,
            width,
            height,
            owner,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if window.is_null() {
            return;
        }
        let button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("OK").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            0,
            0,
            0,
            0,
            window,
            IDOK as usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        let state = Box::new(About {
            resources: dpi::Resources::default(),
            button,
        });
        SetWindowLongPtrW(window, GWLP_USERDATA, Box::into_raw(state) as isize);
        update(window, value);
        ShowWindow(window, SW_SHOW);
        SetFocus(button);
    }
}

unsafe fn update(window: HWND, value: u32) {
    unsafe {
        let state = &mut *(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut About);
        state.resources.update(window, value);
        let mut rect = RECT::default();
        GetClientRect(window, &mut rect);
        MoveWindow(
            state.button,
            rect.right - dpi::scale(108, value),
            rect.bottom - dpi::scale(44, value),
            dpi::scale(88, value),
            dpi::scale(26, value),
            1,
        );
        InvalidateRect(window, null(), 1);
    }
}

unsafe fn label(dc: HDC, value: u32, caption: &str, rect: [i32; 4], size: i32, color: u32) {
    unsafe {
        let font = CreateFontW(
            -dpi::scale(size, value),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let previous = SelectObject(dc, font);
        SetTextColor(dc, color);
        let [x, y, w, h] = rect;
        let mut bounds = RECT {
            left: dpi::scale(x, value),
            top: dpi::scale(y, value),
            right: dpi::scale(x + w, value),
            bottom: dpi::scale(y + h, value),
        };
        DrawTextW(
            dc,
            wide(caption).as_ptr(),
            -1,
            &mut bounds,
            DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
        DeleteObject(font);
    }
}

unsafe extern "system" fn procedure(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut About;
        if pointer.is_null() {
            return DefWindowProcW(window, message, wp, lp);
        }
        match message {
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(window, &mut paint);
                let mut bounds = RECT::default();
                GetClientRect(window, &mut bounds);
                SetDCBrushColor(
                    dc,
                    crate::platform::theme::panel(GetSysColor(COLOR_BTNFACE)),
                );
                FillRect(dc, &bounds, GetStockObject(DC_BRUSH) as HBRUSH);
                let value = (*pointer).resources.dpi;
                let banner = dpi::scale(88, value);
                for x in 0..bounds.right {
                    let t = x as f32 / bounds.right.max(1) as f32;
                    let r = (6.0 + 20.0 * t) as u32;
                    let g = (79.0 + 40.0 * t) as u32;
                    let b = (120.0 + 10.0 * t) as u32;
                    let brush =
                        CreateSolidBrush(crate::platform::theme::panel(r | g << 8 | b << 16));
                    FillRect(
                        dc,
                        &RECT {
                            left: x,
                            top: 0,
                            right: x + 1,
                            bottom: banner,
                        },
                        brush,
                    );
                    DeleteObject(brush);
                }
                SetBkMode(dc, TRANSPARENT as i32);
                let icon = icons::icon(dpi::scale(48, value));
                DrawIconEx(
                    dc,
                    dpi::scale(60, value),
                    dpi::scale(20, value),
                    icon,
                    dpi::scale(48, value),
                    dpi::scale(48, value),
                    0,
                    null_mut(),
                    DI_NORMAL,
                );
                if !icon.is_null() {
                    DestroyIcon(icon);
                }
                label(
                    dc,
                    value,
                    "Windows Mail Vista",
                    [124, 24, 420, 48],
                    24,
                    0x00ffffff,
                );
                label(
                    dc,
                    value,
                    &format!(
                        "Windows Mail Vista\nVersion {} · Rust desktop client",
                        env!("CARGO_PKG_VERSION")
                    ),
                    [58, 114, 448, 52],
                    14,
                    crate::platform::theme::text(GetSysColor(COLOR_WINDOWTEXT)),
                );
                label(
                    dc,
                    value,
                    "A desktop email client inspired by classic Windows Mail.\nRead and organize mail, manage multiple accounts, and compose messages with attachments.",
                    [58, 178, 448, 70],
                    13,
                    crate::platform::theme::text(GetSysColor(COLOR_WINDOWTEXT)),
                );
                label(
                    dc,
                    value,
                    "Supports IMAP, POP3 and SMTP.\nBuilt with Rust and native Windows controls.",
                    [58, 264, 448, 48],
                    13,
                    crate::platform::theme::text(GetSysColor(COLOR_WINDOWTEXT)),
                );
                label(
                    dc,
                    value,
                    "An independent implementation. Not affiliated with Microsoft.\nWindows and Windows Mail are trademarks of Microsoft.",
                    [58, 328, 448, 42],
                    11,
                    crate::platform::theme::text(GetSysColor(COLOR_GRAYTEXT)),
                );
                EndPaint(window, &paint);
                0
            }
            WM_COMMAND if wp & 0xffff == IDOK as usize => {
                DestroyWindow(window);
                0
            }
            WM_CLOSE => {
                DestroyWindow(window);
                0
            }
            WM_DPICHANGED => {
                dpi::suggested(window, lp);
                update(window, (wp & 0xffff) as u32);
                0
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                drop(Box::from_raw(pointer));
                DefWindowProcW(window, message, wp, lp)
            }
            _ => DefWindowProcW(window, message, wp, lp),
        }
    }
}
