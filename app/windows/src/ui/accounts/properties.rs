//! Modal account properties and inline validation feedback.
use super::manager::ManagerHost;
use super::*;
use windows_sys::Win32::Graphics::Gdi::{COLOR_WINDOWTEXT, GetSysColor};

pub(super) const AUTHENTICATION: u16 = 323;
pub(super) const CONFIGURE_OAUTH: u16 = 324;
const TEST_CONNECTION: u16 = 325;
const TEST_TIMER: usize = 326;
type TestResult = (mail_core::network::ConnectionTest, Account);

pub(super) struct PropertyHost {
    owner: HWND,
    main: HWND,
    submissions: Sender<AccountSubmission>,
    updates: Sender<AccountUpdate>,
    pub(super) oauth_window: Cell<HWND>,
    oauth_sender: Sender<mail_core::oauth::OAuthAccount>,
    oauth_updates: Receiver<mail_core::oauth::OAuthAccount>,
    pub(super) state: RefCell<AccountProperties>,
    window_owned: Cell<bool>,
    connection_test: RefCell<Option<Receiver<TestResult>>>,
}

pub(super) struct AccountProperties {
    layout: WindowLayout,
    previous: Option<AccountRecord>,
    edits: [HWND; Field::COUNT],
    protocol: HWND,
    authentication: HWND,
    configure_oauth: HWND,
    auth_hint: HWND,
    pub(super) oauth_account: Option<mail_core::oauth::OAuthAccount>,
    incoming_security: HWND,
    smtp_security: HWND,
    pub(super) status: HWND,
    pub(super) error: bool,
}

impl AccountProperties {
    unsafe fn test_busy(&self, window: HWND, busy: bool) {
        unsafe {
            for control in self.edits.iter().copied().chain([
                self.protocol,
                self.authentication,
                self.configure_oauth,
                self.incoming_security,
                self.smtp_security,
                GetDlgItem(window, SAVE_ACCOUNT as i32),
                GetDlgItem(window, TEST_CONNECTION as i32),
            ]) {
                EnableWindow(control, i32::from(!busy));
            }
            set_window_text(
                GetDlgItem(window, TEST_CONNECTION as i32),
                if busy {
                    "Testing..."
                } else {
                    "Test connection"
                },
            );
        }
    }
    fn new(previous: Option<AccountRecord>) -> Self {
        let oauth_account = previous
            .as_ref()
            .and_then(|record| match &record.account.auth {
                Authentication::OAuth2(oauth) => Some(oauth.clone()),
                Authentication::Password => None,
            });
        Self {
            layout: WindowLayout::default(),
            previous,
            edits: [null_mut(); Field::COUNT],
            protocol: null_mut(),
            authentication: null_mut(),
            configure_oauth: null_mut(),
            auth_hint: null_mut(),
            oauth_account,
            incoming_security: null_mut(),
            smtp_security: null_mut(),
            status: null_mut(),
            error: false,
        }
    }

    unsafe fn edit(&mut self, window: HWND, field: Field, label: &str, rect: [i32; 4]) {
        unsafe {
            let [x, y, width, height] = rect;
            self.layout
                .control(window, "STATIC", label, 0, 0, [28, y + 3, 130, height]);
            self.edits[field as usize] = self.layout.control(
                window,
                "EDIT",
                &field.initial_text(self.previous.as_ref()),
                WS_TABSTOP
                    | ES_AUTOHSCROLL as u32
                    | if field == Field::Password {
                        ES_PASSWORD as u32
                    } else {
                        0
                    },
                300 + field as u16,
                [x, y, width, height],
            );
            SendMessageW(
                self.edits[field as usize],
                EM_SETLIMITTEXT,
                if field == Field::Password { 4096 } else { 1024 },
                0,
            );
        }
    }

