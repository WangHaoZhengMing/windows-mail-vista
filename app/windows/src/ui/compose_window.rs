//! Compose window controls and native events.
mod rich;
use super::{DISPATCH, command_ids::command_id, menus};
use crate::platform::icons;
use crate::{
    Result,
    account::{AccountId, AccountRecord},
    app::Command,
    message::Draft,
    platform::{
        classic,
        controls::{control, window_text as text},
        dpi,
        strings::wide,
    },
};
use std::{
    cell::{Cell, RefCell},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::COLOR_BTNFACE,
    System::LibraryLoader::*,
    UI::{Controls::*, WindowsAndMessaging::*},
};

struct ComposeState {
    metrics: dpi::Resources,
    owner: HWND,
    hwnd: HWND,
    bar: HWND,
    sender: HWND,
    sender_label: HWND,
    accounts: RefCell<Vec<AccountRecord>>,
    sender_identity: RefCell<Option<AccountId>>,
    to: HWND,
    cc: HWND,
    bcc: HWND,
    subject: HWND,
    body: HWND,
    format: Vec<HWND>,
    rich: Cell<bool>,
    colors: RefCell<[u32; 16]>,
    labels: Vec<HWND>,
    paths: RefCell<Vec<std::path::PathBuf>>,
    dirty: Cell<bool>,
    show_bcc: Cell<bool>,
    loading: Cell<bool>,
    icons: HIMAGELIST,
}
pub struct Window {
    state: Box<ComposeState>,
}
impl Window {
    #[cfg(test)]
    pub fn open(owner: HWND, draft: Draft) -> Result<Self> {
        Self::open_for(owner, draft, Vec::new(), None)
    }
    pub fn open_for(
        owner: HWND,
        draft: Draft,
        accounts: Vec<AccountRecord>,
        sender: Option<AccountId>,
    ) -> Result<Self> {
        unsafe {
            let mut state = Box::new(ComposeState {
                metrics: dpi::Resources::default(),
                owner,
                hwnd: null_mut(),
                bar: null_mut(),
                sender: null_mut(),
                sender_label: null_mut(),
                accounts: RefCell::new(Vec::new()),
                sender_identity: RefCell::new(None),
                to: null_mut(),
                cc: null_mut(),
                bcc: null_mut(),
                subject: null_mut(),
                body: null_mut(),
                format: Vec::new(),
                rich: Cell::new(true),
                colors: RefCell::new([0; 16]),
                labels: Vec::new(),
                paths: RefCell::new(Vec::new()),
                dirty: Cell::new(false),
                show_bcc: Cell::new(false),
                loading: Cell::new(true),
                icons: 0,
            });
            let class = wide("WinMailRust.Compose");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: GetModuleHandleW(null()),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: (COLOR_BTNFACE + 1) as _,
                hIcon: null_mut(),
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            RegisterClassW(&wc);
            state.hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("New Message").as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                800,
                660,
                owner,
                menus::menu(true),
                wc.hInstance,
                (&mut *state as *mut ComposeState).cast(),
            );
            if state.hwnd.is_null() {
                return Err("Cannot create message window".into());
            }
            let initial_dpi = dpi::window_dpi(state.hwnd);
            SetWindowPos(
                state.hwnd,
                null_mut(),
                0,
                0,
                dpi::scale(800, initial_dpi),
                dpi::scale(660, initial_dpi),
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let window = Self { state };
            window.set_accounts(accounts, sender.as_ref());
            window.set(draft);
            ShowWindow(window.state.hwnd, SW_SHOW);
            SetForegroundWindow(window.state.hwnd);
            Ok(window)
        }
    }
    pub fn hwnd(&self) -> HWND {
        self.state.hwnd
    }
    pub fn sender_id(&self) -> Option<AccountId> {
        let index = unsafe { SendMessageW(self.state.sender, CB_GETCURSEL, 0, 0) };
        usize::try_from(index).ok().and_then(|index| {
            self.state
                .accounts
                .borrow()
                .get(index)
                .map(|account| account.id.clone())
        })
    }
    pub fn set_accounts(&self, accounts: Vec<AccountRecord>, preferred: Option<&AccountId>) {
        let selected = self
            .sender_id()
            .or_else(|| self.state.sender_identity.borrow().clone());
        // Editing another account cannot silently change this window's sender.
        let wanted = selected.as_ref().or(preferred);
        let index = wanted.and_then(|id| accounts.iter().position(|account| &account.id == id));
        *self.state.sender_identity.borrow_mut() = wanted.cloned();
        self.state.loading.set(true);
        unsafe {
            SendMessageW(self.state.sender, CB_RESETCONTENT, 0, 0);
            for account in &accounts {
                let label = if account.name == account.account.email {
                    account.name.clone()
                } else {
                    format!("{} <{}>", account.name, account.account.email)
                };
                SendMessageW(
                    self.state.sender,
                    CB_ADDSTRING,
                    0,
                    wide(&label).as_ptr() as isize,
                );
            }
            SendMessageW(
                self.state.sender,
                CB_SETCUEBANNER,
                0,
                wide("Choose a sending account").as_ptr() as isize,
            );
            SendMessageW(
                self.state.sender,
                CB_SETCURSEL,
                index.unwrap_or(usize::MAX),
                0,
            );
        }
        *self.state.accounts.borrow_mut() = accounts;
        self.state.loading.set(false);
    }
    pub fn active(&self) -> bool {
        unsafe { IsWindow(self.state.hwnd) != 0 }
    }
    pub fn dirty(&self) -> bool {
        self.state.dirty.get()
    }
    pub fn saved(&self) {
        self.state.dirty.set(false);
    }
    pub fn set(&self, draft: Draft) {
        unsafe {
            self.state.loading.set(true);
            self.state.show_bcc.set(!draft.bcc.is_empty());
            for (control, value) in [
                (self.state.to, &draft.to),
                (self.state.cc, &draft.cc),
                (self.state.bcc, &draft.bcc),
                (self.state.subject, &draft.subject),
                (self.state.body, &draft.body),
            ] {
                SetWindowTextW(
                    control,
                    wide(&value.replace("\r\n", "\n").replace('\n', "\r\n")).as_ptr(),
                );
            }
            if let Some(rtf) = &draft.rtf {
                rich::restore(self.state.body, rtf);
            }
            self.state
                .rich
                .set(draft.html.is_some() || draft.rtf.is_some() || draft.body.is_empty());
            rich::sync(&self.state);
            *self.state.paths.borrow_mut() = draft.attachments;
            self.state.loading.set(false);
            self.state.dirty.set(false);
            self.state.layout();
            SetForegroundWindow(self.state.hwnd);
        }
    }
    pub fn draft(&self) -> Draft {
        unsafe {
            Draft {
                to: text(self.state.to),
                cc: text(self.state.cc),
                bcc: text(self.state.bcc),
                subject: text(self.state.subject),
                body: text(self.state.body),
                html: self.state.rich.get().then(|| rich::html(self.state.body)),
                rtf: self.state.rich.get().then(|| rich::rtf(self.state.body)),
                attachments: self.state.paths.borrow().clone(),
            }
        }
    }
    pub fn attach(&self, path: std::path::PathBuf) {
        self.state.paths.borrow_mut().push(path);
        self.state.dirty.set(true);
    }
    pub fn count(&self) -> usize {
        self.state.paths.borrow().len()
    }
    pub fn close(&self) {
        unsafe {
            if self.active() {
                DestroyWindow(self.state.hwnd);
            }
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        self.close();
        unsafe {
            ImageList_Destroy(self.state.icons);
        }
    }
}
impl ComposeState {
    unsafe fn update_dpi(&mut self, value: u32) {
        unsafe {
            let saved = rich::rtf(self.body);
            let loading = self.loading.replace(true);
            self.metrics.update(self.hwnd, value);
            if !saved.is_empty() {
                rich::restore(self.body, &saved);
            }
            self.loading.set(loading);
            let icons = classic::resize_toolbar(self.bar, self.metrics.dpi);
            ImageList_Destroy(std::mem::replace(&mut self.icons, icons));
        }
    }
    unsafe fn setup(&mut self, hwnd: HWND) {
        unsafe {
            self.hwnd = hwnd;
            (self.bar, self.icons) = classic::toolbar(
                hwnd,
                &[
                    (
                        "Send",
                        command_id(Command::Send),
                        icons::ToolbarIcon::Send,
                        true,
                    ),
                    ("Cut", menus::CUT as u16, icons::ToolbarIcon::Cut, true),
                    ("Copy", menus::COPY as u16, icons::ToolbarIcon::Copy, true),
                    (
                        "Paste",
                        menus::PASTE as u16,
                        icons::ToolbarIcon::Paste,
                        true,
                    ),
                    (
                        "Attach",
                        command_id(Command::Attach),
                        icons::ToolbarIcon::Attach,
                        true,
                    ),
                    (
                        "Save",
                        command_id(Command::SaveDraft),
                        icons::ToolbarIcon::Save,
                        true,
                    ),
                ],
            );
            for label in ["To:", "Cc:", "Bcc:", "Subject:"] {
                self.labels.push(control(hwnd, "STATIC", label, 0, 0));
            }
            self.sender_label = control(hwnd, "STATIC", "From:", 0, 0);
            self.sender = control(
                hwnd,
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP,
                206,
            );
            self.to = control(hwnd, "EDIT", "", ES_AUTOHSCROLL as u32 | WS_TABSTOP, 201);
            self.cc = control(hwnd, "EDIT", "", ES_AUTOHSCROLL as u32 | WS_TABSTOP, 202);
            self.bcc = control(hwnd, "EDIT", "", ES_AUTOHSCROLL as u32 | WS_TABSTOP, 205);
            self.subject = control(hwnd, "EDIT", "", ES_AUTOHSCROLL as u32 | WS_TABSTOP, 203);
            static RICHEDIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
            RICHEDIT.get_or_init(|| LoadLibraryW(wide("Msftedit.dll").as_ptr()) as usize);
            self.body = control(
                hwnd,
                "RICHEDIT50W",
                "",
                ES_MULTILINE as u32
                    | ES_AUTOVSCROLL as u32
                    | ES_NOHIDESEL as u32
                    | WS_VSCROLL
                    | WS_TABSTOP,
                204,
            );
            SendMessageW(self.body, EM_SETLIMITTEXT, 8 * 1024 * 1024, 0);
            for (index, value) in ["Arial", "12", "B", "I", "U", "A", "≡", "•"]
                .iter()
                .enumerate()
            {
                let value = *value;
                let h = control(
                    hwnd,
                    if value == "Arial" || value == "12" {
                        "COMBOBOX"
                    } else {
                        "BUTTON"
                    },
                    value,
                    if value == "Arial" || value == "12" {
                        CBS_DROPDOWNLIST as u32
                    } else {
                        if matches!(index, 2..=4) {
                            BS_AUTOCHECKBOX as u32 | BS_PUSHLIKE as u32
                        } else {
                            BS_PUSHBUTTON as u32
                        }
                    } | WS_TABSTOP,
                    rich::FIRST + index as u16,
                );
                if index < 2 {
                    let values = if index == 0 {
                        vec![
                            "Arial",
                            "Calibri",
                            "Segoe UI",
                            "Times New Roman",
                            "Consolas",
                            "Microsoft YaHei",
                        ]
                    } else {
                        vec![
                            "8", "9", "10", "11", "12", "14", "16", "18", "24", "32", "48", "72",
                        ]
                    };
                    for item in values {
                        SendMessageW(h, CB_ADDSTRING, 0, wide(item).as_ptr() as isize);
                    }
                    let selected = SendMessageW(
                        h,
                        CB_FINDSTRINGEXACT,
                        usize::MAX,
                        wide(value).as_ptr() as isize,
                    );
                    SendMessageW(h, CB_SETCURSEL, selected as usize, 0);
                }
                self.format.push(h);
            }
            self.update_dpi(dpi::window_dpi(hwnd));
            rich::initialize(self);
        }
    }
    fn layout(&self) {
        unsafe {
            let mut rect = RECT::default();
            GetClientRect(self.hwnd, &mut rect);
            let width = dpi::logical(rect.right, self.metrics.dpi);
            let height = dpi::logical(rect.bottom, self.metrics.dpi);
            dpi::move_control(self.metrics.dpi, self.bar, 0, 0, width, 26, 1);
            let show = self.show_bcc.get();
            let edits = [self.to, self.cc, self.bcc, self.subject];
            dpi::move_control(self.metrics.dpi, self.sender_label, 8, 34, 48, 22, 1);
            dpi::move_control(
                self.metrics.dpi,
                self.sender,
                56,
                30,
                (width - 63).max(80),
                180,
                1,
            );
            let mut row = 1;
            for (i, edit) in edits.iter().enumerate() {
                let visible = i != 2 || show;
                ShowWindow(*edit, if visible { SW_SHOW } else { SW_HIDE });
                ShowWindow(self.labels[i], if visible { SW_SHOW } else { SW_HIDE });
                if visible {
                    dpi::move_control(
                        self.metrics.dpi,
                        self.labels[i],
                        8,
                        34 + row * 26,
                        48,
                        22,
                        1,
                    );
                    dpi::move_control(
                        self.metrics.dpi,
                        *edit,
                        56,
                        30 + row * 26,
                        (width - 63).max(80),
                        24,
                        1,
                    );
                    row += 1;
                }
            }
            let top = 34 + row * 26;
            let mut x = 4;
            for (i, control) in self.format.iter().enumerate() {
                let w = if i == 0 {
                    155
                } else if i == 1 {
                    50
                } else {
                    25
                };
                dpi::move_control(
                    self.metrics.dpi,
                    *control,
                    x,
                    top,
                    w,
                    if i < 2 { 220 } else { 22 },
                    1,
                );
                x += w + 2;
            }
            dpi::move_control(
                self.metrics.dpi,
                self.body,
                2,
                top + 26,
                (width - 4).max(80),
                (height - top - 28).max(60),
                1,
            );
        }
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if msg == WM_NCCREATE {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
            );
        }
        if msg == WM_NOTIFY && !lp.eq(&0) {
            let notification = &*(lp as *const NMHDR);
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ComposeState;
            if let Some(state) = pointer.as_ref() {
                if notification.hwndFrom == state.body
                    && notification.code == rich::EN_SELCHANGE
                    && !state.loading.get()
                {
                    rich::sync(state);
                    return 0;
                }
            }
        }
        if msg == WM_NOTIFY
            && let Some(result) = classic::custom_draw(lp)
        {
            return result;
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ComposeState;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        if msg == WM_CREATE {
            (*pointer).setup(hwnd);
            return 0;
        }
        if msg == WM_DPICHANGED {
            (*pointer).update_dpi((wp & 0xffff) as u32);
            dpi::suggested(hwnd, lp);
            (*pointer).layout();
            return 0;
        }
        let state = &*pointer;
        match msg {
            WM_GETMINMAXINFO => {
                let info = &mut *(lp as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT {
                    x: dpi::scale(600, state.metrics.dpi),
                    y: dpi::scale(400, state.metrics.dpi),
                };
                0
            }
            WM_SIZE => {
                state.layout();
                0
            }
            WM_COMMAND => {
                let id = wp & 0xffff;
                let notification = wp >> 16;
                if (rich::FIRST as usize..rich::FIRST as usize + 8).contains(&id) {
                    let index = id - rich::FIRST as usize;
                    if state.rich.get()
                        && ((index < 2 && notification == CBN_SELCHANGE as usize)
                            || (index >= 2 && notification == BN_CLICKED as usize))
                    {
                        rich::command(state, index);
                    }
                    return 0;
                }
                if (201..=205).contains(&id) && notification == EN_CHANGE as usize {
                    if !state.loading.get() {
                        state.dirty.set(true);
                    }
                    return 0;
                }
                if id == 206 && notification == CBN_SELCHANGE as usize {
                    let index = SendMessageW(state.sender, CB_GETCURSEL, 0, 0);
                    *state.sender_identity.borrow_mut() =
                        usize::try_from(index).ok().and_then(|index| {
                            state
                                .accounts
                                .borrow()
                                .get(index)
                                .map(|account| account.id.clone())
                        });
                    if !state.loading.get() {
                        state.dirty.set(true);
                    }
                    return 0;
                }
                match id {
                    menus::CUT..=menus::SELECT_ALL => {
                        let focus = windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus();
                        SendMessageW(
                            focus,
                            match id {
                                menus::CUT => WM_CUT,
                                menus::COPY => WM_COPY,
                                menus::PASTE => WM_PASTE,
                                _ => EM_SETSEL,
                            },
                            0,
                            if id == menus::SELECT_ALL { -1 } else { 0 },
                        );
                    }
                    menus::TOGGLE_BCC => {
                        state.show_bcc.set(!state.show_bcc.get());
                        state.layout();
                    }
                    menus::PLAIN_TEXT | menus::RICH_TEXT => {
                        state.rich.set(id == menus::RICH_TEXT);
                        state.dirty.set(true);
                        rich::sync(state);
                    }
                    _ => {
                        PostMessageW(state.owner, DISPATCH, id, hwnd as isize);
                    }
                }
                0
            }
            WM_CLOSE => {
                PostMessageW(
                    state.owner,
                    DISPATCH,
                    command_id(Command::CloseComposer) as usize,
                    hwnd as isize,
                );
                0
            }
            WM_DESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
