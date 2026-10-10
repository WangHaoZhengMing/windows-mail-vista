//! Account management and properties. Persistence belongs to the main window.
use crate::{
    Result,
    account::{Account, AccountId, AccountRecord, Authentication, Incoming, Security},
    platform::{
        controls::{account_control as child, set_window_text, window_text as text},
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
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND},
    Graphics::Gdi::{
        COLOR_WINDOW, GetSysColorBrush, HBRUSH, HDC, InvalidateRect, SetBkMode, SetTextColor,
        TRANSPARENT,
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
pub(super) mod settings;
#[cfg(test)]
mod tests;

use form::{Field, FormInput, ValidationError};
pub(crate) use manager::open;

struct ControlBounds {
    window: HWND,
    rect: [i32; 4],
}

#[derive(Default)]
struct WindowLayout {
    metrics: dpi::Resources,
    bounds: Vec<ControlBounds>,
}

impl WindowLayout {
    unsafe fn control(
        &mut self,
        owner: HWND,
        class: &str,
        caption: &str,
        style: u32,
        id: u16,
        rect: [i32; 4],
    ) -> HWND {
        unsafe {
            let window = child(owner, class, caption, style, id);
            self.bounds.push(ControlBounds { window, rect });
            let [x, y, width, height] = rect;
            dpi::move_control(self.metrics.dpi, window, x, y, width, height, 0);
            window
        }
    }

    unsafe fn update(&mut self, window: HWND, value: u32) {
        unsafe {
            self.metrics.update(window, value);
            for bounds in &self.bounds {
                let [x, y, width, height] = bounds.rect;
                dpi::move_control(self.metrics.dpi, bounds.window, x, y, width, height, 1);
            }
        }
    }
}

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

unsafe fn register_class(class: &[u16], procedure: WNDPROC) -> Result<()> {
    unsafe {
        let definition = WNDCLASSW {
            lpfnWndProc: procedure,
            hInstance: GetModuleHandleW(null()),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&definition) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(format!(
                "Could not register account window: {}",
                GetLastError()
            ));
        }
        Ok(())
    }
}

unsafe fn error_brush(dc: usize) -> isize {
    unsafe {
        SetTextColor(dc as HDC, 0x002828b4);
        SetBkMode(dc as HDC, TRANSPARENT as i32);
        GetSysColorBrush(COLOR_WINDOW) as isize
    }
}