    unsafe fn build(&mut self, window: HWND) {
        unsafe {
            self.layout.control(
                window,
                "BUTTON",
                "General",
                BS_GROUPBOX as u32,
                0,
                [14, 12, 774, 174],
            );
            for (field, label, y) in [
                (Field::Name, "Account &name:", 40),
                (Field::Email, "&Email address:", 72),
                (Field::Username, "&Username:", 104),
            ] {
                self.edit(window, field, label, [170, y, 596, 26]);
            }
            self.edit(
                window,
                Field::Password,
                "&Authentication:",
                [414, 136, 352, 26],
            );
            self.authentication = self.layout.control(
                window,
                "COMBOBOX",
                "",
                WS_TABSTOP | CBS_DROPDOWNLIST as u32,
                AUTHENTICATION,
                [170, 136, 180, 130],
            );
            for value in ["Password", "OAuth 2.0"] {
                SendMessageW(
                    self.authentication,
                    CB_ADDSTRING,
                    0,
                    wide(value).as_ptr() as isize,
                );
            }
            SendMessageW(
                self.authentication,
                CB_SETCURSEL,
                usize::from(self.oauth_account.is_some()),
                0,
            );
            self.configure_oauth = self.layout.control(
                window,
                "BUTTON",
                "OAuth settings...",
                WS_TABSTOP,
                CONFIGURE_OAUTH,
                [414, 136, 352, 26],
            );
            self.auth_hint = self
                .layout
                .control(window, "STATIC", "", 0, 0, [218, 166, 548, 18]);
            self.layout.control(
                window,
                "BUTTON",
                "Incoming mail",
                BS_GROUPBOX as u32,
                0,
                [14, 196, 774, 132],
            );
            self.layout
                .control(window, "STATIC", "&Protocol:", 0, 0, [28, 222, 178, 26]);
            self.protocol = self.layout.control(
                window,
                "COMBOBOX",
                "",
                WS_TABSTOP | CBS_DROPDOWNLIST as u32,
                320,
                [218, 218, 150, 130],
            );
            for value in ["IMAP", "POP3"] {
                SendMessageW(
                    self.protocol,
                    CB_ADDSTRING,
                    0,
                    wide(value).as_ptr() as isize,
                );
            }
            let previous = self.previous.as_ref().map(|record| &record.account);
            let incoming_security =
                previous.map_or(Security::Tls, |account| account.incoming_security);
            let smtp_security =
                previous.map_or(Security::StartTls, |account| account.smtp_security);
            SendMessageW(
                self.protocol,
                CB_SETCURSEL,
                usize::from(previous.is_some_and(|account| account.incoming == Incoming::Pop3)),
                0,
            );
            self.layout.control(
                window,
                "STATIC",
                "TLS ports: IMAP 993, POP3 995",
                0,
                0,
                [386, 222, 380, 26],
            );
            self.edit(
                window,
                Field::IncomingHost,
                "Incoming &server:",
                [170, 254, 596, 26],
            );
            self.edit(
                window,
                Field::IncomingPort,
                "Incoming p&ort:",
                [170, 290, 90, 26],
            );
            self.layout
                .control(window, "STATIC", "Encryption:", 0, 0, [344, 293, 100, 26]);
            self.incoming_security =
                self.security(window, 321, [452, 290, 314, 140], incoming_security);
            self.layout.control(
                window,
                "BUTTON",
                "Outgoing mail (SMTP)",
                BS_GROUPBOX as u32,
                0,
                [14, 340, 774, 102],
            );
            self.edit(
                window,
                Field::SmtpHost,
                "SMTP ser&ver:",
                [170, 366, 596, 26],
            );
            self.edit(window, Field::SmtpPort, "SMTP por&t:", [170, 400, 90, 26]);
            self.layout
                .control(window, "STATIC", "Encryption:", 0, 0, [344, 403, 100, 26]);
            self.smtp_security = self.security(window, 322, [452, 400, 314, 140], smtp_security);
            self.status = self.layout.control(window, "STATIC",
                "An empty account name uses the email address. Changes are saved only when you choose OK.",
                0, 0, [14, 454, 774, 44]);
            self.layout.control(
                window,
                "BUTTON",
                "&OK",
                WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
                SAVE_ACCOUNT,
                [582, 514, 96, 30],
            );
            self.layout.control(
                window,
                "BUTTON",
                "&Cancel",
                WS_TABSTOP,
                IDCANCEL as u16,
                [690, 514, 96, 30],
            );
            self.layout.control(
                window,
                "BUTTON",
                "Test connection",
                WS_TABSTOP,
                TEST_CONNECTION,
                [14, 514, 150, 30],
            );
            self.update_authentication();
            self.layout.update(window, dpi::window_dpi(window));
        }
    }

