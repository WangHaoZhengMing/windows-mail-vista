//! Account catalog list, selection and management commands.
use super::*;
use windows_sys::Win32::Graphics::Gdi::{COLOR_WINDOWTEXT, GetSysColor};

pub(super) struct ManagerHost {
    pub(super) owner: HWND,
    pub(super) submissions: Sender<AccountSubmission>,
    updates: Receiver<AccountUpdate>,
    pub(super) update_sender: Sender<AccountUpdate>,
    pub(super) property: Cell<HWND>,
    pub(super) embedded: HWND,
    state: RefCell<AccountManager>,
    // Creation owns the Box until CreateWindowExW succeeds. Failed creation can
    // dispatch WM_NCDESTROY before returning, so that callback cannot free it.
    window_owned: Cell<bool>,
}

struct AccountManager {
    layout: WindowLayout,
    snapshot: AccountsSnapshot,
    list: HWND,
    edit: HWND,
    remove: HWND,
    make_default: HWND,
    status: HWND,
    error: bool,
}

impl AccountManager {
    fn new(snapshot: AccountsSnapshot) -> Self {
        Self {
            layout: WindowLayout::default(),
            snapshot,
            list: null_mut(),
            edit: null_mut(),
            remove: null_mut(),
            make_default: null_mut(),
            status: null_mut(),
            error: false,
        }
    }

