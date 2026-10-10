//! Native list notifications and stable actions across independent readers.
use super::*;
use std::time::{Duration, Instant};

unsafe fn pump_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "Native command did not complete");
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn native_double_click_right_click_and_reader_actions_keep_message_identity() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let root = std::env::temp_dir().join(format!("winmail-reader-actions-{}", std::process::id()));
    let mut store = Store::open(&root).unwrap();
    let first = store
        .import(
            b"From: first@example.com\r\nSubject: First\r\n\r\nFirst body",
            "Inbox",
            None,
        )
        .unwrap();
    let second = store
        .import(
            b"From: second@example.com\r\nSubject: Second\r\n\r\nSecond body",
            "Inbox",
            None,
        )
        .unwrap();
    let host = Box::new(WindowHost::new(store));
    unsafe {
        classic::init();
        let class = wide(&format!("WinMailReaderActionsTest.{}", std::process::id()));
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
            wide("Reader actions fixture").as_ptr(),
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
        host.with_window(|window| {
            window.rebuild_folders();
            window.refresh();
        });
        let list = host.with_window(|window| window.controls.list);
        let activation = NMITEMACTIVATE {
            hdr: NMHDR {
                hwndFrom: list,
                idFrom: commands::LIST as usize,
                code: NM_DBLCLK,
            },
            iItem: 1,
            ..Default::default()
        };
        SendMessageW(
            hwnd,
            WM_NOTIFY,
            commands::LIST as usize,
            &activation as *const _ as isize,
        );
        let reader = host.with_window(|window| {
            assert_eq!(window.readers.len(), 1);
            assert_eq!(window.readers[0].id().as_str(), first);
            window.readers[0].hwnd()
        });
        SendMessageW(
            hwnd,
            WM_NOTIFY,
            commands::LIST as usize,
            &activation as *const _ as isize,
        );
        host.with_window(|window| {
            assert_eq!(
                window.readers.len(),
                1,
                "Repeated activation reuses the same message window"
            )
        });
        let selection = LVITEMW {
            state: LVIS_SELECTED | LVIS_FOCUSED,
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        SendMessageW(list, LVM_SETITEMSTATE, 0, &selection as *const _ as isize);
        host.with_window(|window| assert_eq!(window.app.selected_id().unwrap().as_str(), second));

        let mut rect = RECT {
            left: LVIR_BOUNDS as i32,
            ..Default::default()
        };
        assert_ne!(
            SendMessageW(list, LVM_GETITEMRECT, 1, &mut rect as *mut _ as isize),
            0
        );
        let subitem_activation = NMITEMACTIVATE {
            iItem: -1,
            ptAction: POINT {
                x: rect.left + 100,
                y: (rect.top + rect.bottom) / 2,
            },
            ..activation
        };
        SendMessageW(
            hwnd,
            WM_NOTIFY,
            commands::LIST as usize,
            &subitem_activation as *const _ as isize,
        );
        host.with_window(|window| assert_eq!(window.readers.len(), 1));
        let right_click = NMITEMACTIVATE {
            hdr: NMHDR {
                code: NM_RCLICK,
                ..activation.hdr
            },
            iItem: 1,
            ptAction: POINT {
                x: rect.left + 100,
                y: (rect.top + rect.bottom) / 2,
            },
            ..Default::default()
        };
        // Cancel the native popup through its owner, without injecting desktop input.
        let owner = hwnd as usize;
        let cancel = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            PostMessageW(owner as HWND, WM_CANCELMODE, 0, 0);
        });
        assert_eq!(
            SendMessageW(
                hwnd,
                WM_NOTIFY,
                commands::LIST as usize,
                &right_click as *const _ as isize
            ),
            1
        );
        cancel.join().unwrap();
        host.with_window(|window| {
            assert_eq!(
                window.app.selected_id().unwrap().as_str(),
                first,
                "Right-click acts on the clicked row"
            );
            let menu = window.message_context_menu();
            assert_eq!(GetMenuItemCount(menu), 11);
            assert_eq!(
                GetMenuItemID(menu, 0),
                command_id(Command::OpenMessage) as u32
            );
            assert_eq!(
                GetMenuState(
                    menu,
                    command_id(Command::SaveAttachments) as u32,
                    MF_BYCOMMAND
                ) & MF_DISABLED,
                0
            );
            DestroyMenu(menu);
        });
        SendMessageW(list, LVM_SETITEMSTATE, 0, &selection as *const _ as isize);
        SendMessageW(reader, WM_COMMAND, command_id(Command::Reply) as usize, 0);
        pump_until(|| host.with_window(|window| !window.composers.is_empty()));
        host.with_window(|window| {
            assert_eq!(window.composers[0].draft().to, "first@example.com");
            assert_eq!(window.app.selected_id().unwrap().as_str(), second);
        });
        SendMessageW(reader, WM_COMMAND, command_id(Command::Delete) as usize, 0);
        pump_until(|| IsWindow(reader) == 0);
        host.with_window(|window| {
            let first_id = window.readers[0].id();
            assert_eq!(window.app.message_entry(first_id).unwrap().folder, "Trash");
            assert_eq!(window.app.selected_id().unwrap().as_str(), second);
            assert_eq!(
                window
                    .app
                    .message_entry(window.app.selected_id().unwrap())
                    .unwrap()
                    .folder,
                "Inbox"
            );
            assert!(!window.app.busy());
            let first_id = first_id.clone();
            window.open_message(first_id.clone()).unwrap();
            let trash_reader = window.readers[0].hwnd();
            window.pending_reader_deletion = Some(PendingReaderDeletion {
                message: first_id.clone(),
                action: DeleteAction::PermanentDelete,
            });
            window.mail_completed(MailCompletion::Changed {
                account: None,
                action: DeleteAction::PermanentDelete,
                count: 0,
                deferred: 0,
                local_update: Err("Server rejected deletion".into()),
            });
            assert_ne!(
                IsWindow(trash_reader),
                0,
                "A failed server deletion retains the reader"
            );
            assert!(window.pending_reader_deletion.is_none());
            window.pending_reader_deletion = Some(PendingReaderDeletion {
                message: first_id.clone(),
                action: DeleteAction::PermanentDelete,
            });
            window.app.permanently_delete_message(&first_id).unwrap();
            window.mail_completed(MailCompletion::Changed {
                account: None,
                action: DeleteAction::PermanentDelete,
                count: 1,
                deferred: 0,
                local_update: Ok(()),
            });
            assert_eq!(
                IsWindow(trash_reader),
                0,
                "Confirmed completion closes the originating reader"
            );
            assert_eq!(text(window.controls.preview_subject), "Second");
            assert!(
                text(window.controls.preview_body).contains("Second body"),
                "Another selected message retains its preview across server completion"
            );
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

#[test]
fn picture_permission_is_per_message_and_preserves_an_unrelated_preview() {
    use std::io::{Read, Write};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicBool::new(false));
    let (count, stop) = (requests.clone(), stopped.clone());
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
            if let Ok((mut stream, _)) = listener.accept() {
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut buffer = [0u8; 2048];
                let _ = stream.read(&mut buffer);
                count.fetch_add(1, Ordering::Relaxed);
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    });
    let root = std::env::temp_dir().join(format!("winmail-picture-policy-{}", std::process::id()));
    let mut store = Store::open(&root).unwrap();
    for subject in ["First", "Second"] {
        let raw = format!(
            "From: sender@example.com\r\nSubject: {subject}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>{subject} body</p><img src=\"http://{address}/{subject}.png\">"
        );
        store.import(raw.as_bytes(), "Inbox", None).unwrap();
    }
    let host = Box::new(WindowHost::new(store));
    unsafe {
        classic::init();
        let class = wide(&format!("WinMailPicturePolicyTest.{}", std::process::id()));
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
            wide("Picture policy fixture").as_ptr(),
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
        host.with_window(|window| {
            window.rebuild_folders();
            window.refresh();
        });
        let list = host.with_window(|window| window.controls.list);
        let selection = LVITEMW {
            state: LVIS_SELECTED | LVIS_FOCUSED,
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        SendMessageW(list, LVM_SETITEMSTATE, 1, &selection as *const _ as isize);
        pump_until(|| {
            host.with_window(|window| window.layout.preview_format == PreviewFormat::Html)
        });
        host.with_window(|window| {
            assert!(window.layout.blocked_images);
            assert_ne!(
                GetWindowLongW(window.controls.show_pictures, GWL_STYLE) as u32 & WS_VISIBLE,
                0
            );
            assert_eq!(
                window.preview_document.as_ref().unwrap().remote_images,
                RemoteImagePolicy::Block
            );
        });
        assert_eq!(requests.load(Ordering::Relaxed), 0);
        SendMessageW(
            hwnd,
            WM_COMMAND,
            command_id(Command::ShowPictures) as usize,
            0,
        );
        pump_until(|| requests.load(Ordering::Relaxed) > 0);
        host.with_window(|window| {
            assert!(!window.layout.blocked_images);
            window.select().unwrap();
            assert_eq!(
                window.preview_document.as_ref().unwrap().remote_images,
                RemoteImagePolicy::Allow,
                "Reselecting the same message preserves its explicit permission"
            );
            window.mail_completed(MailCompletion::Changed {
                account: None,
                action: DeleteAction::Trash,
                count: 0,
                deferred: 0,
                local_update: Err("Another message could not be deleted".into()),
            });
            assert_eq!(
                window.preview_document.as_ref().unwrap().remote_images,
                RemoteImagePolicy::Allow
            );
            assert_eq!(text(window.controls.preview_subject), "First");
        });
        SendMessageW(list, LVM_SETITEMSTATE, 0, &selection as *const _ as isize);
        host.with_window(|window| {
            assert_eq!(text(window.controls.preview_subject), "Second");
            assert!(window.layout.blocked_images);
            assert_eq!(
                window.preview_document.as_ref().unwrap().remote_images,
                RemoteImagePolicy::Block,
                "Permission must not leak into another message"
            );
        });
        DestroyWindow(hwnd);
        UnregisterClassW(class.as_ptr(), instance);
        let mut message = MSG::default();
        while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {}
    }
    stopped.store(true, Ordering::Relaxed);
    server.join().unwrap();
    drop(host);
    std::fs::remove_dir_all(root).unwrap();
}