    unsafe fn security(
        &mut self,
        window: HWND,
        id: u16,
        rect: [i32; 4],
        security: Security,
    ) -> HWND {
        unsafe {
            let combo = self.layout.control(
                window,
                "COMBOBOX",
                "",
                WS_TABSTOP | CBS_DROPDOWNLIST as u32,
                id,
                rect,
            );
            for caption in ["TLS (implicit)", "STARTTLS", "Unencrypted (localhost only)"] {
                SendMessageW(combo, CB_ADDSTRING, 0, wide(caption).as_ptr() as isize);
            }
            SendMessageW(
                combo,
                CB_SETCURSEL,
                match security {
                    Security::Tls => 0,
                    Security::StartTls => 1,
                    Security::Plain => 2,
                },
                0,
            );
            combo
        }
    }

    unsafe fn input(&self) -> FormInput {
        unsafe {
            let security = |window| match SendMessageW(window, CB_GETCURSEL, 0, 0) {
                1 => Security::StartTls,
                2 => Security::Plain,
                _ => Security::Tls,
            };
            FormInput {
                fields: std::array::from_fn(|index| text(self.edits[index])),
                authentication: if self.uses_oauth() {
                    Authentication::OAuth2(self.oauth_account.clone().unwrap_or_else(|| {
                        mail_core::oauth::OAuthAccount {
                            config: mail_core::oauth::OAuthConfig::microsoft(""),
                            tokens: None,
                        }
                    }))
                } else {
                    Authentication::Password
                },
                incoming: if SendMessageW(self.protocol, CB_GETCURSEL, 0, 0) == 1 {
                    Incoming::Pop3
                } else {
                    Incoming::Imap
                },
                incoming_security: security(self.incoming_security),
                smtp_security: security(self.smtp_security),
            }
        }
    }

    unsafe fn uses_oauth(&self) -> bool {
        unsafe { SendMessageW(self.authentication, CB_GETCURSEL, 0, 0) == 1 }
    }

    unsafe fn update_authentication(&self) {
        unsafe {
            let oauth = self.uses_oauth();
            ShowWindow(
                self.edits[Field::Password as usize],
                if oauth { SW_HIDE } else { SW_SHOW },
            );
            ShowWindow(self.configure_oauth, if oauth { SW_SHOW } else { SW_HIDE });
            set_window_text(
                self.auth_hint,
                if oauth {
                    if self
                        .oauth_account
                        .as_ref()
                        .is_some_and(mail_core::oauth::OAuthAccount::signed_in)
                    {
                        "Signed in with OAuth. Passwords are not used for this account."
                    } else {
                        "Open OAuth settings to configure your client ID and sign in."
                    }
                } else if self
                    .previous
                    .as_ref()
                    .is_some_and(|record| matches!(record.account.auth, Authentication::Password))
                {
                    "Leave password empty to keep the saved password."
                } else {
                    "Enter a password or provider app password for password sign-in."
                },
            );
            if oauth {
                if SendMessageW(self.incoming_security, CB_GETCURSEL, 0, 0) == 2 {
                    SendMessageW(self.incoming_security, CB_SETCURSEL, 0, 0);
                }
                if SendMessageW(self.smtp_security, CB_GETCURSEL, 0, 0) == 2 {
                    SendMessageW(self.smtp_security, CB_SETCURSEL, 1, 0);
                }
            }
        }
    }

