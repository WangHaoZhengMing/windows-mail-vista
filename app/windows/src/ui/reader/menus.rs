//! Menus for an independent message reading window.
use super::*;

pub(super) unsafe fn reader_menu(message: &Message) -> HMENU {
    unsafe {
        let menu = CreateMenu();
        let file = CreatePopupMenu();
        AppendMenuW(
            file,
            MF_STRING,
            command_id(Command::Export) as usize,
            wide("Export Message…").as_ptr(),
        );
        AppendMenuW(
            file,
            MF_STRING
                | if message.downloadable_attachments().next().is_some() {
                    0
                } else {
                    MF_GRAYED
                },
            command_id(Command::SaveAttachments) as usize,
            wide("Save Attachments…").as_ptr(),
        );
        AppendMenuW(file, MF_SEPARATOR, 0, null());
        AppendMenuW(
            file,
            MF_STRING,
            CLOSE_READER as usize,
            wide("Close\tEsc").as_ptr(),
        );
        AppendMenuW(menu, MF_POPUP, file as usize, wide("&File").as_ptr());
        let actions = CreatePopupMenu();
        for (label, command) in [
            ("Reply\tCtrl+R", Command::Reply),
            ("Reply All\tCtrl+Shift+R", Command::ReplyAll),
            ("Forward\tCtrl+F", Command::Forward),
            ("Delete", Command::Delete),
        ] {
            AppendMenuW(
                actions,
                MF_STRING,
                command_id(command) as usize,
                wide(label).as_ptr(),
            );
        }
        AppendMenuW(menu, MF_POPUP, actions as usize, wide("&Message").as_ptr());
        menu
    }
}
