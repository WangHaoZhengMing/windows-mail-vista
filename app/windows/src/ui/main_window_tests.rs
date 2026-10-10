use super::*;
use crate::account::{Account, Incoming, Security};

#[test]
fn account_navigation_delete_restore_and_composer_preserve_ownership() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let root = std::env::temp_dir().join(format!("winmail-accounts-ui-{}", std::process::id()));
    let mut store = Store::open(&root).unwrap();
    let account = Account {
        email: "first@example.com".into(),
        username: "first".into(),
        auth: Default::default(),
        password_dpapi: String::new(),
        incoming: Incoming::Pop3,
        incoming_host: "localhost".into(),
        incoming_port: 995,
        incoming_security: Security::Tls,
        smtp_host: "localhost".into(),
        smtp_port: 587,
        smtp_security: Security::StartTls,
    };
    let first = store
        .add_account_named("First account".into(), account.clone())
        .unwrap();
    let mut second_config = account;
    second_config.email = "second@example.com".into();
    let second = store
        .add_account_named("Second account".into(), second_config)
        .unwrap();
    let raw =
        b"From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Ownership\r\n\r\nBody";
    let first_message = store.import_for(Some(&first), raw, "Inbox", None).unwrap();
    let second_message = store.import_for(Some(&second), raw, "Inbox", None).unwrap();
    store.import_for(None, raw, "Inbox", None).unwrap();
    let host = Box::new(WindowHost::new(store));
    unsafe {
        classic::init();
        let class = wide(&format!("WinMailAccountTreeTest.{}", std::process::id()));
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        assert_ne!(RegisterClassW(&wc), 0);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Accounts fixture").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            1050,
            730,
            null_mut(),
            null_mut(),
            instance,
            (&*host as *const WindowHost).cast(),
        );
        assert!(!hwnd.is_null());
        let switch_folder = |scope: DataScope, folder: Folder| {
            let (tree, node) = host.with_window(|window| {
                let node = window
                    .controls
                    .tree_items
                    .iter()
                    .find(|node| node.scope == scope && node.folder == folder)
                    .unwrap();
                (window.controls.folder, node.handle)
            });
            SendMessageW(tree, TVM_SELECTITEM, TVGN_CARET as usize, node);
        };
        host.with_window(|window| {
            assert_eq!(window.app.scope(), &DataScope::Account(first.clone()));
            assert_eq!(
                window.app.list_messages(ReadFilter::All, "")[0].id,
                first_message
            );
        });
        switch_folder(DataScope::Account(second.clone()), Folder::Inbox);
        let list = host.with_window(|window| window.controls.list);
        let selection = LVITEMW {
            state: LVIS_SELECTED | LVIS_FOCUSED,
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        SendMessageW(list, LVM_SETITEMSTATE, 0, &selection as *const _ as isize);
        host.with_window(|window| {
            assert_eq!(window.app.selected_id().unwrap().as_str(), second_message);
            window.new_message(Draft::default());
            assert_eq!(window.composers[0].sender_id().as_ref(), Some(&second));
        });
        let key = NMLVKEYDOWN {
            hdr: NMHDR {
                hwndFrom: list,
                idFrom: command_ids::LIST as usize,
                code: LVN_KEYDOWN,
            },
            wVKey: windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_DELETE,
            flags: 0,
        };
        SendMessageW(
            hwnd,
            WM_NOTIFY,
            command_ids::LIST as usize,
            &key as *const _ as isize,
        );
        host.with_window(|window| {
            assert!(
                !window.app.busy(),
                "POP3 deletion must not contact the server"
            );
            assert!(window.app.list_messages(ReadFilter::All, "").is_empty());
        });
        switch_folder(DataScope::Account(second.clone()), Folder::Trash);
        SendMessageW(list, LVM_SETITEMSTATE, 0, &selection as *const _ as isize);
        host.with_window(|window| {
            assert_eq!(window.app.selected_id().unwrap().as_str(), second_message);
            window.action(hwnd, Command::Restore).unwrap();
            assert!(window.app.list_messages(ReadFilter::All, "").is_empty());
        });
        switch_folder(DataScope::Account(second.clone()), Folder::Inbox);
        host.with_window(|window| {
            assert_eq!(
                window.app.list_messages(ReadFilter::All, "")[0].id,
                second_message
            )
        });
        switch_folder(DataScope::Account(first.clone()), Folder::Inbox);
        host.with_window(|window| {
            assert_eq!(
                window.app.list_messages(ReadFilter::All, "")[0].id,
                first_message
            );
            assert_eq!(window.composers[0].sender_id().as_ref(), Some(&second));
            window
                .apply_account_change(AccountChange::Remove { id: second.clone() })
                .unwrap();
            assert!(window.composers[0].sender_id().is_none());
            window
                .apply_account_change(AccountChange::SetDefault { id: first.clone() })
                .unwrap();
            assert!(
                window.composers[0].sender_id().is_none(),
                "Removing a sender must not silently select another account"
            );
        });
        switch_folder(DataScope::Local, Folder::Inbox);
        host.with_window(|window| {
            assert_eq!(window.app.list_messages(ReadFilter::All, "").len(), 2);
            window.composers[0].close();
        });
        DestroyWindow(hwnd);
        UnregisterClassW(class.as_ptr(), instance);
        let mut message = MSG::default();
        while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {}
    }
    drop(host);
    std::fs::remove_dir_all(root).unwrap();
}
