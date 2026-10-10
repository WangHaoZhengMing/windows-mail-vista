use super::*;
use super::{manager::ManagerHost, properties::PropertyHost};
use crate::store::Store;
use std::path::PathBuf;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled, VK_TAB};

fn example_account(email: &str) -> Account {
    Account {
        auth: Default::default(),
        email: email.into(),
        username: email.into(),
        password_dpapi: crate::account::protect(b"saved-password").unwrap(),
        incoming: Incoming::Imap,
        incoming_host: "imap.example.test".into(),
        incoming_port: 993,
        incoming_security: Security::Tls,
        smtp_host: "smtp.example.test".into(),
        smtp_port: 587,
        smtp_security: Security::StartTls,
    }
}

#[test]
fn first_run_account_pages_stay_in_the_welcome_window_and_save_to_the_owner() {
    let _lock = crate::ui::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let directory = TestDirectory::new();
    let (submissions, requests) = mpsc::channel();
    let mut state = Box::new(OwnerState {
        store: Store::open(&directory.0).unwrap(),
        requests,
        reject: false,
        count: 0,
    });
    let class = wide(&format!("WinMailEmbeddedSetupTest.{}", std::process::id()));
    unsafe {
        register_class(&class, Some(owner_proc)).unwrap();
        let owner = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Setup fixture").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            900,
            700,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            (&mut *state as *mut OwnerState).cast(),
        );
        super::super::setup::open(owner, false);
        let welcome = FindWindowExW(
            null_mut(),
            null_mut(),
            wide("WinMailRustFirstRun").as_ptr(),
            null(),
        );
        assert!(!welcome.is_null());
        SendMessageW(welcome, WM_COMMAND, 810, 0);
        SendMessageW(welcome, WM_COMMAND, 810, 0);
        SetPropW(owner, wide("WinMailSetupHost").as_ptr(), welcome);
        let manager = open(owner, vec![], None, submissions.clone()).unwrap();
        RemovePropW(owner, wide("WinMailSetupHost").as_ptr());
        let mut windows = TestWindows {
            owner,
            manager,
            class,
        };
        super::super::setup::begin_account(welcome, manager);
        let wizard = property(manager);
        assert_eq!(GetParent(wizard), welcome);
        assert_eq!(GetWindowLongW(wizard, GWL_STYLE) as u32 & WS_CAPTION, 0);
        assert_eq!(
            IsWindowVisible(manager),
            0,
            "account manager must stay hidden"
        );
        assert_ne!(IsWindowVisible(welcome), 0);
        SendMessageW(wizard, WM_COMMAND, IDCANCEL as usize, 0);
        SendMessageW(welcome, WM_TIMER, 1, 0);
        assert_eq!(IsWindow(manager), 0);
        assert_ne!(IsWindowVisible(GetDlgItem(welcome, 812)), 0);
        assert_eq!(state.count, 0);

        SetPropW(owner, wide("WinMailSetupHost").as_ptr(), welcome);
        windows.manager = open(owner, vec![], None, submissions).unwrap();
        RemovePropW(owner, wide("WinMailSetupHost").as_ptr());
        super::super::setup::begin_account(welcome, windows.manager);
        let wizard = property(windows.manager);
        SendMessageW(wizard, WM_COMMAND, 760, 0);
        set_window_text(GetDlgItem(wizard, 763), "setup@icloud.com");
        SendMessageW(wizard, WM_COMMAND, 760, 0);
        let review = property(windows.manager);
        assert_eq!(GetParent(review), welcome);
        assert_eq!(GetWindowLongW(review, GWL_STYLE) as u32 & WS_CAPTION, 0);
        assert_eq!(IsWindowVisible(windows.manager), 0);
        assert_eq!(text(GetDlgItem(review, 304)), "imap.mail.me.com");
        assert_ne!(
            IsWindowEnabled(GetDlgItem(review, 325)),
            0,
            "connection test is available inline"
        );
        set_window_text(GetDlgItem(review, 303), "test-app-password");
        set_window_text(GetDlgItem(review, 305), "bad");
        SendMessageW(review, WM_COMMAND, SAVE_ACCOUNT as usize, 0);
        assert_ne!(IsWindow(review), 0);
        assert_eq!(state.count, 0);
        set_window_text(GetDlgItem(review, 305), "993");
        SendMessageW(review, WM_COMMAND, SAVE_ACCOUNT as usize, 0);
        assert_eq!(IsWindow(review), 0);
        assert_eq!(state.store.accounts().len(), 1);
        let mut message = MSG::default();
        while PeekMessageW(
            &mut message,
            welcome,
            super::super::setup::ACCOUNT_SAVED,
            super::super::setup::ACCOUNT_SAVED,
            PM_REMOVE,
        ) != 0
        {
            DispatchMessageW(&message);
        }
        SendMessageW(welcome, WM_TIMER, 1, 0);
        assert_eq!(text(GetDlgItem(welcome, 810)), "Finish");
        assert_eq!(IsWindow(windows.manager), 0);
        SendMessageW(welcome, WM_COMMAND, IDCANCEL as usize, 0);
        assert_ne!(IsWindowEnabled(owner), 0);
    }
}

