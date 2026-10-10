//! Native read-only controls and DPI-aware layout for message windows.
use super::*;
use crate::platform::icons;
use windows_sys::Win32::System::SystemServices::SS_LEFT;

impl ReaderState {
    pub(super) unsafe fn setup(&self, hwnd: HWND) {
        unsafe {
            self.hwnd.set(hwnd);
            let has_attachments = self.message.downloadable_attachments().next().is_some();
            let (toolbar, icons) = classic::toolbar(
                hwnd,
                &[
                    (
                        "Reply",
                        command_id(Command::Reply),
                        icons::ToolbarIcon::Reply,
                        true,
                    ),
                    (
                        "Reply All",
                        command_id(Command::ReplyAll),
                        icons::ToolbarIcon::ReplyAll,
                        true,
                    ),
                    (
                        "Forward",
                        command_id(Command::Forward),
                        icons::ToolbarIcon::Forward,
                        true,
                    ),
                    (
                        "Delete",
                        command_id(Command::Delete),
                        icons::ToolbarIcon::Delete,
                        true,
                    ),
                    (
                        "Save Attachments",
                        command_id(Command::SaveAttachments),
                        icons::ToolbarIcon::Attach,
                        has_attachments,
                    ),
                    (
                        "Use WebView",
                        SWITCH_RENDERER,
                        icons::ToolbarIcon::Find,
                        self.message.html.is_some(),
                    ),
                    ("Details", SHOW_DETAILS, icons::ToolbarIcon::Contacts, true),
                    ("Copy text", COPY_BODY, icons::ToolbarIcon::Copy, true),
                ],
            );
            self.controls.toolbar.set(toolbar);
            // Keep the complete command names as native tooltips, with compact
            // icons for common mail actions so the new controls fit small windows.
            SendMessageW(
                toolbar,
                TB_SETEXTENDEDSTYLE,
                0,
                TBSTYLE_EX_MIXEDBUTTONS as isize,
            );
            for command in [
                Command::ReplyAll,
                Command::Forward,
                Command::Delete,
                Command::SaveAttachments,
            ] {
                let info = TBBUTTONINFOW {
                    cbSize: std::mem::size_of::<TBBUTTONINFOW>() as u32,
                    dwMask: TBIF_STYLE,
                    fsStyle: BTNS_AUTOSIZE as u8,
                    ..Default::default()
                };
                SendMessageW(
                    toolbar,
                    TB_SETBUTTONINFOW,
                    command_id(command) as usize,
                    &info as *const _ as isize,
                );
            }
            self.icons.set(icons);
            for (name, value, id) in [
                ("Subject:", &self.message.summary.subject, SUBJECT),
                ("From:", &self.message.summary.from, FROM),
                ("To:", &self.message.summary.to, TO),
                ("Date:", &self.message.summary.date, DATE),
            ] {
                let label = control(hwnd, "STATIC", name, SS_LEFT, 0);
                let value = control(
                    hwnd,
                    "EDIT",
                    value,
                    ES_READONLY as u32 | ES_AUTOHSCROLL as u32 | WS_TABSTOP,
                    id,
                );
                // These are selectable message metadata, not form inputs.
                SetWindowLongW(
                    value,
                    GWL_STYLE,
                    GetWindowLongW(value, GWL_STYLE) & !(WS_BORDER as i32),
                );
                SetWindowLongW(
                    value,
                    GWL_EXSTYLE,
                    GetWindowLongW(value, GWL_EXSTYLE) & !(WS_EX_CLIENTEDGE as i32),
                );
                SetWindowTheme(value, wide("").as_ptr(), wide("").as_ptr());
                SetWindowPos(
                    value,
                    null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
                self.controls.headers.borrow_mut().push((label, value));
            }
            let body = control(
                hwnd,
                "EDIT",
                "",
                ES_READONLY as u32
                    | ES_MULTILINE as u32
                    | ES_AUTOVSCROLL as u32
                    | ES_NOHIDESEL as u32
                    | WS_VSCROLL
                    | WS_TABSTOP,
                BODY,
            );
            SendMessageW(body, EM_SETLIMITTEXT, 50 * 1024 * 1024, 0);
            set_window_text(
                body,
                &self
                    .message
                    .body
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .replace('\n', "\r\n"),
            );
            self.controls.body.set(body);
            self.controls.summary.set(control(
                hwnd,
                "STATIC",
                &format!(
                    "{}   ·   {}",
                    super::super::message_list::sender(&self.message.summary.from),
                    super::super::message_list::received(&self.message.summary.date)
                ),
                SS_LEFT,
                0,
            ));
            self.controls
                .loading
                .set(control(hwnd, "STATIC", "Loading message…", SS_LEFT, 0));
            self.controls.blocked.set(control(
                hwnd,
                "STATIC",
                "External pictures have been blocked to protect your privacy.",
                SS_LEFT,
                0,
            ));
            self.controls.pictures.set(control(
                hwnd,
                "BUTTON",
                "Show pictures",
                BS_PUSHBUTTON as u32 | WS_TABSTOP,
                SHOW_PICTURES,
            ));
            self.controls
                .status
                .set(control(hwnd, "STATIC", "", SS_LEFT, 0));
            self.update_dpi(dpi::window_dpi(hwnd));
            self.layout();
        }
    }

    pub(super) unsafe fn update_dpi(&self, value: u32) {
        unsafe {
            self.metrics.borrow_mut().update(self.hwnd.get(), value);
            if let Some((_, subject)) = self.controls.headers.borrow().first() {
                self.metrics
                    .borrow_mut()
                    .apply_font(*subject, dpi::FontWeight::Bold);
            }
            let icons = classic::resize_toolbar(self.controls.toolbar.get(), value);
            ImageList_Destroy(self.icons.replace(icons));
        }
    }

    pub(super) fn layout(&self) {
        unsafe {
            let dpi = self.metrics.borrow().dpi;
            let mut rect = RECT::default();
            GetClientRect(self.hwnd.get(), &mut rect);
            let width = dpi::logical(rect.right, dpi);
            let height = dpi::logical(rect.bottom, dpi);
            let (html, show_html, blocked, loading) = self
                .renderer
                .borrow()
                .as_ref()
                .map(|view| {
                    let status = view.status();
                    if status == HtmlStatus::Ready {
                        self.rendered.set(true);
                    }
                    let loading = status == HtmlStatus::Loading;
                    (
                        view.hwnd(),
                        status == HtmlStatus::Ready || (loading && self.rendered.get()),
                        view.has_blocked_images(),
                        loading,
                    )
                })
                .unwrap_or((null_mut(), false, false, false));
            let blocked = blocked && self.remote_images.get() == RemoteImagePolicy::Block;
            dpi::move_control(dpi, self.controls.toolbar.get(), 0, 0, width, 30, 1);
            for (row, (label, value)) in self.controls.headers.borrow().iter().enumerate() {
                let visible = row == 0 || self.details.get();
                ShowWindow(
                    *label,
                    if visible && row != 0 {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
                ShowWindow(*value, if visible { SW_SHOW } else { SW_HIDE });
                let y = if row == 0 {
                    38
                } else {
                    66 + (row as i32 - 1) * 22
                };
                dpi::move_control(dpi, *label, 16, y + 2, 44, 21, 1);
                let x = if row == 0 { 16 } else { 66 };
                dpi::move_control(
                    dpi,
                    *value,
                    x,
                    y,
                    (width - x - 16).max(80),
                    if row == 0 { 25 } else { 22 },
                    1,
                );
            }
            let header_bottom = if self.details.get() { 140 } else { 92 };
            ShowWindow(
                self.controls.summary.get(),
                if self.details.get() { SW_HIDE } else { SW_SHOW },
            );
            dpi::move_control(
                dpi,
                self.controls.summary.get(),
                16,
                66,
                (width - 32).max(80),
                23,
                1,
            );
            let header = RECT {
                top: dpi::scale(30, dpi),
                bottom: dpi::scale(145, dpi),
                ..rect
            };
            InvalidateRect(self.hwnd.get(), &header, 1);
            ShowWindow(
                self.controls.blocked.get(),
                if blocked { SW_SHOW } else { SW_HIDE },
            );
            ShowWindow(
                self.controls.pictures.get(),
                if blocked { SW_SHOW } else { SW_HIDE },
            );
            dpi::move_control(
                dpi,
                self.controls.blocked.get(),
                12,
                header_bottom + 4,
                (width - 156).max(80),
                28,
                1,
            );
            dpi::move_control(
                dpi,
                self.controls.pictures.get(),
                (width - 132).max(0),
                header_bottom,
                120,
                29,
                1,
            );
            let top = header_bottom + if blocked { 36 } else { 0 };
            let body_height = (height - top - 26).max(40);
            let plain = self.controls.body.get();
            ShowWindow(
                plain,
                if show_html || loading {
                    SW_HIDE
                } else {
                    SW_SHOW
                },
            );
            ShowWindow(
                self.controls.loading.get(),
                if loading && !show_html {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            dpi::move_control(
                dpi,
                self.controls.loading.get(),
                14,
                top + 12,
                (width - 28).max(80),
                28,
                1,
            );
            dpi::move_control(dpi, plain, 2, top, (width - 4).max(80), body_height, 1);
            if !html.is_null() {
                ShowWindow(html, if show_html { SW_SHOW } else { SW_HIDE });
                dpi::move_control(dpi, html, 2, top, (width - 4).max(80), body_height, 1);
                if show_html && GetFocus() == plain {
                    SetFocus(html);
                }
            }
            dpi::move_control(
                dpi,
                self.controls.status.get(),
                8,
                height - 22,
                (width - 16).max(80),
                20,
                1,
            );
        }
    }
}
