//! Native About dialog with the application identity and build version.
use crate::platform::{dpi, icons, strings::wide};
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::HWND,
    UI::{Controls::*, WindowsAndMessaging::*},
};

pub(super) unsafe fn show(owner: HWND) {
    unsafe {
        let title = wide("About Windows Mail");
        let heading = wide("Windows Mail");
        let content = wide(&format!(
            "Version {}\n\nA desktop email client with a classic Windows interface.\n\nRead and organize mail, manage multiple accounts, and compose messages with attachments. Supports IMAP, POP3 and SMTP.\n\nBuilt with Rust.",
            env!("CARGO_PKG_VERSION")
        ));
        let footer = wide("An independent project inspired by classic Windows Mail.");
        let icon = icons::icon(dpi::scale(48, dpi::window_dpi(owner)));
        let config = TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: owner,
            dwFlags: TDF_USE_HICON_MAIN | TDF_POSITION_RELATIVE_TO_WINDOW | TDF_SIZE_TO_CONTENT,
            dwCommonButtons: TDCBF_OK_BUTTON,
            pszWindowTitle: title.as_ptr(),
            Anonymous1: TASKDIALOGCONFIG_0 { hMainIcon: icon },
            pszMainInstruction: heading.as_ptr(),
            pszContent: content.as_ptr(),
            pszFooter: footer.as_ptr(),
            cxWidth: 320,
            ..std::mem::zeroed()
        };
        let module = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(
            wide("comctl32.dll").as_ptr(),
        );
        let function = windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            module,
            c"TaskDialogIndirect".as_ptr().cast(),
        );
        let result = if let Some(function) = function {
            let show: unsafe extern "system" fn(
                *const TASKDIALOGCONFIG,
                *mut i32,
                *mut i32,
                *mut i32,
            ) -> i32 = std::mem::transmute(function);
            show(&config, null_mut(), null_mut(), null_mut())
        } else {
            -1
        };
        if result < 0 {
            MessageBoxW(
                owner,
                content.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
        if !icon.is_null() {
            DestroyIcon(icon);
        }
    }
}