fn input(record: &AccountRecord) -> FormInput {
    FormInput {
        authentication: record.account.auth.clone(),
        fields: std::array::from_fn(|index| Field::ALL[index].initial_text(Some(record))),
        incoming: record.account.incoming,
        incoming_security: record.account.incoming_security,
        smtp_security: record.account.smtp_security,
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "winmail-account-manager-{}-{nonce}",
            std::process::id()
        )))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn form_validation_preserves_password_custom_ports_and_account_name() {
    let directory = TestDirectory::new();
    let mut store = Store::open(&directory.0).unwrap();
    store
        .add_account_named("Work".into(), example_account("work@example.test"))
        .unwrap();
    let record = &store.accounts()[0];
    let mut fields = input(record);
    fields.fields[Field::Name as usize].clear();
    fields.fields[Field::IncomingPort as usize] = "1993".into();
    let (name, account) = fields.account(Some(record)).unwrap();
    assert_eq!(name, record.account.email);
    assert_eq!(account.incoming_port, 1993);
    assert_eq!(account.password_dpapi, record.account.password_dpapi);
    assert_eq!(account.password().unwrap(), "saved-password");

    fields.fields[Field::IncomingPort as usize] = "0".into();
    assert_eq!(
        fields.account(Some(record)).err().unwrap().field,
        Field::IncomingPort
    );
    fields.fields[Field::IncomingPort as usize] = "993".into();
    fields.smtp_security = Security::Plain;
    assert_eq!(
        fields.account(Some(record)).err().unwrap().field,
        Field::SmtpHost
    );
    fields.fields[Field::SmtpHost as usize] = "127.0.0.1".into();
    assert!(fields.account(Some(record)).is_ok());
    fields.fields[Field::Password as usize] = "replacement-password".into();
    assert_eq!(
        fields.account(Some(record)).unwrap().1.password().unwrap(),
        "replacement-password"
    );
}

struct OwnerState {
    store: Store,
    requests: Receiver<AccountSubmission>,
    reject: bool,
    count: usize,
}

impl OwnerState {
    fn snapshot(&self) -> AccountsSnapshot {
        AccountsSnapshot {
            accounts: self.store.accounts().to_vec(),
            default_account: self.store.default_account_id().cloned(),
        }
    }

    fn change(&mut self, change: AccountChange) -> Result<AccountsSnapshot> {
        if self.reject {
            return Err("Fixture persistence failure".into());
        }
        match change {
            AccountChange::Add { name, account } => {
                self.store.add_account_named(name, account)?;
            }
            AccountChange::Update { id, name, account } => {
                self.store.update_account_named(&id, name, account)?;
            }
            AccountChange::Remove { id } => self.store.remove_account(&id)?,
            AccountChange::SetDefault { id } => self.store.set_default_account(&id)?,
        }
        Ok(self.snapshot())
    }
}

