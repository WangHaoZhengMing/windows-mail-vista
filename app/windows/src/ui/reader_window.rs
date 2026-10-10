//! An independent, read-only message window with a stable message identity.
//!
//! Commands carry this window's handle to the owner; the owner resolves its
//! `MessageId` instead of consulting whichever row is currently selected.
mod controls;
mod menus;
#[cfg(test)]
mod tests;

use super::{READER_DISPATCH, command_ids::command_id};
use crate::{
    Result,
    app::{Command, MessageId},
    message::Message,
    platform::{
        classic,
        controls::{control, set_window_text},
        dpi,
        html::{HtmlDocument, HtmlStatus, HtmlView, InlineImage, RemoteImagePolicy},
        strings::wide,
    },
};
use std::{
    cell::{Cell, RefCell},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::COLOR_BTNFACE,
    System::LibraryLoader::*,
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{
            GetFocus, GetKeyState, SetFocus, VK_CONTROL, VK_ESCAPE, VK_SHIFT,
        },
        WindowsAndMessaging::*,
    },
};

const HTML_STATUS: u32 = WM_APP + 13;
const SHOW_PICTURES: u16 = 426;
const CLOSE_READER: u16 = 427;
const SUBJECT: u16 = 420;
const FROM: u16 = 421;
const TO: u16 = 422;
const DATE: u16 = 423;
const BODY: u16 = 424;

#[derive(Default)]
struct Controls {
    toolbar: Cell<HWND>,
    headers: RefCell<Vec<(HWND, HWND)>>,
    body: Cell<HWND>,
    blocked: Cell<HWND>,
    pictures: Cell<HWND>,
    status: Cell<HWND>,
}

struct ReaderState {
    owner: HWND,
    hwnd: Cell<HWND>,
    id: MessageId,
    message: Message,
    controls: Controls,
    metrics: RefCell<dpi::Resources>,
    icons: Cell<HIMAGELIST>,
    renderer: RefCell<Option<HtmlView>>,
    remote_images: Cell<RemoteImagePolicy>,
}

/// Owns the native window and its COM renderer on the creating UI thread.
pub(super) struct Window {
    state: Box<ReaderState>,
}

impl Window {
    pub fn open(owner: HWND, id: MessageId, message: Message) -> Result<Self> {
        Self::create(owner, id, message, true)
    }

    fn create(owner: HWND, id: MessageId, message: Message, visible: bool) -> Result<Self> {
        unsafe {
            let state = Box::new(ReaderState {
                owner,
                hwnd: Cell::new(null_mut()),
                id,
                message,
                controls: Controls::default(),
                metrics: RefCell::new(dpi::Resources::default()),
                icons: Cell::new(0),
                renderer: RefCell::new(None),
                remote_images: Cell::new(RemoteImagePolicy::Block),
            });
            let class = wide("WinMailRust.Reader");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: GetModuleHandleW(null()),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: (COLOR_BTNFACE + 1) as _,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let title = if state.message.summary.subject.is_empty() {
                "Message - Windows Mail".to_owned()
            } else {
                format!("{} - Windows Mail", state.message.summary.subject)
            };
            let menu = menus::reader_menu(&state.message);
            let hwnd = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                class.as_ptr(),
                wide(&title).as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                900,
                660,
                owner,
                menu,
                wc.hInstance,
                (&*state as *const ReaderState).cast(),
            );
            if hwnd.is_null() {
                DestroyMenu(menu);
                return Err("Cannot create message viewing window".into());
            }
            let initial_dpi = dpi::window_dpi(hwnd);
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                dpi::scale(900, initial_dpi),
                dpi::scale(660, initial_dpi),
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let window = Self { state };
            window.state.render_html();
            if visible {
                ShowWindow(hwnd, SW_SHOW);
                window.activate();
                SetFocus(window.state.controls.body.get());
            }
            Ok(window)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.state.hwnd.get()
    }

    pub fn id(&self) -> &MessageId {
        &self.state.id
    }

    pub fn active(&self) -> bool {
        unsafe { !self.hwnd().is_null() && IsWindow(self.hwnd()) != 0 }
    }

    pub fn activate(&self) {
        if self.active() {
            unsafe {
                ShowWindow(self.hwnd(), SW_RESTORE);
                SetForegroundWindow(self.hwnd());
            }
        }
    }

    /// Handle shortcuts before dialog navigation consumes a child's key event.
    pub fn translate(&self, message: &MSG) -> bool {
        if !self.active()
            || message.message != WM_KEYDOWN
            || (message.hwnd != self.hwnd() && unsafe { IsChild(self.hwnd(), message.hwnd) } == 0)
        {
            return false;
        }
        if message.wParam == VK_ESCAPE as usize {
            self.close();
            return true;
        }
        let control = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT as i32) } < 0;
        let command = match message.wParam as u32 {
            0x52 if control && shift => Some(Command::ReplyAll),
            0x52 if control => Some(Command::Reply),
            0x46 if control => Some(Command::Forward),
            _ => None,
        };
        if let Some(command) = command {
            self.state.dispatch(command_id(command));
            true
        } else {
            false
        }
    }

    pub fn close(&self) {
        if self.active() {
            // Close COM controls while their parent and child HWNDs are live.
            self.state.release_renderer();
            unsafe { DestroyWindow(self.hwnd()) };
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.close();
        unsafe { ImageList_Destroy(self.state.icons.replace(0)) };
    }
}