    unsafe fn failure(&mut self, error: ValidationError) {
        unsafe {
            self.error = true;
            set_window_text(self.status, &error.message);
            if error.field == Field::Password && self.uses_oauth() {
                SetFocus(self.configure_oauth);
            } else {
                SetFocus(self.edits[error.field as usize]);
                SendMessageW(self.edits[error.field as usize], EM_SETSEL, 0, -1);
            }
            InvalidateRect(self.status, null(), 1);
        }
    }
}

pub(super) unsafe fn open(manager: HWND, previous: Option<AccountRecord>) {
    unsafe {
        let host = GetWindowLongPtrW(manager, GWLP_USERDATA) as *const ManagerHost;
        if host.is_null() {
            return;
        }
        let property = (*host).property.get();
        if !property.is_null() && IsWindow(property) != 0 {
            SetForegroundWindow(property);
            return;
        }
        let main = (*host).owner;
        let submissions = (*host).submissions.clone();
        let updates = (*host).update_sender.clone();
        let class = wide("WinMailRustAccountProperties");
        if let Err(error) = register_class(&class, Some(property_proc)) {
            manager::error(manager, &error);
            return;
        }
        let title = previous.as_ref().map_or_else(
            || "Add Mail Account".to_owned(),
            |record| format!("{} - Account Properties", record.name),
        );
        let (oauth_sender, oauth_updates) = mpsc::channel();
        let state = Box::new(PropertyHost {
            owner: manager,
            main,
            submissions,
            updates,
            oauth_window: Cell::new(null_mut()),
            oauth_sender,
            oauth_updates,
            state: RefCell::new(AccountProperties::new(previous)),
            window_owned: Cell::new(false),
            connection_test: RefCell::new(None),
        });
        let embedded = (*host).embedded;
        let window = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide(&title).as_ptr(),
            if embedded.is_null() {
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU
            } else {
                WS_CHILD | WS_CLIPCHILDREN
            },
            if embedded.is_null() { CW_USEDEFAULT } else { 0 },
            if embedded.is_null() { CW_USEDEFAULT } else { 0 },
            820,
            600,
            if embedded.is_null() {
                manager
            } else {
                embedded
            },
            null_mut(),
            GetModuleHandleW(null()),
            (&*state as *const PropertyHost).cast(),
        );
        if window.is_null() {
            manager::error(manager, "Could not create account properties.");
            return;
        }
        state.window_owned.set(true);
        let edit = state.state.borrow().edits[Field::Name as usize];
        let _ = Box::into_raw(state);
        (*host).property.set(window);
        let value = dpi::window_dpi(window);
        SetWindowPos(
            window,
            null_mut(),
            0,
            0,
            dpi::scale(820, value),
            dpi::scale(600, value),
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        EnableWindow(manager, 0);
        present_setup(manager, window, 802, 560);
        if !embedded.is_null() {
            set_window_text(GetDlgItem(window, SAVE_ACCOUNT as i32), "Save & continue");
        }
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
        SetFocus(edit);
    }
}

pub(super) unsafe fn open_preset(
    manager: HWND,
    provider: &super::wizard::Provider,
    email: &str,
    name: &str,
) {
    unsafe {
        open(manager, None);
        let host = GetWindowLongPtrW(manager, GWLP_USERDATA) as *const ManagerHost;
        if !host.is_null() {
            let window = (*host).property.get();
            if IsWindow(window) != 0 {
                super::wizard::populate(window, provider, email, name);
                let property = GetWindowLongPtrW(window, GWLP_USERDATA) as *const PropertyHost;
                if !property.is_null() {
                    let status = (*property).state.borrow().status;
                    set_window_text(status, provider.hint);
                }
            }
        }
    }
}