unsafe extern "system" fn owner_proc(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if message == WM_NCCREATE {
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
            );
            return 1;
        }
        if message == ACCOUNT_SUBMITTED {
            let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut OwnerState;
            if !pointer.is_null() {
                let state = &mut *pointer;
                while let Ok(request) = state.requests.try_recv() {
                    state.count += 1;
                    let result = state.change(request.change);
                    let _ = request.reply.send(result);
                }
            }
            return 0;
        }
        if message == WM_NCDESTROY {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        }
        DefWindowProcW(window, message, wp, lp)
    }
}

struct TestWindows {
    owner: HWND,
    manager: HWND,
    class: Vec<u16>,
}

impl Drop for TestWindows {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.manager) != 0 {
                DestroyWindow(self.manager);
            }
            if IsWindow(self.owner) != 0 {
                DestroyWindow(self.owner);
            }
            UnregisterClassW(self.class.as_ptr(), GetModuleHandleW(null()));
        }
    }
}

unsafe fn property(manager: HWND) -> HWND {
    unsafe {
        let host = GetWindowLongPtrW(manager, GWLP_USERDATA) as *const ManagerHost;
        (*host).property.get()
    }
}

#[test]
fn manager_cancel_validation_owner_failure_save_default_and_dpi() {
    let _lock = crate::ui::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let directory = TestDirectory::new();
    let (submissions, requests) = mpsc::channel();
    let mut state = Box::new(OwnerState {
        store: Store::open(&directory.0).unwrap(),
        requests,
        reject: false,
        count: 0,
    });
    let first = state
        .store
        .add_account_named("Work".into(), example_account("work@example.test"))
        .unwrap();
    let second = state
        .store
        .add_account_named("Personal".into(), example_account("personal@example.test"))
        .unwrap();
    let class = wide(&format!(
        "WinMailAccountManagerTest.{}",
        directory.0.file_name().unwrap().to_string_lossy()
    ));
    unsafe {
        register_class(&class, Some(owner_proc)).unwrap();
        let owner = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Account owner fixture").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            100,
            100,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            (&mut *state as *mut OwnerState).cast(),
        );
        assert!(!owner.is_null());
        let snapshot = state.snapshot();
        let manager = open(
            owner,
            snapshot.accounts,
            snapshot.default_account,
            submissions.clone(),
        )
        .unwrap();
        let windows = TestWindows {
            owner,
            manager,
            class,
        };
        ShowWindow(manager, SW_HIDE);
        assert_eq!(IsWindowEnabled(owner), 0);
        let list = GetDlgItem(manager, ACCOUNT_LIST as i32);
        assert_eq!(SendMessageW(list, LVM_GETITEMCOUNT, 0, 0), 2);

        SendMessageW(manager, WM_COMMAND, ADD_ACCOUNT as usize, 0);
        let editor = property(manager);
        assert_ne!(IsWindow(editor), 0);
        let snapshot = state.snapshot();
        assert_eq!(
            open(
                owner,
                snapshot.accounts,
                snapshot.default_account,
                submissions.clone()
            )
            .unwrap(),
            manager,
            "A composer must reuse its owner's account manager."
        );
        assert_eq!(property(manager), editor);
        assert_eq!(IsWindowEnabled(manager), 0);
        SendMessageW(GetDlgItem(editor, 762), LB_SETCURSEL, 2, 0);
        SendMessageW(editor, WM_COMMAND, 760, 0);
        set_window_text(GetDlgItem(editor, 763), "wizard@outlook.com");
        set_window_text(GetDlgItem(editor, 764), "Wizard account");
        SendMessageW(editor, WM_COMMAND, 761, 0);
        assert_eq!(text(GetDlgItem(editor, 763)), "wizard@outlook.com");
        SendMessageW(editor, WM_COMMAND, 760, 0);
        SendMessageW(editor, WM_COMMAND, 760, 0);
        assert_eq!(IsWindow(editor), 0);
        let review = property(manager);
        assert_ne!(IsWindow(review), 0);
        assert_eq!(
            text(GetDlgItem(review, 300 + Field::Email as i32)),
            "wizard@outlook.com"
        );
        assert_eq!(
            text(GetDlgItem(review, 300 + Field::IncomingHost as i32)),
            "outlook.office365.com"
        );
        assert_eq!(
            text(GetDlgItem(review, 300 + Field::SmtpPort as i32)),
            "587"
        );
        assert_eq!(
            SendMessageW(
                GetDlgItem(review, properties::AUTHENTICATION as i32),
                CB_GETCURSEL,
                0,
                0
            ),
            1
        );
        assert_eq!(state.count, 0, "wizard review must not save an account");
        SendMessageW(review, WM_COMMAND, IDCANCEL as usize, 0);
        assert_eq!(IsWindow(editor), 0);
        assert_eq!(state.count, 0);
        assert_ne!(IsWindowEnabled(manager), 0);

        SendMessageW(manager, WM_COMMAND, EDIT_ACCOUNT as usize, 0);
        let editor = property(manager);
        let name = GetDlgItem(editor, 300);
        let port = GetDlgItem(editor, 305);
        SetFocus(name);
        let tab = MSG {
            hwnd: name,
            message: WM_KEYDOWN,
            wParam: VK_TAB as usize,
            ..std::mem::zeroed()
        };
        assert_ne!(IsDialogMessageW(editor, &tab), 0);
        assert_eq!(GetFocus(), GetDlgItem(editor, 301));
        set_window_text(name, "Renamed Work");
        set_window_text(port, "bad-port");
        SendMessageW(editor, WM_COMMAND, SAVE_ACCOUNT as usize, 0);
        assert_ne!(IsWindow(editor), 0);
        assert_eq!(state.count, 0);
        assert_eq!(GetFocus(), port);
        let property_host = GetWindowLongPtrW(editor, GWLP_USERDATA) as *const PropertyHost;
        assert!((*property_host).state.borrow().error);
        set_window_text(port, "1993");

        for value in [96u32, 144, 192, 288, 384, 96] {
            let rect = windows_sys::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: dpi::scale(820, value),
                bottom: dpi::scale(600, value),
            };
            SendMessageW(
                editor,
                WM_DPICHANGED,
                value as usize,
                &rect as *const _ as isize,
            );
            assert_eq!(text(name), "Renamed Work");
            assert_eq!(text(port), "1993");
            assert_ne!(SendMessageW(port, WM_GETFONT, 0, 0), 0);
        }

        state.reject = true;
        SendMessageW(editor, WM_COMMAND, SAVE_ACCOUNT as usize, 0);
        assert_ne!(IsWindow(editor), 0);
        assert_eq!(state.count, 1);
        assert_eq!(state.store.accounts()[0].name, "Work");
        assert_eq!(text(name), "Renamed Work");
        assert_eq!(
            text((*property_host).state.borrow().status),
            "Fixture persistence failure"
        );
        state.reject = false;
        SendMessageW(editor, WM_COMMAND, SAVE_ACCOUNT as usize, 0);
        assert_eq!(IsWindow(editor), 0);
        SendMessageW(manager, SNAPSHOT_UPDATED, 0, 0);
        assert_eq!(state.store.accounts()[0].id, first);
        assert_eq!(state.store.accounts()[0].name, "Renamed Work");
        assert_eq!(state.store.accounts()[0].account.incoming_port, 1993);
        assert_eq!(
            state.store.accounts()[0].account.password().unwrap(),
            "saved-password"
        );

        let clear = LVITEMW {
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..std::mem::zeroed()
        };
        SendMessageW(
            list,
            LVM_SETITEMSTATE,
            usize::MAX,
            &clear as *const _ as isize,
        );
        let select = LVITEMW {
            state: LVIS_SELECTED | LVIS_FOCUSED,
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..std::mem::zeroed()
        };
        SendMessageW(list, LVM_SETITEMSTATE, 1, &select as *const _ as isize);
        SendMessageW(manager, WM_COMMAND, DEFAULT_ACCOUNT as usize, 0);
        assert_eq!(state.store.default_account_id(), Some(&second));
        assert_eq!(state.count, 3);
        properties::open(manager, None);
        let editor = property(manager);
        for (field, value) in [
            (Field::Name, "Third account"),
            (Field::Email, "third@example.test"),
            (Field::Username, "third"),
            (Field::Password, "third-password"),
            (Field::IncomingHost, "imap.example.test"),
            (Field::SmtpHost, "smtp.example.test"),
        ] {
            set_window_text(GetDlgItem(editor, 300 + field as i32), value);
        }
        SendMessageW(editor, WM_COMMAND, SAVE_ACCOUNT as usize, 0);
        assert_eq!(IsWindow(editor), 0);
        SendMessageW(manager, SNAPSHOT_UPDATED, 0, 0);
        assert_eq!(state.count, 4);
        assert_eq!(SendMessageW(list, LVM_GETITEMCOUNT, 0, 0), 3);
        assert_eq!(state.store.accounts()[2].name, "Third account");
        assert_eq!(
            state.store.accounts()[2].account.password().unwrap(),
            "third-password"
        );
        assert_eq!(state.store.default_account_id(), Some(&second));
        SendMessageW(manager, WM_COMMAND, IDCANCEL as usize, 0);
        assert_eq!(IsWindow(manager), 0);
        assert_ne!(IsWindowEnabled(owner), 0);

        drop(windows);
        drop(state);
        let reopened = Store::open(&directory.0).unwrap();
        assert_eq!(reopened.default_account_id(), Some(&second));
        assert_eq!(reopened.accounts()[0].name, "Renamed Work");
        assert_eq!(reopened.accounts()[2].name, "Third account");
    }
}

