//! Settings navigation and persisted desktop layout preferences.
use crate::platform::{dpi, strings::wide};
use crate::ui::shared::window::{WindowLayout, register_class};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::HWND,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, WindowsAndMessaging::*},
};
mod preferences;
pub(crate) use preferences::Preferences;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::SystemServices::SS_NOPREFIX;

pub(crate) const CHANGED: u32 = WM_APP + 35;
const NAV: u16 = 700;
const APPLY: u16 = 701;
const ACCOUNTS: u16 = 702;
const ABOUT: u16 = 703;

struct Settings {
    owner: HWND,
    layout: WindowLayout,
    pages: [Vec<HWND>; 4],
    folders: HWND,
    preview: HWND,
    dark_mode: HWND,
}
impl Settings {
    unsafe fn build(&mut self, window: HWND) {
        unsafe {
            let nav = self.layout.control(
                window,
                "LISTBOX",
                "",
                WS_TABSTOP | LBS_NOTIFY as u32 | LBS_OWNERDRAWFIXED as u32 | LBS_HASSTRINGS as u32,
                NAV,
                [12, 20, 160, 292],
            );
            // Navigation is part of the sidebar, without the account form's inset border.
            SetWindowLongPtrW(
                nav,
                GWL_EXSTYLE,
                GetWindowLongPtrW(nav, GWL_EXSTYLE) & !(WS_EX_CLIENTEDGE as isize),
            );
            SetWindowPos(
                nav,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            for caption in ["Appearance", "Accounts", "Storage", "About"] {
                SendMessageW(nav, LB_ADDSTRING, 0, wide(caption).as_ptr() as isize);
            }
            SendMessageW(
                nav,
                LB_SETITEMHEIGHT,
                0,
                dpi::scale(34, dpi::window_dpi(window)) as isize,
            );
            SendMessageW(nav, LB_SETCURSEL, 0, 0);
            let preferences = Preferences::load();
            self.page_control(
                window,
                0,
                "STATIC",
                "Appearance & layout",
                0,
                0,
                [196, 24, 380, 28],
            );
            self.page_control(
                window,
                0,
                "STATIC",
                "Choose the panes shown in the main window.",
                0,
                0,
                [196, 64, 380, 36],
            );
            self.folders = self.page_control(
                window,
                0,
                "BUTTON",
                "Show folder sidebar",
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                710,
                [196, 116, 380, 28],
            );
            self.preview = self.page_control(
                window,
                0,
                "BUTTON",
                "Show message preview",
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                711,
                [196, 154, 380, 28],
            );
            SendMessageW(
                self.folders,
                BM_SETCHECK,
                usize::from(preferences.folders),
                0,
            );
            SendMessageW(
                self.preview,
                BM_SETCHECK,
                usize::from(preferences.preview),
                0,
            );
            self.dark_mode = self.page_control(
                window,
                0,
                "BUTTON",
                "Use dark mode",
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                713,
                [196, 192, 380, 28],
            );
            SendMessageW(
                self.dark_mode,
                BM_SETCHECK,
                usize::from(preferences.dark_mode),
                0,
            );
            self.page_control(window, 0, "STATIC", "Changes apply when you select Save changes.\nYour choices are remembered for the next launch.", 0, 0, [196, 238, 380, 54]);
            self.page_control(
                window,
                1,
                "STATIC",
                "Mail accounts",
                0,
                0,
                [196, 24, 380, 28],
            );
            self.page_control(
                window,
                1,
                "STATIC",
                "Add an email account, update server settings, or choose your default account.",
                0,
                0,
                [196, 64, 380, 60],
            );
            self.page_control(
                window,
                1,
                "BUTTON",
                "Manage accounts...",
                WS_TABSTOP,
                ACCOUNTS,
                [196, 144, 180, 30],
            );
            self.page_control(window, 1, "STATIC", "Account settings are saved in the account manager.\nRemoving an account keeps downloaded mail in Local Folders.", 0, 0, [196, 220, 380, 72]);
            self.page_control(
                window,
                2,
                "STATIC",
                "Local storage",
                0,
                0,
                [196, 24, 380, 28],
            );
            self.page_control(
                window,
                2,
                "STATIC",
                "Mail, drafts, attachments and account data are stored on this computer.",
                0,
                0,
                [196, 64, 380, 54],
            );
            let path = crate::store::default_root().display().to_string();
            self.page_control(
                window,
                2,
                "EDIT",
                &path,
                WS_TABSTOP | ES_READONLY as u32 | ES_MULTILINE as u32,
                712,
                [196, 134, 380, 64],
            );
            self.page_control(
                window,
                2,
                "STATIC",
                "Local Folders holds imported mail and mail retained after an account is removed.",
                0,
                0,
                [196, 230, 380, 60],
            );
            self.page_control(
                window,
                3,
                "STATIC",
                "Windows Mail Vista",
                0,
                0,
                [196, 24, 380, 28],
            );
            self.page_control(window, 3, "STATIC", &format!("Version {}\n\nA Rust desktop email client with a classic Windows interface. Supports IMAP, POP3 and SMTP.", env!("CARGO_PKG_VERSION")), 0, 0, [196, 64, 380, 110]);
            self.page_control(
                window,
                3,
                "BUTTON",
                "About Windows Mail Vista...",
                WS_TABSTOP,
                ABOUT,
                [196, 210, 190, 30],
            );
            self.layout.control(
                window,
                "BUTTON",
                "Save changes",
                WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
                APPLY,
                [342, 334, 138, 30],
            );
            self.layout.control(
                window,
                "BUTTON",
                "Cancel",
                WS_TABSTOP,
                IDCANCEL as u16,
                [492, 334, 84, 30],
            );
            self.layout.update(window, dpi::window_dpi(window));
            for page in &self.pages {
                if let Some(heading) = page.first() {
                    self.layout
                        .metrics
                        .apply_font(*heading, dpi::FontWeight::Bold);
                }
            }
            self.select(0);
        }
    }
    unsafe fn page_control(
        &mut self,
        window: HWND,
        page: usize,
        class: &str,
        label: &str,
        style: u32,
        id: u16,
        bounds: [i32; 4],
    ) -> HWND {
        let style = if class == "STATIC" {
            style | SS_NOPREFIX
        } else {
            style
        };
        let control = unsafe { self.layout.control(window, class, label, style, id, bounds) };
        self.pages[page].push(control);
        control
    }
    unsafe fn select(&self, page: usize) {
        unsafe {
            for (index, controls) in self.pages.iter().enumerate() {
                for control in controls {
                    ShowWindow(*control, if index == page { SW_SHOW } else { SW_HIDE });
                }
            }
        }
    }
}

pub(crate) unsafe fn open(owner: HWND) {
    unsafe {
        let class = wide("WinMailRustSettings");
        let mut existing = FindWindowExW(null_mut(), null_mut(), class.as_ptr(), null());
        while !existing.is_null() {
            if GetWindow(existing, GW_OWNER) == owner {
                SetForegroundWindow(existing);
                return;
            }
            existing = FindWindowExW(null_mut(), existing, class.as_ptr(), null());
        }
        if register_class(&class, Some(procedure)).is_err() {
            return;
        }
        let mut state = Box::new(Settings {
            owner,
            layout: WindowLayout::default(),
            pages: std::array::from_fn(|_| Vec::new()),
            folders: null_mut(),
            preview: null_mut(),
            dark_mode: null_mut(),
        });
        let window = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide("Settings — Windows Mail Vista").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            dpi::scale(610, dpi::window_dpi(owner)),
            dpi::scale(412, dpi::window_dpi(owner)),
            owner,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if window.is_null() {
            return;
        }
        SetWindowLongPtrW(
            window,
            GWLP_USERDATA,
            (&mut *state as *mut Settings) as isize,
        );
        state.build(window);
        let _ = Box::into_raw(state); // WM_NCDESTROY owns the allocation.
        ShowWindow(window, SW_SHOW);
    }
}
unsafe extern "system" fn procedure(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Settings;
        if pointer.is_null() {
            return DefWindowProcW(window, message, wp, lp);
        }
        match message {
            WM_DRAWITEM if wp == NAV as usize => {
                let item = &*(lp as *const DRAWITEMSTRUCT);
                if item.itemID >= 4 {
                    return 1;
                }
                let selected = item.itemState & ODS_SELECTED != 0;
                let brush = CreateSolidBrush(if crate::platform::theme::is_dark() && selected {
                    0x68462b
                } else {
                    crate::platform::theme::panel(if selected { 0x00f4dfc5 } else { 0x00faf5ef })
                });
                FillRect(item.hDC, &item.rcItem, brush);
                DeleteObject(brush);
                if selected {
                    let accent = CreateSolidBrush(crate::platform::theme::panel(0x00ba7733));
                    FillRect(
                        item.hDC,
                        &RECT {
                            right: item.rcItem.left + dpi::scale(3, dpi::window_dpi(window)),
                            ..item.rcItem
                        },
                        accent,
                    );
                    DeleteObject(accent);
                }
                SetBkMode(item.hDC, TRANSPARENT as i32);
                SetTextColor(
                    item.hDC,
                    crate::platform::theme::text(if selected { 0x00804416 } else { 0x00604c3c }),
                );
                let caption =
                    wide(["Appearance", "Accounts", "Storage", "About"][item.itemID as usize]);
                let mut bounds = item.rcItem;
                bounds.left += dpi::scale(14, dpi::window_dpi(window));
                DrawTextW(
                    item.hDC,
                    caption.as_ptr(),
                    -1,
                    &mut bounds,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                );
                if item.itemState & ODS_FOCUS != 0 {
                    DrawFocusRect(item.hDC, &item.rcItem);
                }
                1
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(window, &mut paint);
                let mut bounds = RECT::default();
                GetClientRect(window, &mut bounds);
                let scale = |n| dpi::scale(n, dpi::window_dpi(window));
                SetDCBrushColor(
                    dc,
                    crate::platform::theme::surface(GetSysColor(COLOR_WINDOW)),
                );
                FillRect(dc, &bounds, GetStockObject(DC_BRUSH) as HBRUSH);
                let brush = CreateSolidBrush(crate::platform::theme::panel(0x00faf5ef));
                FillRect(
                    dc,
                    &RECT {
                        right: scale(182),
                        bottom: scale(320),
                        ..bounds
                    },
                    brush,
                );
                DeleteObject(brush);
                FillRect(
                    dc,
                    &RECT {
                        top: scale(320),
                        ..bounds
                    },
                    {
                        SetDCBrushColor(
                            dc,
                            crate::platform::theme::panel(GetSysColor(COLOR_BTNFACE)),
                        );
                        GetStockObject(DC_BRUSH) as HBRUSH
                    },
                );
                let line = CreateSolidBrush(crate::platform::theme::panel(0x00ddd6ce));
                FillRect(
                    dc,
                    &RECT {
                        left: scale(182),
                        right: scale(182) + 1,
                        bottom: scale(320),
                        ..bounds
                    },
                    line,
                );
                FillRect(
                    dc,
                    &RECT {
                        top: scale(320),
                        bottom: scale(320) + 1,
                        ..bounds
                    },
                    line,
                );
                FillRect(
                    dc,
                    &RECT {
                        left: scale(196),
                        top: scale(56),
                        right: scale(576),
                        bottom: scale(56) + 1,
                    },
                    line,
                );
                DeleteObject(line);
                EndPaint(window, &paint);
                0
            }
            WM_CTLCOLORLISTBOX => {
                SetBkColor(wp as HDC, 0x00faf5ef);
                SetDCBrushColor(wp as HDC, crate::platform::theme::panel(0x00faf5ef));
                GetStockObject(DC_BRUSH) as isize
            }
            WM_COMMAND => {
                let id = (wp & 0xffff) as u16;
                match id {
                    NAV if (wp >> 16) as u32 == LBN_SELCHANGE => {
                        let page = SendMessageW(lp as HWND, LB_GETCURSEL, 0, 0);
                        if (0..4).contains(&page) {
                            (*pointer).select(page as usize);
                        }
                    }
                    APPLY => {
                        let preferences = Preferences {
                            dark_mode: SendMessageW((*pointer).dark_mode, BM_GETCHECK, 0, 0)
                                == BST_CHECKED as isize,
                            folders: SendMessageW((*pointer).folders, BM_GETCHECK, 0, 0)
                                == BST_CHECKED as isize,
                            preview: SendMessageW((*pointer).preview, BM_GETCHECK, 0, 0)
                                == BST_CHECKED as isize,
                        };
                        if let Err(error) = preferences.save() {
                            MessageBoxW(
                                window,
                                wide(&error).as_ptr(),
                                wide("Could not save settings").as_ptr(),
                                MB_ICONERROR,
                            );
                        } else {
                            PostMessageW((*pointer).owner, CHANGED, 0, 0);
                            DestroyWindow(window);
                        }
                    }
                    ACCOUNTS => {
                        PostMessageW(
                            (*pointer).owner,
                            WM_COMMAND,
                            crate::ui::shared::commands::command_id(
                                crate::app::Command::ConfigureAccount,
                            ) as usize,
                            0,
                        );
                    }
                    ABOUT => crate::ui::about::show(window),
                    id if id == IDCANCEL as u16 => {
                        DestroyWindow(window);
                    }
                    _ => {}
                }
                0
            }
            WM_CTLCOLORSTATIC => {
                SetBkMode(wp as HDC, TRANSPARENT as i32);
                let heading = (*pointer)
                    .pages
                    .iter()
                    .any(|page| page.first().copied() == Some(lp as HWND));
                SetTextColor(
                    wp as HDC,
                    if heading {
                        0x00993300
                    } else {
                        GetSysColor(COLOR_WINDOWTEXT)
                    },
                );
                GetSysColorBrush(COLOR_WINDOW) as isize
            }
            WM_DPICHANGED => {
                (*pointer).layout.update(window, (wp & 0xffff) as u32);
                for page in &(*pointer).pages {
                    if let Some(heading) = page.first() {
                        (*pointer)
                            .layout
                            .metrics
                            .apply_font(*heading, dpi::FontWeight::Bold);
                    }
                }
                SendMessageW(
                    GetDlgItem(window, NAV as i32),
                    LB_SETITEMHEIGHT,
                    0,
                    dpi::scale(34, (wp & 0xffff) as u32) as isize,
                );
                dpi::suggested(window, lp);
                InvalidateRect(window, null(), 1);
                0
            }
            WM_CLOSE => {
                DestroyWindow(window);
                0
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                drop(Box::from_raw(pointer));
                DefWindowProcW(window, message, wp, lp)
            }
            _ => DefWindowProcW(window, message, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_preferences_keep_light_mode_and_dark_mode_round_trips() {
        let old: Preferences = serde_json::from_str(r#"{"folders":false,"preview":true}"#).unwrap();
        assert!(!old.dark_mode);
        let dark = Preferences {
            dark_mode: true,
            ..old
        };
        let restored: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&dark).unwrap()).unwrap();
        assert!(restored.dark_mode);
        assert!(!restored.folders);
    }

    #[test]
    fn pages_switch_without_saving_and_cancel_releases_window_state() {
        let _lock = crate::ui::WINDOW_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            let owner = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("Settings test").as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                640,
                480,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!owner.is_null());
            let before = std::fs::read(Preferences::path()).ok();
            open(owner);
            let mut window = FindWindowExW(
                null_mut(),
                null_mut(),
                wide("WinMailRustSettings").as_ptr(),
                null(),
            );
            while !window.is_null() && GetWindow(window, GW_OWNER) != owner {
                window = FindWindowExW(
                    null_mut(),
                    window,
                    wide("WinMailRustSettings").as_ptr(),
                    null(),
                );
            }
            assert!(!window.is_null());
            let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *const Settings;
            assert!(IsWindowVisible((*pointer).folders) != 0);
            let nav = GetDlgItem(window, NAV as i32);
            SendMessageW(nav, LB_SETCURSEL, 2, 0);
            SendMessageW(
                window,
                WM_COMMAND,
                NAV as usize | ((LBN_SELCHANGE as usize) << 16),
                nav as isize,
            );
            assert_eq!(IsWindowVisible((*pointer).folders), 0);
            assert!(IsWindowVisible((&(*pointer).pages[2])[0]) != 0);
            SendMessageW((*pointer).folders, BM_SETCHECK, BST_UNCHECKED as usize, 0);
            SendMessageW(window, WM_COMMAND, IDCANCEL as usize, 0);
            assert_eq!(IsWindow(window), 0);
            assert_eq!(std::fs::read(Preferences::path()).ok(), before);
            DestroyWindow(owner);
        }
    }
}
