//! Native file-picker boundary.
use super::strings::wide;
use std::path::PathBuf;
use windows_sys::Win32::{Foundation::HWND, UI::Controls::Dialogs::*};
/// Show the native file picker with a double-zero-terminated filter string.
///
/// # Safety
/// `owner` must be a live window on the calling UI thread. The caller must allow
/// synchronous window message reentry while the modal dialog runs.
pub unsafe fn file_dialog(owner: HWND, save: bool, filter: &str) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    unsafe {
        let mut filename = vec![0u16; 32768];
        let filter = wide(filter);
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = owner;
        ofn.lpstrFile = filename.as_mut_ptr();
        ofn.nMaxFile = filename.len() as u32;
        ofn.lpstrFilter = filter.as_ptr();
        ofn.Flags = OFN_EXPLORER
            | OFN_NOCHANGEDIR
            | OFN_PATHMUSTEXIST
            | if save {
                OFN_OVERWRITEPROMPT
            } else {
                OFN_FILEMUSTEXIST
            };
        if (if save {
            GetSaveFileNameW(&mut ofn)
        } else {
            GetOpenFileNameW(&mut ofn)
        }) == 0
        {
            return None;
        }
        let end = filename
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(filename.len());
        Some(std::ffi::OsString::from_wide(&filename[..end]).into())
    }
}