#[test]
fn oauth_form_does_not_require_or_retain_passwords_and_always_requires_tls() {
    let directory = TestDirectory::new();
    let mut store = Store::open(&directory.0).unwrap();
    store
        .add_account_named("Work".into(), example_account("work@example.test"))
        .unwrap();
    let record = &store.accounts()[0];
    let mut fields = input(record);
    fields.authentication = Authentication::OAuth2(mail_core::oauth::OAuthAccount {
        config: mail_core::oauth::OAuthConfig::microsoft("registered-desktop-client"),
        tokens: None,
    });
    fields.fields[Field::Password as usize] = "ignored\r\npassword".into();
    let account = fields.account(None).unwrap().1;
    assert!(account.password_dpapi.is_empty());
    assert!(matches!(account.auth, Authentication::OAuth2(_)));
    fields.fields[Field::IncomingHost as usize] = "127.0.0.1".into();
    fields.incoming_security = Security::Plain;
    assert!(
        fields
            .account(None)
            .err()
            .unwrap()
            .message
            .contains("OAuth sign-in requires TLS")
    );
    fields.incoming_security = Security::Tls;
    fields.fields[Field::Password as usize].clear();
    fields.authentication = Authentication::Password;
    let mut previous_oauth = record.clone();
    previous_oauth.account.auth = account.auth;
    assert_eq!(
        fields.account(Some(&previous_oauth)).err().unwrap().field,
        Field::Password
    );
    fields.fields[Field::Password as usize] = "new-app-password".into();
    let switched = fields.account(Some(&previous_oauth)).unwrap().1;
    assert!(matches!(switched.auth, Authentication::Password));
    assert_eq!(switched.password().unwrap(), "new-app-password");
}
#[test]
fn oauth_settings_are_a_properties_draft_until_parent_ok_and_parent_cancel_discards_them() {
    let _lock = crate::ui::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let directory = TestDirectory::new();
    let (submissions, requests) = mpsc::channel();
    let mut state = Box::new(OwnerState {
        store: Store::open(&directory.0).unwrap(),
        requests,
        reject: false,
        count: 0,
    });
    state
        .store
        .add_account_named("Work".into(), example_account("work@example.test"))
        .unwrap();
    let class = wide(&format!(
        "WinMailOAuthPropertiesTest.{}",
        directory.0.file_name().unwrap().to_string_lossy()
    ));
    unsafe {
        register_class(&class, Some(owner_proc)).unwrap();
        let owner = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("OAuth properties fixture").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            100,
            100,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            (&mut *state as *mut OwnerState).cast(),
        );
        assert!(!owner.is_null());
        let snapshot = state.snapshot();
        let manager = open(
            owner,
            snapshot.accounts,
            snapshot.default_account,
            submissions,
        )
        .unwrap();
        let windows = TestWindows {
            owner,
            manager,
            class,
        };
        ShowWindow(manager, SW_HIDE);
        for save in [false, true] {
            SendMessageW(manager, WM_COMMAND, EDIT_ACCOUNT as usize, 0);
            let editor = property(manager);
            ShowWindow(editor, SW_HIDE);
            let combo = GetDlgItem(editor, properties::AUTHENTICATION as i32);
            SendMessageW(combo, CB_SETCURSEL, 1, 0);
            SendMessageW(
                editor,
                WM_COMMAND,
                properties::AUTHENTICATION as usize | ((CBN_SELCHANGE as usize) << 16),
                0,
            );
            assert_eq!(IsWindowVisible(GetDlgItem(editor, 303)), 0);
            SendMessageW(editor, WM_COMMAND, properties::CONFIGURE_OAUTH as usize, 0);
            let host = GetWindowLongPtrW(editor, GWLP_USERDATA) as *const PropertyHost;
            let settings = (*host).oauth_window.get();
            assert_ne!(IsWindow(settings), 0);
            ShowWindow(settings, SW_HIDE);
            assert_eq!(IsWindowEnabled(editor), 0);
            set_window_text(
                GetDlgItem(settings, oauth::CLIENT_ID as i32),
                "registered-desktop-client",
            );
            SendMessageW(settings, WM_COMMAND, IDOK as usize, 0);
            assert_eq!(IsWindow(settings), 0);
            assert_eq!(state.count, 0);
            assert_ne!(IsWindowEnabled(editor), 0);
            assert!(matches!(
                state.store.accounts()[0].account.auth,
                Authentication::Password
            ));
            assert_eq!(
                (*host)
                    .state
                    .borrow()
                    .oauth_account
                    .as_ref()
                    .unwrap()
                    .config
                    .client_id,
                "registered-desktop-client"
            );
            SendMessageW(
                editor,
                WM_COMMAND,
                if save {
                    IDOK as usize
                } else {
                    IDCANCEL as usize
                },
                0,
            );
            assert_eq!(IsWindow(editor), 0);
            if save {
                assert_eq!(state.count, 1);
                SendMessageW(manager, SNAPSHOT_UPDATED, 0, 0);
            } else {
                assert_eq!(state.count, 0);
                assert!(matches!(
                    state.store.accounts()[0].account.auth,
                    Authentication::Password
                ));
                assert_eq!(
                    state.store.accounts()[0].account.password().unwrap(),
                    "saved-password"
                );
            }
        }
        drop(windows);
    }
    drop(state);
    let reopened = Store::open(&directory.0).unwrap();
    let Authentication::OAuth2(oauth) = &reopened.accounts()[0].account.auth else {
        panic!("Parent OK must save the OAuth method");
    };
    assert_eq!(oauth.config.client_id, "registered-desktop-client");
    assert!(reopened.accounts()[0].account.password_dpapi.is_empty());
}
