//! Account management and properties. Persistence belongs to the main window.
use crate::{
    Result,
    account::{Account, AccountId, AccountRecord, Authentication, Incoming, Security},
    platform::{
        controls::{set_window_text, window_text as text},
        dpi,
        strings::wide,
    },
};
use std::{
    cell::{Cell, RefCell},
    net::IpAddr,
    ptr::{null, null_mut},
    sync::mpsc::{self, Receiver, Sender},
};
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{
        COLOR_WINDOW, GetSysColorBrush, HDC, InvalidateRect, SetBkMode, SetTextColor, TRANSPARENT,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{EnableWindow, SetFocus},
        WindowsAndMessaging::*,
    },
};

pub(super) const ACCOUNT_LIST: u16 = 340;
pub(super) const ADD_ACCOUNT: u16 = 341;
const EDIT_ACCOUNT: u16 = 342;
const REMOVE_ACCOUNT: u16 = 343;
const DEFAULT_ACCOUNT: u16 = 344;
const SAVE_ACCOUNT: u16 = IDOK as u16;
pub(crate) const ACCOUNT_SUBMITTED: u32 = WM_APP + 3;
const SNAPSHOT_UPDATED: u32 = WM_APP + 30;
const PROPERTY_CLOSED: u32 = WM_APP + 31;

pub(super) unsafe fn setup_editor(manager: HWND) -> HWND {
    unsafe {
        let host = GetWindowLongPtrW(manager, GWLP_USERDATA) as *const manager::ManagerHost;
        if host.is_null() {
            null_mut()
        } else {
            (*host).property.get()
        }
    }
}

pub(super) unsafe fn setup_parent(manager: HWND) -> HWND {
    unsafe {
        let host = GetWindowLongPtrW(manager, GWLP_USERDATA) as *const manager::ManagerHost;
        if host.is_null() {
            null_mut()
        } else {
            (*host).embedded
        }
    }
}

unsafe fn present_setup(manager: HWND, editor: HWND, width: i32, height: i32) {
    unsafe {
        let parent = setup_parent(manager);
        if !parent.is_null() {
            SendMessageW(
                parent,
                super::setup::EMBEDDED,
                editor as usize,
                (width | (height << 16)) as isize,
            );
        }
    }
}

#[derive(Clone)]
pub(crate) struct AccountsSnapshot {
    pub(crate) accounts: Vec<AccountRecord>,
    pub(crate) default_account: Option<AccountId>,
}

#[derive(Clone)]
pub(crate) enum AccountChange {
    Add {
        name: String,
        account: Account,
    },
    Update {
        id: AccountId,
        name: String,
        account: Account,
    },
    Remove {
        id: AccountId,
    },
    SetDefault {
        id: AccountId,
    },
}

/// The owner persists a change during ACCOUNT_SUBMITTED and sends one snapshot
/// before returning. This channel never shares application-state pointers.
pub(crate) struct AccountSubmission {
    pub(crate) change: AccountChange,
    pub(crate) reply: Sender<Result<AccountsSnapshot>>,
}

struct AccountUpdate {
    snapshot: AccountsSnapshot,
    selected: Option<AccountId>,
}

mod form;
mod manager;
mod oauth;
mod properties;
#[cfg(test)]
mod tests;
mod wizard;

use crate::ui::shared::window::{WindowLayout, register_class};
use form::{Field, FormInput, ValidationError};
pub(crate) use manager::open;

unsafe fn submit_change(
    owner: HWND,
    submissions: &Sender<AccountSubmission>,
    change: AccountChange,
) -> Result<AccountsSnapshot> {
    unsafe {
        let (reply, response) = mpsc::channel();
        submissions
            .send(AccountSubmission { change, reply })
            .map_err(|_| "The main window is no longer accepting account changes.".to_owned())?;
        SendMessageW(owner, ACCOUNT_SUBMITTED, 0, 0);
        response
            .try_recv()
            .map_err(|_| "The main window did not confirm the account change.".to_owned())?
    }
}

unsafe fn error_brush(dc: usize) -> isize {
    unsafe {
        SetTextColor(dc as HDC, 0x002828b4);
        SetBkMode(dc as HDC, TRANSPARENT as i32);
        GetSysColorBrush(COLOR_WINDOW) as isize
    }
}
