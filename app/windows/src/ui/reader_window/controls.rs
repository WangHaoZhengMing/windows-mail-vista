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
                ],
            );
            self.controls.toolbar.set(toolbar);
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
            let (html, show_html, blocked) = self
                .renderer
                .borrow()
                .as_ref()
                .map(|view| {
                    (
                        view.hwnd(),
                        view.status() == HtmlStatus::Ready,
                        view.has_blocked_images(),
                    )
                })
                .unwrap_or((null_mut(), false, false));
            let blocked = blocked && self.remote_images.get() == RemoteImagePolicy::Block;
            dpi::move_control(dpi, self.controls.toolbar.get(), 0, 0, width, 30, 1);
            for (row, (label, value)) in self.controls.headers.borrow().iter().enumerate() {
                let y = 36 + row as i32 * 27;
                dpi::move_control(dpi, *label, 12, y + 4, 72, 21, 1);
                dpi::move_control(dpi, *value, 84, y, (width - 96).max(80), 25, 1);
            }
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
                149,
                (width - 156).max(80),
                28,
                1,
            );
            dpi::move_control(
                dpi,
                self.controls.pictures.get(),
                (width - 132).max(0),
                145,
                120,
                29,
                1,
            );
            let top = if blocked { 181 } else { 145 };
            let body_height = (height - top - 26).max(40);
            let plain = self.controls.body.get();
            ShowWindow(plain, if show_html { SW_HIDE } else { SW_SHOW });
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
