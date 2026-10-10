//! Main-window layout state, expressed in logical pixels.
use super::{controls::MainWindowControls, toolbar_layout};
use crate::platform::dpi;
use windows_sys::Win32::UI::{Controls::*, WindowsAndMessaging::*};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DragTarget {
    Sidebar,
    Preview,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PreviewFormat {
    #[default]
    PlainText,
    Html,
}

pub(super) struct WindowLayout {
    pub sidebar: i32,
    pub preview_height: i32,
    pub show_folders: bool,
    pub show_preview: bool,
    pub drag: Option<DragTarget>,
    pub preview_format: PreviewFormat,
    pub blocked_images: bool,
}
impl Default for WindowLayout {
    fn default() -> Self {
        Self {
            sidebar: 260,
            preview_height: 280,
            show_folders: true,
            show_preview: true,
            drag: None,
            preview_format: PreviewFormat::default(),
            blocked_images: false,
        }
    }
}
impl WindowLayout {
    pub(super) unsafe fn arrange(
        &self,
        controls: &MainWindowControls,
        dpi_value: u32,
        width: i32,
        height: i32,
    ) {
        unsafe {
            let width = dpi::logical(width, dpi_value);
            let height = dpi::logical(height, dpi_value);
            let menu_height = if controls.menu_bar.is_null() { 0 } else { 26 };
            let filter_width = 174;
            let search_width = 150;
            let filter_left = (width - filter_width - 6).max(8);
            let search_left = if menu_height == 0 {
                (filter_left - search_width - 8).max(8)
            } else {
                (width - search_width - 6).max(8)
            };
            let buttons_right = if menu_height == 0 {
                search_left
            } else {
                filter_left
            };
            toolbar_layout::fit(controls.toolbar, dpi::scale(buttons_right - 8, dpi_value));
            dpi::move_control(dpi_value, controls.menu_bar, 0, 0, width, menu_height, 1);
            dpi::move_control(dpi_value, controls.toolbar, 0, menu_height, width, 32, 1);
            dpi::move_control(
                dpi_value,
                controls.search_host,
                search_left,
                2,
                search_width,
                22,
                1,
            );
            dpi::move_control(
                dpi_value,
                controls.filter,
                filter_left,
                menu_height + 3,
                filter_width,
                190,
                1,
            );
            // Toolbar backgrounds must stay below input siblings when they repaint.
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
            SetWindowPos(controls.toolbar, HWND_BOTTOM, 0, 0, 0, 0, flags);
            if !controls.menu_bar.is_null() {
                SetWindowPos(controls.menu_bar, HWND_TOP, 0, 0, 0, 0, flags);
            }
            SetWindowPos(controls.search_host, HWND_TOP, 0, 0, 0, 0, flags);
            SetWindowPos(controls.filter, HWND_TOP, 0, 0, 0, 0, flags);
            SetWindowLongPtrW(
                controls.toolbar,
                GWL_STYLE,
                GetWindowLongPtrW(controls.toolbar, GWL_STYLE) | WS_CLIPSIBLINGS as isize,
            );
            let left = if self.show_folders {
                self.sidebar.clamp(120, (width / 2).max(120))
            } else {
                0
            };
            let content = (width - left - 8).max(120);
            let bottom = (height - 24).max(120);
            let top = menu_height + 32;
            ShowWindow(
                controls.folder,
                if self.show_folders { SW_SHOW } else { SW_HIDE },
            );
            dpi::move_control(
                dpi_value,
                controls.folder,
                0,
                top,
                left,
                (bottom - top).max(80),
                1,
            );
            ShowWindow(
                controls.sidebar_splitter,
                if self.show_folders { SW_SHOW } else { SW_HIDE },
            );
            dpi::move_control(
                dpi_value,
                controls.sidebar_splitter,
                left,
                top,
                7,
                (bottom - top).max(80),
                1,
            );
            let split = if self.show_preview {
                (bottom - self.preview_height).clamp(top + 90, (bottom - 110).max(top + 90))
            } else {
                bottom
            };
            dpi::move_control(
                dpi_value,
                controls.list,
                left + 7,
                top,
                content,
                (split - top - 4).max(70),
                1,
            );
            let preview = if self.show_preview { SW_SHOW } else { SW_HIDE };
            let usable = (content - 20).max(100);
            let group_widths = [
                usable * 36 / 100,
                usable * 28 / 100,
                usable - usable * 36 / 100 - usable * 28 / 100,
            ];
            let label_widths = [36, 24, 50];
            let mut group_left = left + 12;
            for (index, (label, control)) in controls
                .labels
                .iter()
                .zip([
                    controls.preview_from,
                    controls.preview_to,
                    controls.preview_subject,
                ])
                .enumerate()
            {
                ShowWindow(*label, preview);
                ShowWindow(control, preview);
                dpi::move_control(
                    dpi_value,
                    *label,
                    group_left,
                    split + 8,
                    label_widths[index],
                    20,
                    1,
                );
                dpi::move_control(
                    dpi_value,
                    control,
                    group_left + label_widths[index],
                    split + 8,
                    (group_widths[index] - label_widths[index] - 8).max(10),
                    20,
                    1,
                );
                group_left += group_widths[index];
            }
            ShowWindow(
                controls.preview_body,
                if self.show_preview && self.preview_format == PreviewFormat::PlainText {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            ShowWindow(
                controls.preview_html,
                if self.show_preview && self.preview_format == PreviewFormat::Html {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            let images_blocked = self.show_preview && self.blocked_images;
            for window in [controls.image_notice, controls.show_pictures] {
                ShowWindow(window, if images_blocked { SW_SHOW } else { SW_HIDE });
            }
            dpi::move_control(
                dpi_value,
                controls.image_notice,
                left + 12,
                split + 34,
                (content - 154).max(80),
                26,
                1,
            );
            dpi::move_control(
                dpi_value,
                controls.show_pictures,
                left + content - 130,
                split + 33,
                126,
                24,
                1,
            );
            let body_top = split + 32 + if images_blocked { 30 } else { 0 };
            dpi::move_control(
                dpi_value,
                controls.preview_body,
                left + 7,
                body_top,
                content,
                (bottom - body_top).max(40),
                1,
            );
            dpi::move_control(
                dpi_value,
                controls.preview_html,
                left + 7,
                body_top,
                content,
                (bottom - body_top).max(40),
                1,
            );
            SendMessageW(controls.status, WM_SIZE, 0, 0);
            let parts = [
                dpi::scale((width - 340).max(200), dpi_value),
                dpi::scale((width - 150).max(300), dpi_value),
                -1,
            ];
            SendMessageW(controls.status, SB_SETPARTS, 3, parts.as_ptr() as isize);
        }
    }
}
