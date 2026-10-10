use super::folder_tree::TreeItem;
use windows_sys::Win32::Foundation::HWND;

/// Borrowed child-window handles. Their parent window owns their lifetime.
#[derive(Default)]
pub(super) struct MainWindowControls {
    pub main: HWND,
    pub list: HWND,
    pub folder: HWND,
    pub sidebar_splitter: HWND,
    pub preview_body: HWND,
    pub preview_html: HWND,
    pub image_notice: HWND,
    pub show_pictures: HWND,
    pub preview_from: HWND,
    pub preview_to: HWND,
    pub preview_subject: HWND,
    pub status: HWND,
    pub labels: Vec<HWND>,
    pub toolbar: HWND,
    pub menu_bar: HWND,
    pub search: HWND,
    pub filter: HWND,
    pub search_host: HWND,
    pub tree_items: Vec<TreeItem>,
}