    unsafe fn build(&mut self, window: HWND) {
        unsafe {
            self.layout.control(
                window,
                "STATIC",
                "Manage your mail accounts",
                0,
                0,
                [14, 14, 750, 28],
            );
            self.list = self.layout.control(
                window,
                "SysListView32",
                "",
                WS_TABSTOP | WS_BORDER | LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS,
                ACCOUNT_LIST,
                [14, 52, 596, 180],
            );
            SendMessageW(
                self.list,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                0,
                (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
            );
            for (index, caption) in ["Account name", "Email address", "Type", "Status"]
                .into_iter()
                .enumerate()
            {
                let mut caption = wide(caption);
                let column = LVCOLUMNW {
                    mask: LVCF_TEXT | LVCF_WIDTH,
                    cx: 100,
                    pszText: caption.as_mut_ptr(),
                    ..std::mem::zeroed()
                };
                SendMessageW(
                    self.list,
                    LVM_INSERTCOLUMNW,
                    index,
                    &column as *const _ as isize,
                );
            }
            self.layout.control(
                window,
                "BUTTON",
                "&Add...",
                WS_TABSTOP,
                ADD_ACCOUNT,
                [628, 52, 140, 28],
            );
            self.edit = self.layout.control(
                window,
                "BUTTON",
                "&Properties...",
                WS_TABSTOP,
                EDIT_ACCOUNT,
                [628, 90, 140, 28],
            );
            self.remove = self.layout.control(
                window,
                "BUTTON",
                "&Remove",
                WS_TABSTOP,
                REMOVE_ACCOUNT,
                [628, 128, 140, 28],
            );
            self.make_default = self.layout.control(
                window,
                "BUTTON",
                "Set &Default",
                WS_TABSTOP,
                DEFAULT_ACCOUNT,
                [628, 166, 140, 28],
            );
            self.status = self
                .layout
                .control(window, "STATIC", "", 0, 0, [14, 248, 754, 42]);
            self.layout.control(
                window,
                "BUTTON",
                "&Close",
                WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
                IDCANCEL as u16,
                [628, 297, 140, 28],
            );
            self.layout.update(window, dpi::window_dpi(window));
            self.columns();
            self.refresh(None);
        }
    }

    unsafe fn columns(&self) {
        unsafe {
            for (index, width) in [180, 250, 70, 90].into_iter().enumerate() {
                SendMessageW(
                    self.list,
                    LVM_SETCOLUMNWIDTH,
                    index,
                    dpi::scale(width, self.layout.metrics.dpi) as isize,
                );
            }
        }
    }

    unsafe fn selected(&self) -> Option<&AccountRecord> {
        unsafe {
            let index = SendMessageW(
                self.list,
                LVM_GETNEXTITEM,
                usize::MAX,
                LVNI_SELECTED as isize,
            );
            usize::try_from(index)
                .ok()
                .and_then(|index| self.snapshot.accounts.get(index))
        }
    }

    unsafe fn refresh(&mut self, requested: Option<&AccountId>) {
        unsafe {
            let selected = requested
                .cloned()
                .or_else(|| self.selected().map(|record| record.id.clone()))
                .or_else(|| self.snapshot.default_account.clone());
            SendMessageW(self.list, LVM_DELETEALLITEMS, 0, 0);
            for (index, record) in self.snapshot.accounts.iter().enumerate() {
                let protocol = match record.account.incoming {
                    Incoming::Imap => "IMAP",
                    Incoming::Pop3 => "POP3",
                };
                let status = if self.snapshot.default_account.as_ref() == Some(&record.id) {
                    "Default"
                } else {
                    ""
                };
                for (column, value) in [
                    record.name.as_str(),
                    &record.account.email,
                    protocol,
                    status,
                ]
                .into_iter()
                .enumerate()
                {
                    let mut value = wide(value);
                    let item = LVITEMW {
                        mask: LVIF_TEXT,
                        iItem: index as i32,
                        iSubItem: column as i32,
                        pszText: value.as_mut_ptr(),
                        ..std::mem::zeroed()
                    };
                    SendMessageW(
                        self.list,
                        if column == 0 {
                            LVM_INSERTITEMW
                        } else {
                            LVM_SETITEMTEXTW
                        },
                        if column == 0 { 0 } else { index },
                        &item as *const _ as isize,
                    );
                }
            }
            if !self.snapshot.accounts.is_empty() {
                let index = self
                    .snapshot
                    .accounts
                    .iter()
                    .position(|record| Some(&record.id) == selected.as_ref())
                    .unwrap_or(0);
                let item = LVITEMW {
                    state: LVIS_SELECTED | LVIS_FOCUSED,
                    stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                    ..std::mem::zeroed()
                };
                SendMessageW(
                    self.list,
                    LVM_SETITEMSTATE,
                    index,
                    &item as *const _ as isize,
                );
                SendMessageW(self.list, LVM_ENSUREVISIBLE, index, 0);
            }
            self.error = false;
            set_window_text(
                self.status,
                if self.snapshot.accounts.is_empty() {
                    "Add an account to send and receive mail."
                } else {
                    "Select an account to edit its settings. Removing an account keeps downloaded mail and drafts in Local Folders."
                },
            );
            self.buttons();
        }
    }

    unsafe fn buttons(&self) {
        unsafe {
            let selected = self.selected();
            EnableWindow(self.edit, i32::from(selected.is_some()));
            EnableWindow(self.remove, i32::from(selected.is_some()));
            EnableWindow(
                self.make_default,
                i32::from(selected.is_some_and(|record| {
                    self.snapshot.default_account.as_ref() != Some(&record.id)
                })),
            );
        }
    }
}

pub(crate) unsafe fn open(
    owner: HWND,
    accounts: Vec<AccountRecord>,
    default_account: Option<AccountId>,
    submissions: Sender<AccountSubmission>,
) -> Result<HWND> {
    unsafe {
        if IsWindow(owner) == 0 {
            return Err("The account manager requires a live mail window.".into());
        }
        let class = wide("WinMailRustAccountManager");
        let mut existing = FindWindowExW(null_mut(), null_mut(), class.as_ptr(), null());
        while !existing.is_null() {
            if GetWindow(existing, GW_OWNER) == owner {
                let host = GetWindowLongPtrW(existing, GWLP_USERDATA) as *const ManagerHost;
                let property = if host.is_null() {
                    null_mut()
                } else {
                    (*host).property.get()
                };
                let popup = if IsWindow(property) != 0 {
                    property
                } else {
                    existing
                };
                ShowWindow(popup, SW_SHOW);
                SetForegroundWindow(popup);
                return Ok(existing);
            }
            existing = FindWindowExW(null_mut(), existing, class.as_ptr(), null());
        }
        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        };
        InitCommonControlsEx(&controls);
        register_class(&class, Some(manager_proc))?;
        let (update_sender, updates) = mpsc::channel();
        let state = Box::new(ManagerHost {
            owner,
            submissions,
            updates,
            update_sender,
            property: Cell::new(null_mut()),
            embedded: GetPropW(owner, wide("WinMailSetupHost").as_ptr()),
            state: RefCell::new(AccountManager::new(AccountsSnapshot {
                accounts,
                default_account,
            })),
            window_owned: Cell::new(false),
        });
        let window = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide("Accounts - Windows Mail Vista").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            800,
            380,
            owner,
            null_mut(),
            GetModuleHandleW(null()),
            (&*state as *const ManagerHost).cast(),
        );
        if window.is_null() {
            return Err("Could not create account manager.".into());
        }
        state.window_owned.set(true);
        let list = state.state.borrow().list;
        let embedded = state.embedded;
        let _ = Box::into_raw(state);
        let value = dpi::window_dpi(window);
        SetWindowPos(
            window,
            null_mut(),
            0,
            0,
            dpi::scale(800, value),
            dpi::scale(380, value),
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        EnableWindow(owner, 0);
        if embedded.is_null() {
            ShowWindow(window, SW_SHOW);
            SetForegroundWindow(window);
            SetFocus(list);
        }
        Ok(window)
    }
}

