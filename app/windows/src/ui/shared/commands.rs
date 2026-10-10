//! Translate native menu and toolbar IDs at the UI boundary.
use crate::app::Command;
use crate::app::SortColumn;

pub(in crate::ui) const LIST: u16 = 100;
pub(in crate::ui) const FOLDER: u16 = 110;
pub(in crate::ui) const SEARCH: u16 = 119;
pub(in crate::ui) const FILTER: u16 = 120;
// Reserved IDs for visible toolbar actions that are not implemented yet.
pub(in crate::ui) const PRINT: u16 = 114;
pub(in crate::ui) const CONTACTS: u16 = 115;

pub(in crate::ui) fn sort_column(index: i32) -> Option<SortColumn> {
    Some(match index {
        0 => SortColumn::Priority,
        1 => SortColumn::Attachment,
        2 => SortColumn::Flag,
        3 => SortColumn::From,
        4 => SortColumn::Subject,
        5 => SortColumn::Received,
        _ => return None,
    })
}

pub(in crate::ui) const fn command_id(command: Command) -> u16 {
    match command {
        Command::NewMessage => 101,
        Command::OpenMessage => 124,
        Command::SaveAttachments => 125,
        Command::ShowPictures => 126,
        Command::MarkRead => 127,
        Command::MarkUnread => 128,
        Command::ToggleFlag => 129,
        Command::Import => 102,
        Command::Export => 103,
        Command::Attach => 104,
        Command::ConfigureAccount => 105,
        Command::Receive => 106,
        Command::CancelSync => 130,
        Command::SaveDraft => 107,
        Command::Send => 108,
        Command::Delete => 109,
        Command::Restore => 121,
        Command::DeletePermanently => 122,
        Command::EmptyTrash => 123,
        Command::Reply => 111,
        Command::Forward => 112,
        Command::ReplyAll => 113,
        Command::Find => 118,
        Command::About => 180,
        Command::Exit => 181,
        Command::ToggleFolders => 182,
        Command::TogglePreview => 183,
        Command::Refresh => 184,
        Command::CloseComposer => 190,
    }
}

pub(in crate::ui) fn decode(id: u16) -> Option<Command> {
    Some(match id {
        101 => Command::NewMessage,
        124 => Command::OpenMessage,
        125 => Command::SaveAttachments,
        126 => Command::ShowPictures,
        127 => Command::MarkRead,
        128 => Command::MarkUnread,
        129 => Command::ToggleFlag,
        102 => Command::Import,
        103 => Command::Export,
        104 => Command::Attach,
        105 => Command::ConfigureAccount,
        106 => Command::Receive,
        130 => Command::CancelSync,
        107 => Command::SaveDraft,
        108 => Command::Send,
        109 => Command::Delete,
        121 => Command::Restore,
        122 => Command::DeletePermanently,
        123 => Command::EmptyTrash,
        111 => Command::Reply,
        112 => Command::Forward,
        113 => Command::ReplyAll,
        118 => Command::Find,
        180 => Command::About,
        181 => Command::Exit,
        182 => Command::ToggleFolders,
        183 => Command::TogglePreview,
        184 => Command::Refresh,
        190 => Command::CloseComposer,
        _ => return None,
    })
}

/// Native notifications exchanged by main, compose, and reader windows.
pub(crate) const DISPATCH: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 10;
pub(crate) const READER_DISPATCH: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 12;
pub(crate) const HTML_STATUS: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 11;