impl ReaderState {
    fn release_renderer(&self) {
        let renderer = self.renderer.borrow_mut().take();
        // COM teardown may call the host's window procedure synchronously.
        // Release the RefCell guard before invoking the renderer's destructor.
        drop(renderer);
    }

    fn dispatch(&self, command: u16) {
        unsafe {
            PostMessageW(
                self.owner,
                READER_DISPATCH,
                command as usize,
                self.hwnd.get() as isize,
            );
        }
    }

    fn render_html(&self) {
        let Some(html) = &self.message.html else {
            self.update_status();
            return;
        };
        if self.renderer.borrow().is_none() {
            match unsafe { HtmlView::new(self.hwnd.get(), self.hwnd.get(), HTML_STATUS) } {
                Ok(view) => *self.renderer.borrow_mut() = Some(view),
                Err(error) => {
                    unsafe {
                        set_window_text(
                            self.controls.status.get(),
                            &format!("Cannot display HTML; showing plain text: {error}"),
                        )
                    };
                    self.layout();
                    return;
                }
            }
        }
        let document = HtmlDocument {
            html: html.clone(),
            inline_images: self
                .message
                .inline_images()
                .map(|part| InlineImage {
                    content_id: part.content_id.clone().unwrap_or_default(),
                    content_type: part.content_type.clone(),
                    bytes: part.bytes.clone(),
                })
                .collect(),
            remote_images: self.remote_images.get(),
        };
        let result = self
            .renderer
            .borrow()
            .as_ref()
            .expect("renderer initialized")
            .render(document);
        if let Err(error) = result {
            unsafe {
                set_window_text(
                    self.controls.status.get(),
                    &format!("Cannot display HTML; showing plain text: {error}"),
                )
            };
        } else {
            self.update_status();
        }
        self.layout();
    }

    fn update_status(&self) {
        let count = self.message.downloadable_attachments().count();
        let attachments = if count == 1 {
            "1 attachment".to_owned()
        } else {
            format!("{count} attachments")
        };
        let status = match self.renderer.borrow().as_ref().map(HtmlView::status) {
            Some(HtmlStatus::Loading) => format!("{attachments} · Loading HTML…"),
            Some(HtmlStatus::Failed(error)) => {
                format!("HTML unavailable; displaying plain text: {error}")
            }
            _ => attachments,
        };
        unsafe { set_window_text(self.controls.status.get(), &status) };
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if msg == WM_NCCREATE {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
            );
        }
        if msg == WM_NOTIFY
            && let Some(result) = classic::custom_draw(lp)
        {
            return result;
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ReaderState;
        let Some(state) = pointer.as_ref() else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        match msg {
            WM_CREATE => {
                state.setup(hwnd);
                0
            }
            WM_SIZE => {
                state.layout();
                0
            }
            WM_DPICHANGED => {
                state.update_dpi((wp & 0xffff) as u32);
                dpi::suggested(hwnd, lp);
                state.layout();
                0
            }
            WM_GETMINMAXINFO => {
                let dpi = state.metrics.borrow().dpi;
                (*(lp as *mut MINMAXINFO)).ptMinTrackSize = POINT {
                    x: dpi::scale(620, dpi),
                    y: dpi::scale(440, dpi),
                };
                0
            }
            HTML_STATUS => {
                state.update_status();
                state.layout();
                0
            }
            WM_COMMAND => {
                let id = (wp & 0xffff) as u16;
                match id {
                    SHOW_PICTURES => {
                        state.remote_images.set(RemoteImagePolicy::Allow);
                        state.render_html();
                    }
                    CLOSE_READER | 2 => {
                        state.release_renderer();
                        DestroyWindow(hwnd);
                    }
                    _ if matches!(
                        super::command_ids::decode(id),
                        Some(
                            Command::Reply
                                | Command::ReplyAll
                                | Command::Forward
                                | Command::Delete
                                | Command::SaveAttachments
                                | Command::Export
                        )
                    ) =>
                    {
                        state.dispatch(id)
                    }
                    _ => {}
                }
                0
            }
            WM_KEYDOWN if wp == VK_ESCAPE as usize => {
                state.release_renderer();
                DestroyWindow(hwnd);
                0
            }
            WM_CLOSE => {
                state.release_renderer();
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                state.release_renderer();
                0
            }
            WM_NCDESTROY => {
                state.hwnd.set(null_mut());
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