unsafe fn property_save(window: HWND) {
    unsafe {
        let host = GetWindowLongPtrW(window, GWLP_USERDATA) as *const PropertyHost;
        if host.is_null() {
            return;
        }
        let oauth_window = (*host).oauth_window.get();
        if !oauth_window.is_null() && IsWindow(oauth_window) != 0 {
            SetForegroundWindow(oauth_window);
            return;
        }
        let (change, identity, selected) = {
            let Ok(mut state) = (*host).state.try_borrow_mut() else {
                return;
            };
            let input = state.input();
            let (name, account) = match input.account(state.previous.as_ref()) {
                Ok(account) => account,
                Err(error) => {
                    state.failure(error);
                    return;
                }
            };
            let identity = account.id();
            let selected = state.previous.as_ref().map(|record| record.id.clone());
            let change = match selected.clone() {
                Some(id) => AccountChange::Update { id, name, account },
                None => AccountChange::Add { name, account },
            };
            (change, identity, selected)
        };
        let main = (*host).main;
        let owner = (*host).owner;
        let submissions = (*host).submissions.clone();
        let updates = (*host).updates.clone();
        let result = submit_change(main, &submissions, change);
        if IsWindow(window) == 0 {
            return;
        }
        let host = GetWindowLongPtrW(window, GWLP_USERDATA) as *const PropertyHost;
        if host.is_null() {
            return;
        }
        match result {
            Ok(snapshot) => {
                let embedded = setup_parent(owner);
                if !embedded.is_null() {
                    PostMessageW(embedded, super::super::setup::ACCOUNT_SAVED, 0, 0);
                }
                let selected = selected.or_else(|| {
                    snapshot
                        .accounts
                        .iter()
                        .find(|record| record.account.id() == identity)
                        .map(|record| record.id.clone())
                });
                if updates.send(AccountUpdate { snapshot, selected }).is_ok() {
                    PostMessageW(owner, SNAPSHOT_UPDATED, 0, 0);
                }
                DestroyWindow(window);
            }
            Err(message) => {
                if let Ok(mut state) = (*host).state.try_borrow_mut() {
                    state.failure(ValidationError::new(Field::Name, message));
                }
            }
        }
    }
}

unsafe fn configure_oauth(window: HWND, host: &PropertyHost) {
    unsafe {
        if IsWindow(host.oauth_window.get()) != 0 {
            SetForegroundWindow(host.oauth_window.get());
            return;
        }
        let (account, login_hint) = {
            let Ok(state) = host.state.try_borrow() else {
                return;
            };
            if !state.uses_oauth() {
                return;
            }
            (
                state.oauth_account.clone(),
                text(state.edits[Field::Email as usize]),
            )
        };
        match oauth::open(window, account, login_hint, host.oauth_sender.clone()) {
            Ok(child) => host.oauth_window.set(child),
            Err(message) => {
                if let Ok(mut state) = host.state.try_borrow_mut() {
                    state.failure(ValidationError::new(Field::Password, message));
                }
            }
        }
    }
}

unsafe fn test_connection(window: HWND, host: &PropertyHost) {
    unsafe {
        if host.connection_test.borrow().is_some() || IsWindow(host.oauth_window.get()) != 0 {
            return;
        }
        let mut account = {
            let Ok(mut state) = host.state.try_borrow_mut() else {
                return;
            };
            match state.input().account(state.previous.as_ref()) {
                Ok((_, account)) => account,
                Err(error) => {
                    state.failure(error);
                    return;
                }
            }
        };
        if SetTimer(window, TEST_TIMER, 200, None) == 0 {
            manager::error(window, "Could not start connection test.");
            return;
        }
        let (sender, receiver) = mpsc::channel();
        *host.connection_test.borrow_mut() = Some(receiver);
        {
            let state = host.state.borrow();
            state.test_busy(window, true);
            set_window_text(
                state.status,
                "Testing incoming and outgoing login. No messages will be sent or downloaded.",
            );
        }
        let result = std::thread::Builder::new()
            .name("mail-connection-test".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if matches!(account.auth, Authentication::OAuth2(_)) {
                        mail_core::oauth::access_token(&mut account)?;
                    }
                    Ok::<_, String>(mail_core::network::test_connection(&account))
                }));
                let report = match outcome {
                    Ok(Ok(report)) => report,
                    error => {
                        let message = match error {
                            Ok(Err(error)) => error,
                            _ => "Connection test failed unexpectedly".into(),
                        };
                        mail_core::network::ConnectionTest {
                            incoming: Err(message.clone()),
                            outgoing: Err(message),
                        }
                    }
                };
                let _ = sender.send((report, account));
            });
        if let Err(error) = result {
            host.connection_test.borrow_mut().take();
            KillTimer(window, TEST_TIMER);
            host.state.borrow().test_busy(window, false);
            manager::error(window, &format!("Could not start connection test: {error}"));
        }
    }
}