pub(super) unsafe fn error(window: HWND, message: &str) {
    unsafe {
        let host = GetWindowLongPtrW(window, GWLP_USERDATA) as *const ManagerHost;
        if host.is_null() {
            return;
        }
        if let Ok(mut state) = (*host).state.try_borrow_mut() {
            state.error = true;
            set_window_text(state.status, message);
            InvalidateRect(state.status, null(), 1);
        }
    }
}

unsafe fn manager_change(window: HWND, command: u16) {
    unsafe {
        let host = GetWindowLongPtrW(window, GWLP_USERDATA) as *const ManagerHost;
        if host.is_null() {
            return;
        }
        let selected = {
            let Ok(state) = (*host).state.try_borrow() else {
                return;
            };
            state.selected().cloned()
        };
        if command == ADD_ACCOUNT {
            wizard::open(window);
            return;
        }
        let Some(selected) = selected else {
            return;
        };
        if command == EDIT_ACCOUNT || command == IDOK as u16 {
            properties::open(window, Some(selected));
            return;
        }
        let owner = (*host).owner;
        let submissions = (*host).submissions.clone();
        let change = match command {
            REMOVE_ACCOUNT => {
                let question = format!(
                    "Remove {} ({})?\n\nDownloaded mail and saved drafts will be kept in Local Folders.",
                    selected.name, selected.account.email
                );
                if MessageBoxW(
                    window,
                    wide(&question).as_ptr(),
                    wide("Remove Account").as_ptr(),
                    MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                ) != IDYES
                {
                    return;
                }
                AccountChange::Remove {
                    id: selected.id.clone(),
                }
            }
            DEFAULT_ACCOUNT => AccountChange::SetDefault {
                id: selected.id.clone(),
            },
            _ => return,
        };
        if IsWindow(window) == 0 {
            return;
        }
        let result = submit_change(owner, &submissions, change);
        if IsWindow(window) == 0 {
            return;
        }
        let host = GetWindowLongPtrW(window, GWLP_USERDATA) as *const ManagerHost;
        if host.is_null() {
            return;
        }
        match result {
            Ok(snapshot) => {
                if let Ok(mut state) = (*host).state.try_borrow_mut() {
                    state.snapshot = snapshot;
                    state.refresh(Some(&selected.id));
                }
            }
            Err(message) => error(window, &message),
        }
    }
}

unsafe extern "system" fn manager_proc(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
            return 1;
        }
        let host = GetWindowLongPtrW(window, GWLP_USERDATA) as *const ManagerHost;
        if host.is_null() {
            return DefWindowProcW(window, message, wp, lp);
        }
        match message {
            WM_CREATE => {
                if let Ok(mut state) = (*host).state.try_borrow_mut() {
                    state.build(window);
                }
                0
            }
            WM_COMMAND => {
                let command = (wp & 0xffff) as u16;
                if command == IDCANCEL as u16 {
                    DestroyWindow(window);
                } else {
                    manager_change(window, command);
                }
                0
            }
            WM_NOTIFY => {
                // NMLVKEYDOWN is packed; its leading NMHDR need not be aligned.
                let header = std::ptr::read_unaligned(lp as *const NMHDR);
                let double_click =
                    header.idFrom == ACCOUNT_LIST as usize && header.code == NM_DBLCLK;
                if let Ok(state) = (*host).state.try_borrow() {
                    state.buttons();
                }
                if double_click {
                    manager_change(window, EDIT_ACCOUNT);
                }
                0
            }
            SNAPSHOT_UPDATED => {
                while let Ok(update) = (*host).updates.try_recv() {
                    if let Ok(mut state) = (*host).state.try_borrow_mut() {
                        state.snapshot = update.snapshot;
                        state.refresh(update.selected.as_ref());
                    }
                }
                0
            }
            PROPERTY_CLOSED => {
                if (*host).property.get() as usize == wp {
                    (*host).property.set(null_mut());
                }
                0
            }
            WM_DPICHANGED => {
                if let Ok(mut state) = (*host).state.try_borrow_mut() {
                    state.layout.update(window, (wp & 0xffff) as u32);
                    state.columns();
                }
                dpi::suggested(window, lp);
                0
            }
            WM_CTLCOLORSTATIC => {
                if let Ok(state) = (*host).state.try_borrow()
                    && state.error
                    && lp as HWND == state.status
                {
                    return error_brush(wp);
                }
                SetTextColor(wp as HDC, GetSysColor(COLOR_WINDOWTEXT));
                SetBkMode(wp as HDC, TRANSPARENT as i32);
                GetSysColorBrush(COLOR_WINDOW) as isize
            }
            WM_CLOSE => {
                DestroyWindow(window);
                0
            }
            WM_NCDESTROY => {
                let owner = (*host).owner;
                let owned = (*host).window_owned.get();
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                if owned {
                    drop(Box::from_raw(host.cast_mut()));
                    if IsWindow(owner) != 0 {
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
