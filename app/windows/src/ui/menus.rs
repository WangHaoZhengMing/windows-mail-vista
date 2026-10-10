//! Native menu descriptions for the main and compose windows.
use super::command_ids::command_id;
use crate::{app::Command, platform::strings::wide};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub(crate) const CUT: usize = 170;
pub(crate) const COPY: usize = 171;
pub(crate) const PASTE: usize = 172;
pub(crate) const SELECT_ALL: usize = 173;
pub(crate) const TOGGLE_BCC: usize = 174;
pub(crate) const PLAIN_TEXT: usize = 175;
pub(crate) const SETTINGS: usize = 184;
pub const RICH_TEXT: usize = 176;
const SPELLING: usize = 177;

fn id(command: Command) -> usize {
    command_id(command) as usize
}

pub(crate) unsafe fn menu(composer: bool) -> HMENU {
    unsafe {
        let bar = CreateMenu();
        type MenuSection<'a> = (&'a str, Vec<(&'a str, usize, bool)>);
        let sections: Vec<MenuSection<'_>> = if composer {
            vec![
                (
                    "&File",
                    vec![
                        ("&Send\tCtrl+Enter", id(Command::Send), true),
                        ("&Save\tCtrl+S", id(Command::SaveDraft), true),
                        ("Close", id(Command::CloseComposer), true),
                    ],
                ),
                (
                    "&Edit",
                    vec![
                        ("Cut\tCtrl+X", CUT, true),
                        ("Copy\tCtrl+C", COPY, true),
                        ("Paste\tCtrl+V", PASTE, true),
                        ("Select All\tCtrl+A", SELECT_ALL, true),
                    ],
                ),
                ("&View", vec![("All Headers (Bcc)", TOGGLE_BCC, true)]),
                (
                    "&Insert",
                    vec![("File Attachment...", id(Command::Attach), true)],
                ),
                (
                    "F&ormat",
                    vec![
                        ("Plain Text", PLAIN_TEXT, true),
                        ("Rich Text (HTML)", RICH_TEXT, true),
                    ],
                ),
                (
                    "&Tools",
                    vec![
                        ("Accounts...", id(Command::ConfigureAccount), true),
                        ("Settings...", SETTINGS, true),
                        ("Spelling", SPELLING, false),
                    ],
                ),
                ("&Message", vec![("Send", id(Command::Send), true)]),
                (
                    "&Help",
                    vec![("About Windows Mail", id(Command::About), true)],
                ),
            ]
        } else {
            vec![
                (
                    "&File",
                    vec![
                        ("New Message\tCtrl+N", id(Command::NewMessage), true),
                        ("Open Message\tEnter", id(Command::OpenMessage), true),
                        ("Import Messages...", id(Command::Import), true),
                        ("Export Message...", id(Command::Export), true),
                        ("Save Attachments...", id(Command::SaveAttachments), true),
                        ("Exit", id(Command::Exit), true),
                    ],
                ),
                (
                    "&Edit",
                    vec![
                        ("Delete\tDel", id(Command::Delete), true),
                        ("Restore from Deleted Items", id(Command::Restore), true),
                        (
                            "Delete Permanently\tShift+Del",
                            id(Command::DeletePermanently),
                            true,
                        ),
                        ("Empty Deleted Items...", id(Command::EmptyTrash), true),
                        ("Mark as Read", id(Command::MarkRead), true),
                        ("Mark as Unread", id(Command::MarkUnread), true),
                        ("Toggle Flag", id(Command::ToggleFlag), true),
                        ("Select All\tCtrl+A", SELECT_ALL, true),
                    ],
                ),
                (
                    "&View",
                    vec![
                        ("Folder List", id(Command::ToggleFolders), true),
                        ("Preview Pane", id(Command::TogglePreview), true),
                        ("Refresh\tF5", id(Command::Refresh), true),
                    ],
                ),
                (
                    "&Tools",
                    vec![
                        ("Send/Receive\tCtrl+M", id(Command::Receive), true),
                        ("Stop synchronization", id(Command::CancelSync), false),
                        ("Accounts...", id(Command::ConfigureAccount), true),
                        ("Settings...", SETTINGS, true),
                    ],
                ),
                (
                    "&Message",
                    vec![
                        ("New Message", id(Command::NewMessage), true),
                        ("Reply\tCtrl+R", id(Command::Reply), true),
                        ("Reply All\tCtrl+Shift+R", id(Command::ReplyAll), true),
                        ("Forward\tCtrl+F", id(Command::Forward), true),
                    ],
                ),
                (
                    "&Help",
                    vec![("About Windows Mail", id(Command::About), true)],
                ),
            ]
        };
        for (name, items) in sections {
            let popup = CreatePopupMenu();
            for (label, id, enabled) in items {
                AppendMenuW(
                    popup,
                    MF_STRING | if enabled { 0 } else { MF_GRAYED },
                    id,
                    wide(label).as_ptr(),
                );
            }
            AppendMenuW(bar, MF_POPUP, popup as usize, wide(name).as_ptr());
        }
        bar
    }
}
