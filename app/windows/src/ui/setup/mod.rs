//! First launch setup. Cancel keeps configuration absent so setup can resume.
use super::settings::{CHANGED, Preferences};
use crate::platform::{
    controls::{control, set_window_text},
    dpi,
    strings::wide,
    theme,
};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::*,
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{EnableWindow, SetFocus},
        WindowsAndMessaging::*,
    },
};
const NEXT: u16 = 810;
const BACK: u16 = 811;
const ACCOUNT: u16 = 812;
const DARK: u16 = 813;
const FOLDERS: u16 = 814;
const PREVIEW: u16 = 815;
pub(super) const EMBEDDED: u32 = WM_APP + 70;
pub(super) const ACCOUNT_SAVED: u32 = WM_APP + 71;
const TITLES: [&str; 4] = [
    "Welcome to Windows Mail Vista",
    "Make yourself at home",
    "Connect your email account",
    "You are ready to go",
];
struct Setup {
    owner: HWND,
    metrics: dpi::Resources,
    page: usize,
    controls: Vec<(HWND, [i32; 4])>,
    pages: [Vec<HWND>; 4],
    heading: HWND,
    steps: HWND,
    manager: HWND,
    account_saved: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;

    #[test]
    fn setup_navigation_keeps_choices_and_cancel_preserves_configuration() {
        let _lock = crate::ui::WINDOW_TEST_LOCK.lock().unwrap();
        let _context = dpi::ThreadContext::enter();
        let path = crate::store::default_root().join("desktop-settings.json");
        let before = std::fs::read(&path).ok();
        unsafe {
            let owner = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("Setup test").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                900,
                650,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!owner.is_null());
            open(owner, false);
            let window = FindWindowExW(
                null_mut(),
                null_mut(),
                wide("WinMailRustFirstRun").as_ptr(),
                null(),
            );
            assert!(!window.is_null());
            assert_eq!(IsWindowEnabled(owner), 0);
            let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Setup;
            assert_eq!((*state).page, 0);
            assert_eq!(IsWindowEnabled(GetDlgItem(window, BACK as i32)), 0);
            SendMessageW(window, WM_COMMAND, NEXT as usize, 0);
            assert_eq!((*state).page, 1);
            let checkbox = GetDlgItem(window, DARK as i32);
            assert_ne!(IsWindowVisible(checkbox), 0);
            SendMessageW(checkbox, BM_SETCHECK, BST_CHECKED as usize, 0);
            SendMessageW(window, WM_COMMAND, NEXT as usize, 0);
            assert_eq!((*state).page, 2);
            assert_eq!(IsWindowVisible(checkbox), 0);
            SendMessageW(window, WM_COMMAND, BACK as usize, 0);
            assert_eq!(
                SendMessageW(checkbox, BM_GETCHECK, 0, 0),
                BST_CHECKED as isize
            );
            let bounds = RECT {
                left: 0,
                top: 0,
                right: 1250,
                bottom: 950,
            };
            SendMessageW(
                window,
                WM_DPICHANGED,
                192 | (192 << 16),
                &bounds as *const RECT as isize,
            );
            let mut button = RECT::default();
            GetWindowRect(GetDlgItem(window, NEXT as i32), &mut button);
            assert_eq!(button.right - button.left, 192);
            SendMessageW(window, WM_COMMAND, NEXT as usize, 0);
            SendMessageW(window, WM_COMMAND, NEXT as usize, 0);
            assert_eq!((*state).page, 3);
            SendMessageW(window, WM_COMMAND, IDCANCEL as usize, 0);
            assert_eq!(IsWindow(window), 0);
            assert_ne!(IsWindowEnabled(owner), 0);
            DestroyWindow(owner);
        }
        assert_eq!(std::fs::read(path).ok(), before);
    }
}
impl Setup {
    unsafe fn item(
        &mut self,
        window: HWND,
        page: Option<usize>,
        class: &str,
        text: &str,
        style: u32,
        id: u16,
        rect: [i32; 4],
    ) -> HWND {
        unsafe {
            let child = control(window, class, text, style, id);
            self.controls.push((child, rect));
            if let Some(page) = page {
                self.pages[page].push(child);
            }
            child
        }
    }
    unsafe fn build(&mut self, window: HWND, has_accounts: bool) {
        unsafe {
            self.heading = self.item(window, None, "STATIC", "", 0, 0, [24, 22, 552, 28]);
            self.steps = self.item(window, None, "STATIC", "", 0, 0, [24, 57, 552, 20]);
            self.item(window,Some(0),"STATIC","Your mail, with a familiar Windows interface.\n\nThis assistant helps you choose your layout and add an email account. You can review server settings and test the connection before saving.\n\nPrefer to start offline? You can skip account setup and use Local Folders.",0,0,[28,112,536,160]);
            self.item(window,Some(0),"STATIC","Passwords are protected using your Windows account.\nWebView is opened only when you choose it while reading mail.",0,0,[28,288,536,52]);
            self.item(window,Some(1),"STATIC","Choose how your mail window looks. You can change these options later in Tools → Settings.",0,0,[28,112,536,44]);
            for (id, label, y, checked) in [
                (FOLDERS, "Show folder sidebar", 172, true),
                (PREVIEW, "Show message preview", 214, true),
                (DARK, "Use dark mode", 256, false),
            ] {
                let child = self.item(
                    window,
                    Some(1),
                    "BUTTON",
                    label,
                    WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                    id,
                    [28, y, 536, 28],
                );
                SendMessageW(child, BM_SETCHECK, usize::from(checked), 0);
            }
            self.item(
                window,
                Some(1),
                "STATIC",
                "Your choices apply when you select Finish.",
                0,
                0,
                [28, 304, 536, 30],
            );
            self.item(window,Some(2),"STATIC",if has_accounts { "Your existing accounts will be kept. You can add another account now, or continue with your current setup." } else { "Set up iCloud, Gmail, Outlook.com, Yahoo, QQ Mail, or another provider. Server and encryption settings are filled in for common services." },0,0,[28,112,536,64]);
            self.item(
                window,
                Some(2),
                "BUTTON",
                "Set up an email account…",
                WS_TABSTOP,
                ACCOUNT,
                [28, 190, 290, 36],
            );
            self.item(window,Some(2),"STATIC","Choose your service, enter your email address, then review and test your server settings here.\n\nYour account is saved when you choose Save & continue. Appearance settings are saved when you finish this assistant.",0,0,[28,242,536,104]);
            self.item(window,Some(3),"STATIC","Select Finish to save your appearance settings and open your inbox.\n\nConfigured accounts synchronize in the background. You can see progress and stop synchronization from the main window.\n\nIf you skipped account setup, choose Tools → Accounts to add an account later.",0,0,[28,112,536,150]);
            self.item(
                window,
                Some(3),
                "STATIC",
                "Welcome to Windows Mail Vista.",
                0,
                0,
                [28, 292, 536, 28],
            );
            self.item(
                window,
                None,
                "BUTTON",
                "‹ Back",
                WS_TABSTOP,
                BACK,
                [270, 376, 90, 30],
            );
            self.item(
                window,
                None,
                "BUTTON",
                "Next ›",
                WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
                NEXT,
                [372, 376, 96, 30],
            );
            self.item(
                window,
                None,
                "BUTTON",
                "Cancel",
                WS_TABSTOP,
                IDCANCEL as u16,
                [480, 376, 96, 30],
            );
            self.resize(window, dpi::window_dpi(window));
            self.select(window, 0);
        }
    }
    unsafe fn resize(&mut self, window: HWND, value: u32) {
        unsafe {
            self.metrics.update(window, value);
            for (child, [x, y, w, h]) in &self.controls {
                dpi::move_control(value, *child, *x, *y, *w, *h, 1);
                self.metrics.apply_font(*child, dpi::FontWeight::Regular);
            }
            self.metrics.apply_font(self.heading, dpi::FontWeight::Bold);
        }
    }
    unsafe fn select(&mut self, window: HWND, page: usize) {
        unsafe {
            self.page = page;
            for (index, children) in self.pages.iter().enumerate() {
                for child in children {
                    ShowWindow(*child, if index == page { SW_SHOW } else { SW_HIDE });
                }
            }
            set_window_text(self.heading, TITLES[page]);
            set_window_text(
                self.steps,
                &format!(
                    "Step {} of 4 · Welcome → Appearance → Account → Finish",
                    page + 1
                ),
            );
            set_window_text(
                GetDlgItem(window, NEXT as i32),
                if page == 3 {
                    "Finish"
                } else if page == 2 {
                    "Continue ›"
                } else {
                    "Next ›"
                },
            );
            EnableWindow(GetDlgItem(window, BACK as i32), i32::from(page > 0));
            SetFocus(if page == 1 {
                GetDlgItem(window, FOLDERS as i32)
            } else {
                GetDlgItem(window, NEXT as i32)
            });
            InvalidateRect(window, null(), 1);
        }
    }
}
pub(super) unsafe fn open(owner: HWND, has_accounts: bool) {
    unsafe {
        let class = wide("WinMailRustFirstRun");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: GetModuleHandleW(null()),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return;
        }
        let value = dpi::window_dpi(owner);
        let mut bounds = RECT {
            right: dpi::scale(604, value),
            bottom: dpi::scale(426, value),
            ..Default::default()
        };
        let style = WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
        AdjustWindowRectEx(
            &mut bounds,
            style,
            0,
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
        );
        let mut parent = RECT::default();
        GetWindowRect(owner, &mut parent);
        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        let window = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide("Welcome — Windows Mail Vista").as_ptr(),
            style,
            parent.left + (parent.right - parent.left - width) / 2,
            parent.top + (parent.bottom - parent.top - height) / 2,
            width,
            height,
            owner,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if window.is_null() {
            return;
        }
        let mut state = Box::new(Setup {
            owner,
            metrics: Default::default(),
            page: 0,
            controls: vec![],
            pages: std::array::from_fn(|_| vec![]),
            heading: null_mut(),
            steps: null_mut(),
            manager: null_mut(),
            account_saved: false,
        });
        SetWindowLongPtrW(window, GWLP_USERDATA, (&mut *state as *mut Setup) as isize);
        state.build(window, has_accounts);
        let _ = Box::into_raw(state);
        EnableWindow(owner, 0);
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
    }
}
unsafe extern "system" fn procedure(window: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Setup;
        if pointer.is_null() {
            return DefWindowProcW(window, msg, wp, lp);
        }

        match msg {
            WM_COMMAND => {
                match (wp & 0xffff) as u16 {
                    NEXT if (*pointer).page < 3 => (*pointer).select(window, (*pointer).page + 1),
                    NEXT => {
                        let checked = |id| {
                            SendMessageW(GetDlgItem(window, id), BM_GETCHECK, 0, 0)
                                == BST_CHECKED as isize
                        };
                        let prefs = Preferences {
                            folders: checked(FOLDERS as i32),
                            preview: checked(PREVIEW as i32),
                            dark_mode: checked(DARK as i32),
                        };
                        if let Err(error) = prefs.save() {
                            MessageBoxW(
                                window,
                                wide(&error).as_ptr(),
                                wide("Could not save setup").as_ptr(),
                                MB_ICONERROR,
                            );
                        } else {
                            PostMessageW((*pointer).owner, CHANGED, 0, 0);
                            DestroyWindow(window);
                        }
                    }
                    BACK if (*pointer).page > 0 => (*pointer).select(window, (*pointer).page - 1),
                    ACCOUNT => {
                        (*pointer).account_saved = false;
                        SetPropW((*pointer).owner, wide("WinMailSetupHost").as_ptr(), window);
                        SendMessageW(
                            (*pointer).owner,
                            WM_COMMAND,
                            crate::ui::shared::commands::command_id(
                                crate::app::Command::ConfigureAccount,
                            ) as usize,
                            0,
                        );
                        RemovePropW((*pointer).owner, wide("WinMailSetupHost").as_ptr());
                        let mut manager = null_mut();
                        loop {
                            manager = FindWindowExW(
                                null_mut(),
                                manager,
                                wide("WinMailRustAccountManager").as_ptr(),
                                null(),
                            );
                            if manager.is_null() || GetWindow(manager, GW_OWNER) == (*pointer).owner
                            {
                                break;
                            }
                        }
                        if !manager.is_null() && GetWindow(manager, GW_OWNER) == (*pointer).owner {
                            begin_account(window, manager);
                        } else {
                            ShowWindow(window, SW_SHOW);
                        }
                    }
                    id if id == IDCANCEL as u16 => {
                        DestroyWindow(window);
                    }
                    _ => {}
                };
                0
            }
            WM_TIMER => {
                if !(*pointer).manager.is_null()
                    && IsWindow(super::accounts::setup_editor((*pointer).manager)) == 0
                {
                    KillTimer(window, 1);
                    if IsWindow((*pointer).manager) != 0 {
                        DestroyWindow((*pointer).manager);
                    }
                    (*pointer).manager = null_mut();
                    EnableWindow((*pointer).owner, 0);
                    size_client(window, 604, 426, (*pointer).metrics.dpi);
                    ShowWindow((*pointer).heading, SW_SHOW);
                    ShowWindow((*pointer).steps, SW_SHOW);
                    for id in [BACK, NEXT, IDCANCEL as u16] {
                        ShowWindow(GetDlgItem(window, id as i32), SW_SHOW);
                    }
                    (*pointer).select(window, if (*pointer).account_saved { 3 } else { 2 });
                    set_window_text(window, "Welcome — Windows Mail Vista");
                    SetForegroundWindow(window);
                }
                0
            }
            EMBEDDED => {
                for (control, _) in &(*pointer).controls {
                    ShowWindow(*control, SW_HIDE);
                }
                let width = (lp & 0xffff) as i32;
                let height = ((lp >> 16) & 0xffff) as i32;
                size_client(window, width, height, (*pointer).metrics.dpi);
                MoveWindow(
                    wp as HWND,
                    0,
                    0,
                    dpi::scale(width, (*pointer).metrics.dpi),
                    dpi::scale(height, (*pointer).metrics.dpi),
                    1,
                );
                set_window_text(window, "Account setup — Windows Mail Vista");
                InvalidateRect(window, null(), 1);
                0
            }
            ACCOUNT_SAVED => {
                (*pointer).account_saved = true;
                0
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(window, &mut ps);
                let mut r = RECT::default();
                GetClientRect(window, &mut r);
                let brush = GetStockObject(DC_BRUSH) as HBRUSH;
                SetDCBrushColor(dc, theme::surface(0xffffff));
                FillRect(dc, &r, brush);
                let header = dpi::scale(90, (*pointer).metrics.dpi);
                for y in 0..header {
                    let t = y as f32 / header as f32;
                    let color = if theme::is_dark() {
                        0x303030
                    } else {
                        let red = (224.0 + 24.0 * t) as u32;
                        let green = (237.0 + 14.0 * t) as u32;
                        red | (green << 8) | (255 << 16)
                    };
                    SetDCBrushColor(dc, color);
                    FillRect(
                        dc,
                        &RECT {
                            top: y,
                            bottom: y + 1,
                            ..r
                        },
                        brush,
                    );
                }
                SetDCBrushColor(dc, theme::panel(0xf0f0f0));
                FillRect(
                    dc,
                    &RECT {
                        top: dpi::scale(360, (*pointer).metrics.dpi),
                        ..r
                    },
                    brush,
                );
                SetDCBrushColor(dc, theme::edge(0xddd6ce));
                for y in [header, dpi::scale(360, (*pointer).metrics.dpi)] {
                    FillRect(
                        dc,
                        &RECT {
                            top: y,
                            bottom: y + 1,
                            ..r
                        },
                        brush,
                    );
                }
                EndPaint(window, &ps);
                0
            }
            WM_CTLCOLORSTATIC => {
                let dc = wp as HDC;
                SetBkMode(dc, TRANSPARENT as i32);
                SetTextColor(
                    dc,
                    theme::text(if lp as HWND == (*pointer).heading {
                        0x8e4e1c
                    } else {
                        0x40362d
                    }),
                );
                if lp as HWND == (*pointer).heading || lp as HWND == (*pointer).steps {
                    SetDCBrushColor(dc, theme::panel(0x00fff1e5));
                    GetStockObject(DC_BRUSH) as isize
                } else {
                    SetDCBrushColor(dc, theme::surface(0xffffff));
                    GetStockObject(DC_BRUSH) as isize
                }
            }
            WM_DPICHANGED => {
                (*pointer).resize(window, (wp & 0xffff) as u32);
                dpi::suggested(window, lp);
                if IsWindow((*pointer).manager) != 0 {
                    let editor = super::accounts::setup_editor((*pointer).manager);
                    if IsWindow(editor) != 0 {
                        let mut bounds = RECT::default();
                        GetClientRect(window, &mut bounds);
                        SendMessageW(editor, WM_DPICHANGED, wp, &bounds as *const RECT as isize);
                    }
                }
                InvalidateRect(window, null(), 1);
                0
            }
            WM_CLOSE => {
                DestroyWindow(window);
                0
            }
            WM_NCDESTROY => {
                KillTimer(window, 1);
                let owner = (*pointer).owner;
                let manager = (*pointer).manager;
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                if IsWindow(manager) != 0 {
                    DestroyWindow(manager);
                }
                drop(Box::from_raw(pointer));
                if IsWindow(owner) != 0 {
                    EnableWindow(owner, 1);
                    SetForegroundWindow(owner);
                }
                DefWindowProcW(window, msg, wp, lp)
            }
            _ => DefWindowProcW(window, msg, wp, lp),
        }
    }
}

pub(super) unsafe fn begin_account(window: HWND, manager: HWND) {
    unsafe {
        let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Setup;
        (*state).manager = manager;
        (*state).account_saved = false;
        SendMessageW(
            manager,
            WM_COMMAND,
            super::accounts::ADD_ACCOUNT as usize,
            0,
        );
        SetTimer(window, 1, 100, None);
    }
}

unsafe fn size_client(window: HWND, width: i32, height: i32, value: u32) {
    unsafe {
        let mut rect = RECT {
            right: dpi::scale(width, value),
            bottom: dpi::scale(height, value),
            ..Default::default()
        };
        AdjustWindowRectEx(
            &mut rect,
            GetWindowLongW(window, GWL_STYLE) as u32,
            0,
            GetWindowLongW(window, GWL_EXSTYLE) as u32,
        );
        SetWindowPos(
            window,
            null_mut(),
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}
