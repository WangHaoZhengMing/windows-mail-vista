//! Owned OAuth settings window. Only OK updates the parent properties draft.
use super::*;
use mail_core::oauth::{LoopbackHost, OAuthAccount, OAuthConfig, OAuthProvider};
use windows_sys::Win32::UI::Shell::ShellExecuteW;

mod config;
mod controls;
#[cfg(test)]
mod tests;
mod worker;
use config::{ConfigInput, preset};
use worker::LoginWorker;

pub(super) const OAUTH_UPDATED: u32 = WM_APP + 32;
pub(super) const OAUTH_CLOSED: u32 = WM_APP + 33;
pub(super) const PROVIDER: u16 = 350;
pub(super) const CLIENT_ID: u16 = 351;
pub(super) const CLIENT_SECRET: u16 = 352;
pub(super) const AUTHORIZATION_ENDPOINT: u16 = 353;
pub(super) const TOKEN_ENDPOINT: u16 = 354;
pub(super) const SCOPES: u16 = 355;
pub(super) const REDIRECT_HOST: u16 = 356;
pub(super) const SIGN_IN: u16 = 357;
pub(super) const SIGN_OUT: u16 = 358;
const CLEAR_SECRET: u16 = 359;
const POLL_TIMER: usize = 1;
const WIDTH: i32 = 850;
const HEIGHT: i32 = 620;

struct OAuthHost {
    owner: HWND,
    updates: Sender<OAuthAccount>,
    state: RefCell<OAuthSettings>,
    window_owned: Cell<bool>,
}
struct OAuthSettings {
    layout: WindowLayout,
    account: OAuthAccount,
    login_hint: String,
    provider: HWND,
    edits: [HWND; 5], // Client ID, secret, authorize, token, scopes.
    redirect_host: HWND,
    clear_secret: HWND,
    hint: HWND,
    status: HWND,
    sign_in: HWND,
    sign_out: HWND,
    save: HWND,
    worker: Option<LoginWorker>,
    error: bool,
}
impl OAuthSettings {
    fn new(account: Option<OAuthAccount>, login_hint: String) -> Self {
        Self {
            layout: WindowLayout::default(),
            account: account.unwrap_or_else(|| OAuthAccount {
                config: OAuthConfig::microsoft(""),
                tokens: None,
            }),
            login_hint,
            provider: null_mut(),
            edits: [null_mut(); 5],
            redirect_host: null_mut(),
            clear_secret: null_mut(),
            hint: null_mut(),
            status: null_mut(),
            sign_in: null_mut(),
            sign_out: null_mut(),
            save: null_mut(),
            worker: None,
            error: false,
        }
    }
    unsafe fn start_login(&mut self, window: HWND) {
        unsafe {
            if self.worker.is_some() {
                return;
            }
            let account = match self.input().account(&self.account) {
                Ok(account) => account,
                Err(error) => {
                    self.status(true, &error);
                    SetFocus(self.edits[0]);
                    return;
                }
            };
            let worker = match LoginWorker::start(
                account.config.clone(),
                self.login_hint.clone(),
                open_browser,
            ) {
                Ok(worker) => worker,
                Err(error) => {
                    self.status(true, &error);
                    return;
                }
            };
            self.account = account;
            self.worker = Some(worker);
            // Secret text is never populated from storage, and can be discarded now.
            set_window_text(self.edits[1], "");
            SendMessageW(self.clear_secret, BM_SETCHECK, BST_UNCHECKED as usize, 0);
            if SetTimer(window, POLL_TIMER, 200, None) == 0 {
                self.worker = None;
                self.status(true, "Could not monitor sign-in. Please try again.");
                self.enable_fields();
                return;
            }
            self.status(false, "Waiting for browser sign-in. Cancel closes this window and stops the callback listener.");
            self.enable_fields();
        }
    }
    unsafe fn poll_login(&mut self, window: HWND) {
        unsafe {
            let result = self.worker.as_mut().and_then(LoginWorker::poll);
            let Some(result) = result else {
                return;
            };
            KillTimer(window, POLL_TIMER);
            self.worker = None;
            match result {
                Ok(account) => {
                    self.account = account;
                    self.fill();
                    self.status(
                        false,
                        "Signed in. Choose OK here, then OK in Account Properties to save.",
                    );
                }
                Err(error) => self.status(true, &error),
            }
            self.enable_fields();
        }
    }
}
fn open_browser(url: &str) -> Result<()> {
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            wide("open").as_ptr(),
            wide(url).as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err("Could not open your default browser for sign-in.".into())
    } else {
        Ok(())
    }
}

