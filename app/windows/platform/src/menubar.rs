//! A client-area menu row which can share its white background with search.
//! The popup menus remain native Windows menus, with their original command IDs.
use super::{dpi, strings::wide};
use std::{
    cell::Cell,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, POINT, RECT},
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::*,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

mod keyboard;
mod paint;
#[cfg(test)]
mod tests;

const CLASS: &str = "WinMailRustMenuBar";
const FIRST_BUTTON: u16 = 0x7d00;
const SUBCLASS: usize = 0x7d00;
const SET_DPI: u32 = WM_APP + 78;
const OPEN: u32 = WM_APP + 79;
const WHITE: u32 = 0x00ffffff;

/// Borrowed handles: the parent destroys this child, which owns the detached
/// menu tree. Do not call DestroyMenu on `menu()` or any of its submenus.
#[derive(Clone, Copy)]
pub struct MenuBar {
    window: HWND,
    menu: HMENU,
}
impl MenuBar {
    /// Move the parent's existing menu into a 30-logical-pixel client row.
    /// Failure leaves the original menu attached and owned by its window.
    ///
    /// # Safety
    /// `parent` must be a live top-level window on the calling UI thread. The
    /// caller must continue to own its child windows and borrowed fonts.
    pub unsafe fn create(parent: HWND) -> Result<Self, String> {
        unsafe {
            let menu = GetMenu(parent);
            if menu.is_null() || IsMenu(menu) == 0 {
                return Err("The window has no native menu to display.".into());
            }
            let count = GetMenuItemCount(menu);
            if !(1..=64).contains(&count) {
                return Err("The menu has no supported top-level sections.".into());
            }
            let mut sections = Vec::new();
            for index in 0..count {
                let popup = GetSubMenu(menu, index);
                if popup.is_null() {
                    return Err("A top-level menu section has no popup.".into());
                }
                let length = GetMenuStringW(menu, index as u32, null_mut(), 0, MF_BYPOSITION);
                let mut label = vec![0; length.max(0) as usize + 1];
                GetMenuStringW(
                    menu,
                    index as u32,
                    label.as_mut_ptr(),
                    label.len() as i32,
                    MF_BYPOSITION,
                );
                sections.push(Section {
                    label,
                    button: Cell::new(null_mut()),
                    popup,
                });
            }
            let class = wide(CLASS);
            let definition = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: GetModuleHandleW(null()),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            if RegisterClassW(&definition) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err("Could not register the menu row.".into());
            }
            let state = Box::new(State {
                parent,
                menu,
                sections,
                dpi: Cell::new(dpi::window_dpi(parent)),
                font: Cell::new(GetStockObject(DEFAULT_GUI_FONT) as HFONT),
                owns_menu: Cell::new(false),
                window_owned: Cell::new(false),
                hot: Cell::new(None),
                active: Cell::new(None),
                pending: Cell::new(None),
                keyboard: Cell::new(false),
                alt_down: Cell::new(false),
                previous_focus: Cell::new(null_mut()),
                popup_open: Cell::new(false),
                selected_popup: Cell::new(null_mut()),
                selected_has_children: Cell::new(false),
                buttons_width: Cell::new(0),
            });
            let window = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                class.as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                dpi::scale(500, state.dpi.get()),
                dpi::scale(30, state.dpi.get()),
                parent,
                null_mut(),
                GetModuleHandleW(null()),
                (&*state as *const State).cast(),
            );
            if window.is_null() {
                return Err("Could not create the menu row.".into());
            }
            // Detach only after every control exists. SetMenu never destroys the
            // previous menu; ownership transfers to this child only on success.
            if SetMenu(parent, null_mut()) == 0 {
                DestroyWindow(window);
                return Err("Could not move the native menu into its client row.".into());
            }
            state.owns_menu.set(true);
            state.window_owned.set(true);
            let _ = Box::into_raw(state);
            Ok(Self { window, menu })
        }
    }
    pub fn hwnd(&self) -> HWND {
        self.window
    }
    pub fn menu(&self) -> HMENU {
        self.menu
    }
    /// Width occupied by menu buttons in physical pixels, including left padding.
    ///
    /// # Safety
    /// This control must be live and on the calling UI thread.
    pub unsafe fn buttons_width(&self) -> i32 {
        unsafe { lookup(self.window).map_or(0, |state| state.buttons_width.get()) }
    }
    /// # Safety
    /// This control must be live and on the calling UI thread.
    pub unsafe fn update_dpi(&self, value: u32) {
        unsafe {
            SendMessageW(self.window, SET_DPI, value.max(96) as usize, 0);
        }
    }
    /// Handle menu mnemonics before IsDialogMessage/TranslateMessage.
    ///
    /// # Safety
    /// The message and control must belong to the calling UI thread.
    pub unsafe fn translate(&self, message: &MSG) -> bool {
        unsafe { keyboard::translate(self.window, message) }
    }
}
struct Section {
    label: Vec<u16>,
    button: Cell<HWND>,
    popup: HMENU,
}
struct State {
    parent: HWND,
    menu: HMENU,
    sections: Vec<Section>,
    dpi: Cell<u32>,
    font: Cell<HFONT>,
    owns_menu: Cell<bool>,
    window_owned: Cell<bool>,
    hot: Cell<Option<usize>>,
    active: Cell<Option<usize>>,
    pending: Cell<Option<usize>>,
    keyboard: Cell<bool>,
    alt_down: Cell<bool>,
    previous_focus: Cell<HWND>,
    popup_open: Cell<bool>,
    selected_popup: Cell<HMENU>,
    selected_has_children: Cell<bool>,
    buttons_width: Cell<i32>,
}
impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            if self.owns_menu.get() {
                DestroyMenu(self.menu);
            }
        }
    }
}
impl State {
    unsafe fn create_buttons(&self, window: HWND) -> Result<(), String> {
        unsafe {
            for (index, section) in self.sections.iter().enumerate() {
                let button = CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    section.label.as_ptr(),
                    WS_CHILD | WS_VISIBLE | BS_OWNERDRAW as u32,
                    0,
                    0,
                    1,
                    1,
                    window,
                    (FIRST_BUTTON as usize + index) as HMENU,
                    GetModuleHandleW(null()),
                    null(),
                );
                if button.is_null() {
                    return Err("Could not create a menu button.".into());
                }
                section.button.set(button);
                SendMessageW(button, WM_SETFONT, self.font.get() as usize, 0);
                if SetWindowSubclass(button, Some(button_proc), SUBCLASS, window as usize) == 0 {
                    return Err("Could not initialize menu keyboard behavior.".into());
                }
            }
            if SetWindowSubclass(self.parent, Some(owner_proc), SUBCLASS, window as usize) == 0 {
                return Err("Could not initialize native popup navigation.".into());
            }
            self.layout(window);
            Ok(())
        }
    }
    unsafe fn layout(&self, window: HWND) {
        unsafe {
            let mut rect = RECT::default();
            GetClientRect(window, &mut rect);
            let dc = GetDC(window);
            let previous = SelectObject(dc, self.font.get());
            let mut x = dpi::scale(2, self.dpi.get());
            for section in &self.sections {
                let mut bounds = RECT::default();
                DrawTextW(
                    dc,
                    section.label.as_ptr(),
                    (section.label.len() - 1) as i32,
                    &mut bounds,
                    DT_CALCRECT | DT_SINGLELINE,
                );
                let width = (bounds.right - bounds.left + dpi::scale(14, self.dpi.get()))
                    .max(dpi::scale(24, self.dpi.get()));
                MoveWindow(
                    section.button.get(),
                    x,
                    1,
                    width,
                    (rect.bottom - 2).max(1),
                    1,
                );
                x += width;
            }
            self.buttons_width.set(x);
            SelectObject(dc, previous);
            ReleaseDC(window, dc);
            InvalidateRect(window, null(), 0);
        }
    }
    unsafe fn invalidate(&self, window: HWND) {
        unsafe {
            InvalidateRect(window, null(), 0);
            for section in &self.sections {
                InvalidateRect(section.button.get(), null(), 0);
            }
        }
    }
    unsafe fn focus(&self, window: HWND, index: usize) {
        unsafe {
            if !self.keyboard.replace(true) {
                self.previous_focus.set(GetFocus());
            }
            self.active.set(Some(index));
            SetFocus(self.sections[index].button.get());
            self.invalidate(window);
        }
    }
}
unsafe fn lookup<'a>(window: HWND) -> Option<&'a State> {
    unsafe { (GetWindowLongPtrW(window, GWLP_USERDATA) as *const State).as_ref() }
}
unsafe fn restore_focus(window: HWND) {
    unsafe {
        let Some(state) = lookup(window) else {
            return;
        };
        let focus = state.previous_focus.get();
        state.keyboard.set(false);
        state.active.set(None);
        state.invalidate(window);
        if !focus.is_null() && IsWindow(focus) != 0 {
            SetFocus(focus);
        }
    }
}
unsafe fn open_popup(window: HWND, mut index: usize) {
    unsafe {
        let Some(state) = lookup(window) else {
            return;
        };
        if state.popup_open.replace(true) {
            return;
        }
        if state.previous_focus.get().is_null() {
            state.previous_focus.set(GetFocus());
        }
        let keyboard = state.keyboard.get();
        loop {
            let Some(state) = lookup(window) else {
                return;
            };
            if index >= state.sections.len() {
                return;
            }
            state.active.set(Some(index));
            state.pending.set(None);
            state.selected_popup.set(state.sections[index].popup);
            state.selected_has_children.set(false);
            state.invalidate(window);
            let mut bounds = RECT::default();
            GetWindowRect(state.sections[index].button.get(), &mut bounds);
            let parent = state.parent;
            let popup = state.sections[index].popup;
            let _hook = keyboard::PopupHook::new(window);
            if keyboard {
                PostMessageW(parent, WM_KEYDOWN, VK_DOWN as usize, 0);
            }
            let parameters = TPMPARAMS {
                cbSize: std::mem::size_of::<TPMPARAMS>() as u32,
                rcExclude: bounds,
            };
            let command = TrackPopupMenuEx(
                popup,
                TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RETURNCMD,
                bounds.left,
                bounds.bottom,
                parent,
                &parameters,
            );
            // Native popup callbacks can synchronously close the owner. Resolve
            // state afresh; never keep a reference alive after TrackPopupMenuEx.
            let Some(state) = lookup(window) else {
                return;
            };
            if let Some(next) = state.pending.take() {
                index = next;
                continue;
            }
            state.popup_open.set(false);
            let parent = state.parent;
            restore_focus(window);
            if command != 0 && IsWindow(parent) != 0 {
                PostMessageW(parent, WM_COMMAND, command as usize, 0);
            }
            break;
        }
    }
}
unsafe extern "system" fn window_proc(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if message == WM_NCCREATE {
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (*(lp as *const CREATESTRUCTW)).lpCreateParams as _,
            );
            return 1;
        }
        let Some(state) = lookup(window) else {
            return DefWindowProcW(window, message, wp, lp);
        };
        match message {
            WM_CREATE => {
                if state.create_buttons(window).is_ok() {
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
                state.font.set(wp as HFONT);
                for section in &state.sections {
                    SendMessageW(section.button.get(), WM_SETFONT, wp, lp);
                }
                state.layout(window);
                0
            }
            WM_GETFONT => state.font.get() as isize,
            WM_COMMAND => {
                let index = (wp & 0xffff).checked_sub(FIRST_BUTTON as usize);
                if let Some(index) = index.filter(|&i| i < state.sections.len())
                    && lp as HWND == state.sections[index].button.get()
                {
                    open_popup(window, index);
                } else {
                    let parent = state.parent;
                    SendMessageW(parent, message, wp, lp);
                }
                0
            }
            OPEN => {
                open_popup(window, wp);
                0
            }
            WM_DRAWITEM
                if (FIRST_BUTTON as usize..FIRST_BUTTON as usize + state.sections.len())
                    .contains(&wp) =>
            {
                paint::button(
                    &*(lp as *const DRAWITEMSTRUCT),
                    state,
                    wp - FIRST_BUTTON as usize,
                );
                1
            }
            WM_PAINT => {
                let mut ps = std::mem::zeroed();
                let dc = BeginPaint(window, &mut ps);
                paint::background(window, dc);
                EndPaint(window, &ps);
                0
            }
            WM_PRINTCLIENT => {
                paint::background(window, wp as HDC);
                0
            }
            WM_ERASEBKGND => 1,
            WM_NCDESTROY => {
                let owned = state.window_owned.get();
                RemoveWindowSubclass(state.parent, Some(owner_proc), SUBCLASS);
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
unsafe extern "system" fn button_proc(
    window: HWND,
    message: u32,
    wp: usize,
    lp: isize,
    _id: usize,
    host: usize,
) -> isize {
    unsafe {
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(window, Some(button_proc), SUBCLASS);
            return DefSubclassProc(window, message, wp, lp);
        }
        let Some(state) = lookup(host as HWND) else {
            return DefSubclassProc(window, message, wp, lp);
        };
        let index = (GetDlgCtrlID(window) - FIRST_BUTTON as i32) as usize;
        match message {
            WM_LBUTTONDOWN => {
                if !state.keyboard.get() {
                    state.previous_focus.set(GetFocus());
                }
            }
            WM_MOUSEMOVE => {
                if state.hot.replace(Some(index)) != Some(index) {
                    state.invalidate(host as HWND);
                }
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: window,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut tracking);
            }
            WM_MOUSELEAVE => {
                state.hot.set(None);
                state.invalidate(host as HWND);
            }
            WM_GETDLGCODE => return DLGC_WANTARROWS as isize | DLGC_WANTMESSAGE as isize,
            WM_KEYDOWN if keyboard::button_key(host as HWND, index, wp) => return 0,
            WM_CHAR if [VK_RETURN as usize, VK_ESCAPE as usize].contains(&wp) => return 0,
            WM_KILLFOCUS
                if !state.popup_open.get()
                    && !state
                        .sections
                        .iter()
                        .any(|section| section.button.get() == wp as HWND) =>
            {
                state.keyboard.set(false);
                state.active.set(None);
                state.invalidate(host as HWND);
            }
            _ => {}
        }
        DefSubclassProc(window, message, wp, lp)
    }
}
unsafe extern "system" fn owner_proc(
    window: HWND,
    message: u32,
    wp: usize,
    lp: isize,
    _id: usize,
    host: usize,
) -> isize {
    unsafe {
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(window, Some(owner_proc), SUBCLASS);
        } else if message == WM_MENUSELECT
            && let Some(state) = lookup(host as HWND)
        {
            state.selected_popup.set(lp as HMENU);
            state
                .selected_has_children
                .set(((wp >> 16) as u32 & MF_POPUP) != 0);
        }
        DefSubclassProc(window, message, wp, lp)
    }
}
