//! A classic search field with a real EDIT, accessible icon button, and a
//! scalable Win7-style frame. The parent window owns both child controls.
use super::{dpi, strings::wide};
use std::{
    cell::Cell,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, RECT},
    Graphics::{
        Gdi::*,
        GdiPlus::{
            DashCapFlat, GdipCreateFromHDC, GdipCreatePen1, GdipDeleteGraphics, GdipDeletePen,
            GdipDrawEllipse, GdipDrawLine, GdipSetPenLineCap197819, GdipSetSmoothingMode,
            GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, GpGraphics, GpPen, LineCapRound,
            SmoothingModeAntiAlias8x8, UnitPixel,
        },
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::*,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

mod paint;
#[cfg(test)]
mod tests;

const CLASS: &str = "WinMailRustSearchBox";
const ICON_BUTTON: u16 = 0x7e01;
const SUBCLASS: usize = 1;
const SET_DPI: u32 = WM_APP + 77;
const WHITE: u32 = 0x00ffffff;

/// Borrowed handles. Destroying the parent destroys the search field; dropping
/// this value does not destroy Windows-owned controls or the caller's font.
#[derive(Clone, Copy)]
pub struct SearchBox {
    window: HWND,
    edit: HWND,
}
impl SearchBox {
    /// Create a 170 × 26 logical-pixel search field. EN_CHANGE is forwarded to
    /// `parent` with `command_id` and the real EDIT handle in lParam.
    ///
    /// # Safety
    /// `parent` must be live and owned by the calling UI thread. Allow synchronous
    /// message reentry when editing text or changing keyboard focus.
    pub unsafe fn create(parent: HWND, command_id: u16) -> Result<Self, String> {
        unsafe {
            let class = wide(CLASS);
            let definition = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: GetModuleHandleW(null()),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            if RegisterClassW(&definition) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err("Could not register the search field.".into());
            }
            let state = Box::new(State {
                parent,
                command_id,
                edit: Cell::new(null_mut()),
                button: Cell::new(null_mut()),
                dpi: Cell::new(dpi::window_dpi(parent)),
                focused: Cell::new(false),
                hot: Cell::new(false),
                has_text: Cell::new(false),
                window_owned: Cell::new(false),
                _graphics: GraphicsRuntime::new()?,
            });
            let value = state.dpi.get();
            let window = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                class.as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                dpi::scale(170, value),
                dpi::scale(26, value),
                parent,
                null_mut(),
                GetModuleHandleW(null()),
                (&*state as *const State).cast(),
            );
            if window.is_null() {
                return Err("Could not create the search field.".into());
            }
            state.window_owned.set(true);
            let edit = state.edit.get();
            let _ = Box::into_raw(state);
            Ok(Self { window, edit })
        }
    }
    pub fn hwnd(&self) -> HWND {
        self.window
    }
    pub fn edit(&self) -> HWND {
        self.edit
    }
    /// Reflow the EDIT and redraw vector strokes using physical pixel dimensions.
    /// The caller continues to own and apply fonts, including through EnumChildWindows.
    ///
    /// # Safety
    /// This control must be live and on the calling UI thread.
    pub unsafe fn update_dpi(&self, value: u32) {
        unsafe {
            SendMessageW(self.window, SET_DPI, value.max(96) as usize, 0);
        }
    }
}