pub(super) unsafe fn open(
    owner: HWND,
    previous: Option<OAuthAccount>,
    login_hint: String,
    updates: Sender<OAuthAccount>,
) -> Result<HWND> {
    unsafe {
        let class = wide("WinMailRustOAuthSettings");
        register_class(&class, Some(window_proc))?;
        let host = Box::new(OAuthHost {
            owner,
            updates,
            state: RefCell::new(OAuthSettings::new(previous, login_hint)),
            window_owned: Cell::new(false),
        });
        let window = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide("OAuth Sign-in Settings").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WIDTH,
            HEIGHT,
            owner,
            null_mut(),
            GetModuleHandleW(null()),
            (&*host as *const OAuthHost).cast(),
        );
        if window.is_null() {
            return Err("Could not create OAuth settings.".into());
        }
        host.window_owned.set(true);
        let edit = host.state.borrow().edits[0];
        let _ = Box::into_raw(host);
        let value = dpi::window_dpi(window);
        SetWindowPos(
            window,
            null_mut(),
            0,
            0,
            dpi::scale(WIDTH, value),
            dpi::scale(HEIGHT, value),
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        EnableWindow(owner, 0);
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
        SetFocus(edit);
        Ok(window)
    }
}
unsafe fn save(window: HWND, host: &OAuthHost) {
    unsafe {
        let account = {
            let Ok(mut state) = host.state.try_borrow_mut() else {
                return;
            };
            if state.worker.is_some() {
                return;
            }
            match state.input().account(&state.account) {
                Ok(account) => account,
                Err(error) => {
                    state.status(true, &error);
                    SetFocus(state.edits[0]);
                    return;
                }
            }
        };
        if host.updates.send(account).is_err() {
            if let Ok(mut state) = host.state.try_borrow_mut() {
                state.status(
                    true,
                    "Account Properties is no longer accepting OAuth settings.",
                );
            }
            return;
        }
        // Native callbacks run on this thread; workers only send owned channel data.
        SendMessageW(host.owner, OAUTH_UPDATED, 0, 0);
        if IsWindow(window) != 0 {
            DestroyWindow(window);
        }
    }
}
unsafe extern "system" fn window_proc(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if message == WM_NCCREATE {
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
            );
            return 1;
        }
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *const OAuthHost;
        if pointer.is_null() {
            return DefWindowProcW(window, message, wp, lp);
        }
        let host = &*pointer;
        match message {
            WM_CREATE => {
                if let Ok(mut state) = host.state.try_borrow_mut() {
                    state.build(window);
                }
                0
            }
            WM_COMMAND => {
                let id = (wp & 0xffff) as u16;
                if id == IDOK as u16 {
                    save(window, host);
                } else if id == IDCANCEL as u16 {
                    DestroyWindow(window);
                } else if let Ok(mut state) = host.state.try_borrow_mut() {
                    match id {
                        PROVIDER if (wp >> 16) as u32 == CBN_SELCHANGE => state.change_provider(),
                        SIGN_IN => state.start_login(window),
                        SIGN_OUT if state.worker.is_none() => {
                            state.account.sign_out();
                            state.status(false, "Signed out locally. Choose OK to keep this change, or Cancel to discard it.");
                            state.enable_fields();
                        }
                        _ => {}
                    }
                }
                0
            }
            WM_TIMER if wp == POLL_TIMER => {
                if let Ok(mut state) = host.state.try_borrow_mut() {
                    state.poll_login(window);
                }
                0
            }
            WM_DPICHANGED => {
                if let Ok(mut state) = host.state.try_borrow_mut() {
                    state.layout.update(window, (wp & 0xffff) as u32);
                }
                dpi::suggested(window, lp);
                0
            }
            WM_CTLCOLORSTATIC => {
                if let Ok(state) = host.state.try_borrow()
                    && state.error
                    && lp as HWND == state.status
                {
                    return error_brush(wp);
                }
                DefWindowProcW(window, message, wp, lp)
            }
            WM_CLOSE => {
                DestroyWindow(window);
                0
            }
            WM_DESTROY => {
                KillTimer(window, POLL_TIMER);
                if let Ok(mut state) = host.state.try_borrow_mut() {
                    state.worker = None;
                }
                0
            }
            WM_NCDESTROY => {
                let owner = host.owner;
                let owned = host.window_owned.get();
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                if owned {
                    drop(Box::from_raw(pointer.cast_mut()));
                    if IsWindow(owner) != 0 {
                        SendMessageW(owner, OAUTH_CLOSED, window as usize, 0);
                        EnableWindow(owner, 1);
                        SetForegroundWindow(owner);
                    }
                }
                DefWindowProcW(window, message, wp, lp)
            }
            _ => DefWindowProcW(window, message, wp, lp),
        }
    }
}