unsafe extern "system" fn property_proc(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
            return 1;
        }
        let host = GetWindowLongPtrW(window, GWLP_USERDATA) as *const PropertyHost;
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
                match (wp & 0xffff) as u16 {
                    SAVE_ACCOUNT => property_save(window),
                    CONFIGURE_OAUTH => configure_oauth(window, &*host),
                    TEST_CONNECTION => test_connection(window, &*host),
                    AUTHENTICATION if (wp >> 16) as u32 == CBN_SELCHANGE => {
                        if let Ok(state) = (*host).state.try_borrow() {
                            state.update_authentication();
                        }
                    }
                    command if command == IDCANCEL as u16 => {
                        DestroyWindow(window);
                    }
                    _ => {}
                }
                0
            }
            oauth::OAUTH_UPDATED => {
                while let Ok(account) = (*host).oauth_updates.try_recv() {
                    if let Ok(mut state) = (*host).state.try_borrow_mut() {
                        state.oauth_account = Some(account);
                        state.update_authentication();
                    }
                }
                0
            }
            WM_TIMER if wp == TEST_TIMER => {
                let result = (*host)
                    .connection_test
                    .borrow()
                    .as_ref()
                    .and_then(|receiver| receiver.try_recv().ok());
                if let Some((report, account)) = result {
                    (*host).connection_test.borrow_mut().take();
                    KillTimer(window, TEST_TIMER);
                    if let Ok(mut state) = (*host).state.try_borrow_mut() {
                        if let Authentication::OAuth2(oauth) = account.auth {
                            state.oauth_account = Some(oauth);
                        }
                        state.test_busy(window, false);
                        let describe = |result: &Result<()>| match result {
                            Ok(()) => "Connected and authenticated".to_owned(),
                            Err(error) => format!("Failed: {error}"),
                        };
                        let message = format!(
                            "Incoming: {}\nOutgoing: {}",
                            describe(&report.incoming),
                            describe(&report.outgoing)
                        );
                        set_window_text(state.status, &message);
                        state.error = report.incoming.is_err() || report.outgoing.is_err();
                        let icon = if state.error {
                            MB_ICONWARNING
                        } else {
                            MB_ICONINFORMATION
                        };
                        drop(state);
                        MessageBoxW(
                            window,
                            wide(&message).as_ptr(),
                            wide("Connection test").as_ptr(),
                            MB_OK | icon,
                        );
                    }
                }
                0
            }
            oauth::OAUTH_CLOSED => {
                if (*host).oauth_window.get() as usize == wp {
                    (*host).oauth_window.set(null_mut());
                }
                0
            }
            WM_DESTROY => {
                let child = (*host).oauth_window.replace(null_mut());
                if IsWindow(child) != 0 {
                    DestroyWindow(child);
                }
                0
            }
            WM_DPICHANGED => {
                if let Ok(mut state) = (*host).state.try_borrow_mut() {
                    state.layout.update(window, (wp & 0xffff) as u32);
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
                        SendMessageW(owner, PROPERTY_CLOSED, window as usize, 0);
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