struct GraphicsRuntime(usize);
impl GraphicsRuntime {
    unsafe fn new() -> Result<Self, String> {
        unsafe {
            let mut token = 0;
            let input = GdiplusStartupInput {
                GdiplusVersion: 1,
                SuppressExternalCodecs: 1,
                ..Default::default()
            };
            if GdiplusStartup(&mut token, &input, null_mut()) != 0 {
                return Err("Could not initialize search icon drawing.".into());
            }
            Ok(Self(token))
        }
    }
}
impl Drop for GraphicsRuntime {
    fn drop(&mut self) {
        unsafe {
            GdiplusShutdown(self.0);
        }
    }
}
struct State {
    parent: HWND,
    command_id: u16,
    edit: Cell<HWND>,
    button: Cell<HWND>,
    dpi: Cell<u32>,
    focused: Cell<bool>,
    hot: Cell<bool>,
    has_text: Cell<bool>,
    window_owned: Cell<bool>,
    _graphics: GraphicsRuntime,
}
impl State {
    unsafe fn create_children(&self, window: HWND) -> Result<(), String> {
        unsafe {
            let instance = GetModuleHandleW(null());
            let edit = CreateWindowExW(
                0,
                wide("EDIT").as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                0,
                0,
                1,
                1,
                window,
                self.command_id as usize as HMENU,
                instance,
                null(),
            );
            if edit.is_null() {
                return Err("Could not create search text input.".into());
            }
            self.edit.set(edit);
            SendMessageW(
                edit,
                WM_SETFONT,
                GetStockObject(DEFAULT_GUI_FONT) as usize,
                0,
            );
            SendMessageW(edit, EM_SETLIMITTEXT, 1024, 0);
            SendMessageW(
                edit,
                EM_SETMARGINS,
                (EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize,
                0,
            );
            SendMessageW(
                edit,
                EM_SETCUEBANNER,
                1,
                wide("Search mail").as_ptr() as isize,
            );
            let button = CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide("Search mail").as_ptr(),
                WS_CHILD | WS_VISIBLE | BS_OWNERDRAW as u32,
                0,
                0,
                1,
                1,
                window,
                ICON_BUTTON as usize as HMENU,
                instance,
                null(),
            );
            if button.is_null() {
                return Err("Could not create search icon button.".into());
            }
            self.button.set(button);
            if SetWindowSubclass(edit, Some(child_proc), SUBCLASS, window as usize) == 0
                || SetWindowSubclass(button, Some(child_proc), SUBCLASS, window as usize) == 0
            {
                return Err("Could not initialize search input behavior.".into());
            }
            self.layout(window);
            Ok(())
        }
    }
    unsafe fn layout(&self, window: HWND) {
        unsafe {
            let mut rect = RECT::default();
            GetClientRect(window, &mut rect);
            let value = self.dpi.get();
            let padding = dpi::scale(2, value);
            let button_width = dpi::scale(22, value).min((rect.right - padding * 2).max(1));
            let right = (rect.right - padding - button_width).max(padding);
            let left = padding + dpi::scale(3, value);
            let height = (rect.bottom - padding * 2).max(1);
            let mut metrics: TEXTMETRICW = std::mem::zeroed();
            let dc = GetDC(self.edit.get());
            let font = SendMessageW(self.edit.get(), WM_GETFONT, 0, 0) as HGDIOBJ;
            let previous = SelectObject(dc, font);
            GetTextMetricsW(dc, &mut metrics);
            SelectObject(dc, previous);
            ReleaseDC(self.edit.get(), dc);
            let text_height = (metrics.tmHeight + dpi::scale(2, value)).min(height).max(1);
            MoveWindow(
                self.edit.get(),
                left,
                padding + (height - text_height) / 2,
                (right - left - dpi::scale(2, value)).max(1),
                text_height,
                1,
            );
            MoveWindow(self.button.get(), right, padding, button_width, height, 1);
            InvalidateRect(window, null(), 0);
            InvalidateRect(self.button.get(), null(), 0);
        }
    }
    unsafe fn text_changed(&self, window: HWND) {
        unsafe {
            self.has_text
                .set(GetWindowTextLengthW(self.edit.get()) != 0);
            SetWindowTextW(
                self.button.get(),
                wide(if self.has_text.get() {
                    "Clear search"
                } else {
                    "Search mail"
                })
                .as_ptr(),
            );
            InvalidateRect(window, null(), 0);
            InvalidateRect(self.button.get(), null(), 0);
        }
    }
}
unsafe fn lookup(window: HWND) -> Option<&'static State> {
    unsafe { (GetWindowLongPtrW(window, GWLP_USERDATA) as *const State).as_ref() }
}
unsafe fn track(window: HWND) {
    unsafe {
        let mut tracking = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: window,
            dwHoverTime: 0,
        };
        TrackMouseEvent(&mut tracking);
    }
}
unsafe fn activate(edit: HWND) {
    unsafe {
        let filled = GetWindowTextLengthW(edit) != 0;
        SetFocus(edit);
        if filled && IsWindow(edit) != 0 {
            SetWindowTextW(edit, wide("").as_ptr());
        }
    }
}
unsafe extern "system" fn window_proc(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if message == WM_NCCREATE {
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
            );
            return 1;
        }
        let Some(state) = lookup(window) else {
            return DefWindowProcW(window, message, wp, lp);
        };
        match message {
            WM_CREATE => {
                if state.create_children(window).is_ok() {
                    0
                } else {
                    -1
                }
            }
            WM_SIZE => {
                state.layout(window);
                0
            }
            SET_DPI => {
                state.dpi.set((wp as u32).max(96));
                state.layout(window);
                0
            }
            WM_SETFONT => {
                let edit = state.edit.get();
                SendMessageW(edit, WM_SETFONT, wp, lp);
                0
            }
            WM_GETFONT => SendMessageW(state.edit.get(), WM_GETFONT, 0, 0),
            WM_SETFOCUS => {
                SetFocus(state.edit.get());
                0
            }
            WM_COMMAND => {
                if lp as HWND == state.button.get() && (wp & 0xffff) as u16 == ICON_BUTTON {
                    activate(state.edit.get());
                } else if lp as HWND == state.edit.get() {
                    let notification = (wp >> 16) as u16;
                    if notification == EN_CHANGE as u16 {
                        state.text_changed(window);
                    }
                    let parent = state.parent;
                    let edit = state.edit.get();
                    let command = state.command_id as usize | ((notification as usize) << 16);
                    // No state access after forwarding: the owner may synchronously close.
                    SendMessageW(parent, WM_COMMAND, command, edit as isize);
                }
                0
            }
            WM_CTLCOLOREDIT => {
                SetBkColor(wp as HDC, super::theme::surface(WHITE));
                SetTextColor(wp as HDC, super::theme::text(GetSysColor(COLOR_WINDOWTEXT)));
                GetStockObject(WHITE_BRUSH) as isize
            }
            WM_DRAWITEM if wp == ICON_BUTTON as usize => {
                let item = &*(lp as *const DRAWITEMSTRUCT);
                paint::icon(item.hDC, item.rcItem, state, item.itemState);
                1
            }
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let dc = BeginPaint(window, &mut ps);
                let mut rect = RECT::default();
                GetClientRect(window, &mut rect);
                paint::frame(dc, rect, state);
                EndPaint(window, &ps);
                0
            }
            WM_PRINTCLIENT => {
                let mut rect = RECT::default();
                GetClientRect(window, &mut rect);
                paint::frame(wp as HDC, rect, state);
                0
            }
            WM_ERASEBKGND => 1,
            WM_MOUSEMOVE => {
                state.hot.set(true);
                track(window);
                InvalidateRect(window, null(), 0);
                0
            }
            WM_MOUSELEAVE => {
                state.hot.set(false);
                InvalidateRect(window, null(), 0);
                0
            }
            WM_LBUTTONDOWN => {
                SetFocus(state.edit.get());
                0
            }
            WM_NCDESTROY => {
                let owned = state.window_owned.get();
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                if owned {
                    drop(Box::from_raw(state as *const _ as *mut State));
                }
                DefWindowProcW(window, message, wp, lp)
            }
            _ => DefWindowProcW(window, message, wp, lp),
        }
    }
}
unsafe extern "system" fn child_proc(
    window: HWND,
    message: u32,
    wp: usize,
    lp: isize,
    _id: usize,
    host: usize,
) -> isize {
    unsafe {
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(window, Some(child_proc), SUBCLASS);
            return DefSubclassProc(window, message, wp, lp);
        }
        let Some(state) = lookup(host as HWND) else {
            return DefSubclassProc(window, message, wp, lp);
        };
        match message {
            WM_MOUSEMOVE => {
                state.hot.set(true);
                track(window);
                InvalidateRect(host as HWND, null(), 0);
                InvalidateRect(state.button.get(), null(), 0);
            }
            WM_MOUSELEAVE => {
                state.hot.set(false);
                InvalidateRect(host as HWND, null(), 0);
                InvalidateRect(state.button.get(), null(), 0);
            }
            WM_SETFOCUS => {
                state.focused.set(true);
                InvalidateRect(host as HWND, null(), 0);
            }
            WM_KILLFOCUS => {
                state
                    .focused
                    .set(wp as HWND == state.edit.get() || wp as HWND == state.button.get());
                InvalidateRect(host as HWND, null(), 0);
            }
            WM_GETDLGCODE
                if window == state.edit.get()
                    && [VK_RETURN as usize, VK_ESCAPE as usize].contains(&wp) =>
            {
                return DefSubclassProc(window, message, wp, lp) | DLGC_WANTMESSAGE as isize;
            }
            WM_KEYDOWN if window == state.edit.get() && wp == VK_ESCAPE as usize => {
                SetWindowTextW(window, wide("").as_ptr());
                return 0;
            }
            WM_KEYDOWN if window == state.edit.get() && wp == VK_RETURN as usize => {
                let command = state.command_id as usize | ((EN_CHANGE as usize) << 16);
                SendMessageW(host as HWND, WM_COMMAND, command, window as isize);
                return 0;
            }
            WM_CHAR
                if window == state.edit.get()
                    && [VK_RETURN as usize, VK_ESCAPE as usize].contains(&wp) =>
            {
                return 0;
            }
            _ => {}
        }
        let result = DefSubclassProc(window, message, wp, lp);
        if message == WM_SETFONT
            && IsWindow(host as HWND) != 0
            && let Some(state) = lookup(host as HWND)
        {
            state.layout(host as HWND);
        }
        result
    }
}
