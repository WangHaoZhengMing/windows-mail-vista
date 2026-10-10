use super::*;
use crate::account::{Account, Incoming, Security};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;

#[test]
fn reentrant_events_own_payloads_and_account_saves_reply_synchronously() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let root = std::env::temp_dir().join(format!("winmail-dispatch-{}", std::process::id()));
    let host = Box::new(WindowHost::new(Store::open(&root).unwrap()));
    unsafe {
        classic::init();
        let class = wide(&format!("WinMailDispatchTest.{}", std::process::id()));
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
            wide("Dispatch test").as_ptr(),
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
        let mut account = Account {
            email: "dispatch@example.com".into(),
            username: "dispatch".into(),
            auth: Default::default(),
            password_dpapi: String::new(),
            incoming: Incoming::Imap,
            incoming_host: "localhost".into(),
            incoming_port: 993,
            incoming_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::StartTls,
        };
        account.set_password("fixture secret").unwrap();
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        host.with_window(|window| {
            // Synchronous native reentry queues the command until this borrow ends.
            SendMessageW(
                hwnd,
                WM_COMMAND,
                command_id(Command::TogglePreview) as usize,
                0,
            );
            assert!(window.layout.show_preview);
            let mut words = wide("/mail");
            let packet = COPYDATASTRUCT {
                dwData: 1,
                cbData: (words.len() * 2) as u32,
                lpData: words.as_mut_ptr().cast(),
            };
            assert_eq!(
                SendMessageW(hwnd, WM_COPYDATA, 0, &packet as *const _ as isize),
                1
            );
            words.fill(b'X' as u16);
            window
                .account_tx
                .send(AccountSubmission {
                    change: AccountChange::Add {
                        name: account.email.clone(),
                        account: account.clone(),
                    },
                    reply: reply_tx,
                })
                .unwrap();
            SendMessageW(hwnd, accounts::ACCOUNT_SUBMITTED, 0, 0);
            assert!(reply_rx.try_recv().unwrap().err().unwrap().contains("busy"));
        });
        host.with_window(|window| {
            assert!(!window.layout.show_preview);
            assert_eq!(window.pending, ["/mail"]);
            assert!(
                window.app.account().is_none(),
                "Rejected saves must never execute later"
            );
        });
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        let account_tx = host.with_window(|window| window.account_tx.clone());
        account_tx
            .send(AccountSubmission {
                change: AccountChange::Add {
                    name: account.email.clone(),
                    account,
                },
                reply: reply_tx,
            })
            .unwrap();
        SendMessageW(hwnd, accounts::ACCOUNT_SUBMITTED, 0, 0);
        assert!(reply_rx.try_recv().unwrap().is_ok());
        host.with_window(|window| {
            assert_eq!(window.app.account().unwrap().email, "dispatch@example.com")
        });

        // External compose requests use the existing main window and report real UI outcomes.
        let eml = root.join("shell opened message.eml");
        std::fs::write(&eml, b"From: sender@example.com\r\nTo: dispatch@example.com\r\nSubject: Shell opened\r\n\r\nHello").unwrap();
        host.with_window(|window| {
            window.apply(&format!("\"{}\"", eml.display()));
            assert_eq!(window.readers.len(), 1);
            assert!(window.readers[0].active());
            window.readers[0].close();
        });
        for sent in [false, true] {
            let request = crate::mapi_bridge::Request::create(Draft {
                to: "external@example.com".into(),
                subject: "External draft".into(),
                ..Default::default()
            })
            .unwrap();
            host.with_window(|window| {
                window.apply(&format!("\"/mapi-request:{}\"", request.path().display()));
                assert_eq!(window.composer().unwrap().draft().subject, "External draft");
                assert_eq!(request.result(), None);
                if sent {
                    window.mail_completed(MailCompletion::Sent {
                        account: window.app.default_account_id().unwrap().clone(),
                        composer: window.active_composer.unwrap(),
                        local_update: Ok(()),
                    });
                } else {
                    window.composer().unwrap().set_sending(true);
                    assert!(window.close_composer().is_err());
                    window.composer().unwrap().set_sending(false);
                    window.close_composer().unwrap();
                }
            });
            assert_eq!(request.result(), Some(if sent { 0 } else { 1 }));
        }

        // Destruction cancels queued UI work instead of accessing dead controls.
        host.with_window(|window| {
            SendMessageW(
                hwnd,
                WM_COMMAND,
                command_id(Command::TogglePreview) as usize,
                0,
            );
            DestroyWindow(hwnd);
            assert!(!window.layout.show_preview);
        });
        host.with_window(|window| {
            assert!(!window.layout.show_preview);
            assert!(window.icons.is_empty());
        });
        assert_eq!(IsWindow(hwnd), 0);
        UnregisterClassW(class.as_ptr(), instance);
        let mut msg = MSG::default();
        // Remove the test's posted IPC and quit messages from this thread.
        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {}
    }
    drop(host);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn dpi_changes_resize_real_windows_fonts_and_images_without_losing_edits() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let root = std::env::temp_dir().join(format!(
        "winmail-dpi-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let host = Box::new(WindowHost::new(Store::open(&root).unwrap()));
    unsafe {
        classic::init();
        let class = wide(&format!("WinMailDpiTest.{}", std::process::id()));
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
            wide("DPI test").as_ptr(),
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
        let composer = compose::Window::open(
            hwnd,
            Draft {
                subject: "DPI 中文 fixture".into(),
                body: "Preserved edits".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let account_tx = host.with_window(|window| window.account_tx.clone());
        let account = accounts::open(hwnd, Vec::new(), None, account_tx).unwrap();
        SendMessageW(account, WM_COMMAND, accounts::ADD_ACCOUNT as usize, 0);
        let wizard = FindWindowW(wide("WinMailRustAccountWizard").as_ptr(), null());
        assert_eq!(GetWindow(wizard, GW_OWNER), account);
        SendMessageW(wizard, WM_COMMAND, 760, 0);
        SetWindowTextW(GetDlgItem(wizard, 763), wide("dpi@example.test").as_ptr());
        SendMessageW(wizard, WM_COMMAND, 760, 0);
        let properties = FindWindowW(wide("WinMailRustAccountProperties").as_ptr(), null());
        assert_eq!(GetWindow(properties, GW_OWNER), account);
        assert_ne!(properties, account);
        let mut before = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
        // RichEdit initializes a process font cache at each new DPI. Warm
        // all sizes once, then measure repeated transitions for leaks.
        for pass in 0..3 {
            for value in [96, 120, 144, 168, 192, 288, 384, 96] {
                for (window, width, height) in [
                    (hwnd, 1050, 730),
                    (composer.hwnd(), 800, 660),
                    (account, 800, 480),
                    (properties, 820, 600),
                ] {
                    let rect = RECT {
                        left: 0,
                        top: 0,
                        right: dpi::scale(width, value),
                        bottom: dpi::scale(height, value),
                    };
                    SendMessageW(
                        window,
                        WM_DPICHANGED,
                        (value | (value << 16)) as usize,
                        &rect as *const _ as isize,
                    );
                }
                let (search, list, folder, toolbar, icons) = host.with_window(|window| {
                    (
                        window.controls.search_host,
                        window.controls.list,
                        window.controls.folder,
                        window.controls.toolbar,
                        window.icons.clone(),
                    )
                });
                let mut rect = RECT::default();
                GetWindowRect(search, &mut rect);
                assert_eq!(rect.right - rect.left, dpi::scale(150, value));
                assert_eq!(rect.bottom - rect.top, dpi::scale(22, value));
                assert_eq!(
                    SendMessageW(list, LVM_GETCOLUMNWIDTH, 3, 0),
                    dpi::scale(140, value) as isize
                );
                for (image, set) in
                    icons
                        .iter()
                        .zip([IconSet::Toolbar, IconSet::Folders, IconSet::FolderTree])
                {
                    let (mut width, mut height) = (0, 0);
                    assert_ne!(ImageList_GetIconSize(*image, &mut width, &mut height), 0);
                    let logical_size = if set != IconSet::Folders { 20 } else { 16 };
                    assert_eq!(
                        (width, height),
                        (
                            dpi::scale(logical_size, value),
                            dpi::scale(logical_size, value)
                        )
                    );
                    assert_eq!(ImageList_GetImageCount(*image), set.count() as i32);
                }
                assert_eq!(icons.len(), 3);
                // Both controls must retain their dedicated image lists after
                // each DPI change, without retaining a stale image list.
                for (attached, owned, set) in [
                    (
                        SendMessageW(folder, TVM_GETIMAGELIST, TVSIL_NORMAL as usize, 0),
                        icons[2],
                        IconSet::FolderTree,
                    ),
                    (
                        SendMessageW(list, LVM_GETIMAGELIST, LVSIL_SMALL as usize, 0),
                        icons[1],
                        IconSet::Folders,
                    ),
                ] {
                    assert_eq!(attached, owned);
                    assert_eq!(ImageList_GetImageCount(attached), set.count() as i32);
                }
                host.with_window(|window| {
                    for node in &window.controls.tree_items {
                        let mut item = TVITEMW {
                            mask: TVIF_IMAGE | TVIF_SELECTEDIMAGE,
                            hItem: node.handle,
                            ..Default::default()
                        };
                        assert_ne!(
                            SendMessageW(folder, TVM_GETITEMW, 0, &mut item as *mut _ as isize),
                            0
                        );
                        let expected = folder_tree::tree_icon(
                            node.folder,
                            node.folder == Folder::All
                                && matches!(node.selection, FolderSelection::Local(Folder::All)),
                            &node.scope,
                        );
                        assert_eq!(item.iImage, expected as i32);
                        assert_eq!(item.iSelectedImage, expected as i32);
                    }
                });
                // Verify both actual toolbar controls use complete lists at the
                // current DPI, rather than only checking the owned handles.
                for bar in [toolbar, GetDlgItem(composer.hwnd(), classic::TOOLBAR)] {
                    let images = SendMessageW(bar, TB_GETIMAGELIST, 0, 0);
                    assert_ne!(images, 0);
                    assert_eq!(
                        ImageList_GetImageCount(images),
                        IconSet::Toolbar.count() as i32
                    );
                    let (mut width, mut height) = (0, 0);
                    assert_ne!(ImageList_GetIconSize(images, &mut width, &mut height), 0);
                    assert_eq!(
                        (width, height),
                        (dpi::scale(20, value), dpi::scale(20, value))
                    );
                }
                let font =
                    SendMessageW(GetDlgItem(composer.hwnd(), 203), WM_GETFONT, 0, 0) as HFONT;
                let mut info = LOGFONTW::default();
                assert_ne!(
                    GetObjectW(
                        font,
                        std::mem::size_of::<LOGFONTW>() as i32,
                        (&mut info as *mut LOGFONTW).cast()
                    ),
                    0
                );
                assert_eq!(info.lfHeight, -dpi::scale(12, value));
                // RichEdit owns character formats rather than a GDI font
                // handle. Point size must survive monitor DPI changes.
                {
                    use windows::Win32::UI::Controls::RichEdit::{
                        CHARFORMATW, EM_GETCHARFORMAT, SCF_SELECTION,
                    };
                    let mut format = CHARFORMATW {
                        cbSize: std::mem::size_of::<CHARFORMATW>() as u32,
                        ..Default::default()
                    };
                    SendMessageW(
                        GetDlgItem(composer.hwnd(), 204),
                        EM_GETCHARFORMAT,
                        SCF_SELECTION as usize,
                        &mut format as *mut _ as isize,
                    );
                    assert_eq!(format.yHeight, 240);
                }
                GetWindowRect(GetDlgItem(properties, 300), &mut rect);
                assert_eq!(rect.right - rect.left, dpi::scale(596, value));
                assert_eq!(composer.draft().subject, "DPI 中文 fixture");
                assert_eq!(composer.draft().body, "Preserved edits");
                let mut icon = ICONINFO::default();
                assert_ne!(
                    GetIconInfo(
                        SendMessageW(hwnd, WM_GETICON, ICON_BIG as usize, 0) as HICON,
                        &mut icon
                    ),
                    0
                );
                let mut bitmap = BITMAP::default();
                GetObjectW(
                    icon.hbmColor,
                    std::mem::size_of::<BITMAP>() as i32,
                    (&mut bitmap as *mut BITMAP).cast(),
                );
                assert_eq!(bitmap.bmWidth, dpi::scale(32, value));
                DeleteObject(icon.hbmColor);
                DeleteObject(icon.hbmMask);
            }
            if pass == 0 {
                before = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
            }
        }
        let after = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
        assert!(
            after <= before + 4,
            "GDI resources grew from {before} to {after}"
        );
        DestroyWindow(properties);
        DestroyWindow(account);
        composer.close();
        DestroyWindow(hwnd);
        UnregisterClassW(class.as_ptr(), instance);
        let mut msg = MSG::default();
        PeekMessageW(&mut msg, null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE);
    }
    drop(host);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn runtime_command_parser_preserves_quoted_url_and_file_arguments() {
    let words = super::command_words(
        r#"/mail "/mailurl:mailto:to@example.com?subject=你好 world" /eml:"C:\mail folder\你好.eml""#,
    );
    assert_eq!(
        words,
        [
            "/mail",
            "/mailurl:mailto:to@example.com?subject=你好 world",
            r"/eml:C:\mail folder\你好.eml"
        ]
    );
}
