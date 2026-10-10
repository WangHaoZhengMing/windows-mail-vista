use super::*;
use crate::account::{Account, Incoming, Security};
use mail_core::network::{
    ImapDownload, ImapFlags, ImapFolder, ImapFolderRole, ImapLocation, ImapMailboxSnapshot,
    ImapSyncReport,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const CHILD: &str = "Projects/&ZeVnLIqe-";
struct Fixture {
    host: Option<Box<WindowHost>>,
    hwnd: HWND,
    instance: windows_sys::Win32::Foundation::HMODULE,
    class: Vec<u16>,
    root: PathBuf,
    first: AccountId,
    second: AccountId,
    deferred: String,
}
impl Fixture {
    fn new(name: &str) -> Self {
        Self::with_inbox_cache(name, "INBOX")
    }
    fn with_inbox_cache(name: &str, inbox: &str) -> Self {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "winmail-server-ui-{name}-{}-{serial}",
            std::process::id()
        ));
        let mut store = Store::open(&root).unwrap();
        let configuration = Account {
            email: "first@example.test".into(),
            username: "first@example.test".into(),
            auth: Default::default(),
            password_dpapi: String::new(),
            incoming: Incoming::Imap,
            incoming_host: "imap.example.test".into(),
            incoming_port: 993,
            incoming_security: Security::Tls,
            smtp_host: "smtp.example.test".into(),
            smtp_port: 587,
            smtp_security: Security::StartTls,
        };
        let first = store
            .add_account_named("First server".into(), configuration.clone())
            .unwrap();
        let mut second_configuration = configuration.clone();
        second_configuration.email = "second@example.test".into();
        second_configuration.username = "second@example.test".into();
        let second = store
            .add_account_named("Second server".into(), second_configuration.clone())
            .unwrap();
        let mut first_report = report(&configuration, "first", false);
        for folder in &mut first_report.folders {
            if folder.role == Some(ImapFolderRole::Inbox) {
                folder.mailbox = inbox.into();
            }
        }
        for snapshot in &mut first_report.mailboxes {
            if snapshot.mailbox == "INBOX" {
                snapshot.mailbox = inbox.into();
            }
        }
        // A server can change INBOX spelling while legacy UID locations retain
        // uppercase INBOX. Only that mailbox has case-insensitive identity.
        store.apply_imap_sync(&first, first_report).unwrap();
        store
            .apply_imap_sync(&second, report(&second_configuration, "second", true))
            .unwrap();
        for (owner, prefix) in [(&first, "first"), (&second, "second")] {
            store
                .import_for(
                    Some(owner),
                    &raw(&format!("{prefix}-local-sent")),
                    "Sent",
                    None,
                )
                .unwrap();
            store
                .save_draft_for(
                    Some(owner),
                    &configuration.email,
                    &Draft {
                        subject: format!("{prefix}-local-draft"),
                        body: "Unsent local draft".into(),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        let entry = store
            .entries()
            .iter()
            .find(|entry| {
                entry.account_id.as_ref() == Some(&first)
                    && entry
                        .remote
                        .as_ref()
                        .is_some_and(|remote| remote.mailbox == "Trash")
            })
            .unwrap();
        let deferred = entry.id.clone();
        let source = entry.remote.clone().unwrap();
        store.defer_permanent_delete(&deferred, source).unwrap();
        // Reopen the durable folder cache, matching startup after synchronization.
        drop(store);
        let store = Store::open(&root).unwrap();
        let host = Box::new(WindowHost::new(store));
        let class = wide(&format!(
            "WinMailServerFolders.{name}.{}.{serial}",
            std::process::id()
        ));
        unsafe {
            classic::init();
            let instance = GetModuleHandleW(null());
            let registration = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&registration), 0);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("Server folder fixture").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                1050,
                730,
                null_mut(),
                menus::menu(false),
                instance,
                (&*host as *const WindowHost).cast(),
            );
            assert!(!hwnd.is_null());
            // run() populates the message list after native control creation.
            // Selecting the already active Inbox does not emit a tree event.
            host.with_window(|window| window.refresh());
            Self {
                host: Some(host),
                hwnd,
                instance,
                class,
                root,
                first,
                second,
                deferred,
            }
        }
    }
    fn host(&self) -> &WindowHost {
        self.host.as_ref().unwrap()
    }
    fn node(&self, owner: &AccountId, mailbox: &str) -> (HWND, HTREEITEM, usize) {
        self.host().with_window(|window| {
            let (index, node) = window
                .controls
                .tree_items
                .iter()
                .enumerate()
                .find(|(_, node)| {
                    node.scope == DataScope::Account(owner.clone())
                        && node.selection == FolderSelection::ServerMailbox(mailbox.into())
                })
                .unwrap();
            (window.controls.folder, node.handle, index)
        })
    }
    fn select_mailbox(&self, owner: &AccountId, mailbox: &str) {
        let (tree, node, _) = self.node(owner, mailbox);
        unsafe {
            assert_ne!(
                SendMessageW(tree, TVM_SELECTITEM, TVGN_CARET as usize, node),
                0
            );
        }
    }
    fn notify_mailbox(&self, owner: &AccountId, mailbox: &str) {
        let (tree, node, index) = self.node(owner, mailbox);
        let notification = NMTREEVIEWW {
            hdr: NMHDR {
                hwndFrom: tree,
                idFrom: command_ids::FOLDER as usize,
                code: TVN_SELCHANGEDW,
            },
            itemNew: TVITEMW {
                hItem: node,
                lParam: index as isize,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                self.hwnd,
                WM_NOTIFY,
                command_ids::FOLDER as usize,
                &notification as *const _ as isize,
            );
        }
    }
    fn select_first_message(&self) {
        let list = self.host().with_window(|window| window.controls.list);
        let item = LVITEMW {
            state: LVIS_SELECTED | LVIS_FOCUSED,
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        unsafe {
            assert_ne!(
                SendMessageW(list, LVM_SETITEMSTATE, 0, &item as *const _ as isize),
                0
            );
        }
    }
    fn command(&self, command: Command) {
        unsafe {
            SendMessageW(self.hwnd, WM_COMMAND, command_id(command) as usize, 0);
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
            UnregisterClassW(self.class.as_ptr(), self.instance);
            let mut message = MSG::default();
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {}
        }
        drop(self.host.take());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn raw(subject: &str) -> Vec<u8> {
    format!("From: sender@example.test\r\nTo: recipient@example.test\r\nSubject: {subject}\r\n\r\nCached plain text body").into_bytes()
}
fn report(account: &Account, prefix: &str, second_only: bool) -> ImapSyncReport {
    let mut folders = vec![
        folder("INBOX", "Inbox", true, Some(ImapFolderRole::Inbox)),
        folder("Projects", "Projects", false, None),
        folder(CHILD, "Projects/日本語", true, None),
        folder("INBOX/Sub", "Inbox/Sub", true, None),
        folder("Archive", "Archive", true, Some(ImapFolderRole::Archive)),
        folder("Trash", "Deleted Items", true, Some(ImapFolderRole::Trash)),
        folder("Sent", "Sent Items", true, Some(ImapFolderRole::Sent)),
        folder("Drafts", "Drafts", true, Some(ImapFolderRole::Drafts)),
    ];
    if second_only {
        folders.push(folder("Second-only", "Second-only", true, None));
    }
    let mut report = ImapSyncReport {
        folders: folders.clone(),
        ..Default::default()
    };
    for (index, folder) in folders
        .into_iter()
        .filter(|folder| folder.selectable)
        .enumerate()
    {
        let uid = index as u32 + 1;
        let location = ImapLocation {
            connection: account.id(),
            mailbox: folder.mailbox.clone(),
            uid_validity: 100,
            uid,
        };
        let flags = ImapFlags::default();
        report.mailboxes.push(ImapMailboxSnapshot {
            mailbox: folder.mailbox,
            uid_validity: 100,
            uid_next: Some(uid + 1),
            messages: BTreeMap::from([(uid, flags.clone())]),
        });
        report.downloads.push(ImapDownload {
            location,
            raw: raw(&format!("{prefix}-server-{index}")),
            flags,
        });
    }
    report
}
fn folder(
    mailbox: &str,
    caption: &str,
    selectable: bool,
    role: Option<ImapFolderRole>,
) -> ImapFolder {
    ImapFolder {
        mailbox: mailbox.into(),
        display_name: caption.into(),
        delimiter: Some('/'),
        selectable,
        role,
    }
}
unsafe fn tree_caption(tree: HWND, node: HTREEITEM) -> String {
    unsafe {
        let mut buffer = [0u16; 128];
        let mut item = TVITEMW {
            mask: TVIF_TEXT,
            hItem: node,
            pszText: buffer.as_mut_ptr(),
            cchTextMax: buffer.len() as i32,
            ..Default::default()
        };
        assert_ne!(
            SendMessageW(tree, TVM_GETITEMW, 0, &mut item as *mut _ as isize),
            0
        );
        String::from_utf16_lossy(&buffer[..buffer.iter().position(|&value| value == 0).unwrap()])
    }
}
unsafe fn menu_caption(menu: HMENU, index: i32) -> String {
    unsafe {
        let mut buffer = [0u16; 128];
        let length = GetMenuStringW(
            menu,
            index as u32,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
            MF_BYPOSITION,
        );
        String::from_utf16_lossy(&buffer[..length as usize])
    }
}
unsafe fn status_part(status: HWND, part: usize) -> String {
    unsafe {
        let mut buffer = [0u16; 512];
        let length =
            SendMessageW(status, SB_GETTEXTW, part, buffer.as_mut_ptr() as isize) as u32 & 0xffff;
        String::from_utf16_lossy(&buffer[..length as usize])
    }
}

#[test]
fn cached_server_tree_uses_native_hierarchy_unicode_and_exact_account_mailboxes() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let fixture = Fixture::new("hierarchy");
    fixture.host().with_window(|window| {
        assert_eq!(
            window.app.scope(),
            &DataScope::Account(fixture.first.clone())
        );
        assert_eq!(
            window.app.folder_selection(),
            &FolderSelection::ServerMailbox("INBOX".into())
        );
    });
    let (tree, parent, _) = fixture.node(&fixture.first, "Projects");
    let (_, child, _) = fixture.node(&fixture.first, CHILD);
    unsafe {
        assert_eq!(
            SendMessageW(tree, TVM_GETNEXTITEM, TVGN_PARENT as usize, child),
            parent
        );
        assert_eq!(tree_caption(tree, child), "日本語");
    }
    fixture.select_mailbox(&fixture.first, CHILD);
    fixture.host().with_window(|window| {
        assert_eq!(
            window.app.folder_selection(),
            &FolderSelection::ServerMailbox(CHILD.into())
        );
        let rows = window.app.list_messages(ReadFilter::All, "");
        assert_eq!(rows.len(), 1);
        assert!(
            rows.iter()
                .all(|entry| entry.account_id.as_ref() == Some(&fixture.first)
                    && entry.remote.as_ref().unwrap().mailbox == CHILD)
        );
    });
    fixture.notify_mailbox(&fixture.second, CHILD);
    fixture.host().with_window(|window| {
        assert_eq!(
            window.app.scope(),
            &DataScope::Account(fixture.second.clone())
        );
        let rows = window.app.list_messages(ReadFilter::All, "");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].summary.subject.starts_with("second-"));
    });
    for mailbox in ["Archive", "INBOX/Sub"] {
        fixture.select_mailbox(&fixture.first, mailbox);
        fixture.host().with_window(|window| {
            let rows = window.app.list_messages(ReadFilter::All, "");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].remote.as_ref().unwrap().mailbox, mailbox);
        });
    }
    fixture.select_mailbox(&fixture.first, "Projects");
    fixture.host().with_window(|window| {
        assert!(window.app.list_messages(ReadFilter::All, "").is_empty());
        assert!(!window.app.busy());
    });
    for mailbox in ["Sent", "Drafts"] {
        fixture.select_mailbox(&fixture.first, mailbox);
        fixture.host().with_window(|window| {
            let rows = window.app.list_messages(ReadFilter::All, "");
            assert_eq!(
                rows.len(),
                2,
                "Server special-use folder includes its matching local pending messages"
            );
            assert!(
                rows.iter()
                    .all(|entry| entry.account_id.as_ref() == Some(&fixture.first))
            );
        });
    }
}

#[test]
fn native_message_menu_excludes_nonselectable_source_and_foreign_mailboxes_and_flags_stay_unread() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let fixture = Fixture::new("commands");
    fixture.select_mailbox(&fixture.first, CHILD);
    fixture.select_first_message();
    fixture.host().with_window(|window| unsafe {
        let destinations = window.move_destinations();
        assert!(destinations.iter().any(|(mailbox, _)| mailbox == "Archive"));
        assert!(destinations.iter().all(|(mailbox, _)| mailbox != "Projects"
            && mailbox != CHILD
            && mailbox != "Second-only"));
        let popup = window.message_context_menu();
        let index = (0..GetMenuItemCount(popup))
            .find(|&index| menu_caption(popup, index) == "Move to Folder")
            .unwrap();
        let submenu = GetSubMenu(popup, index);
        assert!(!submenu.is_null());
        assert_eq!(GetMenuItemCount(submenu) as usize, destinations.len());
        for (index, (_, caption)) in destinations.iter().enumerate() {
            assert_eq!(menu_caption(submenu, index as i32), *caption);
            assert_eq!(GetMenuItemID(submenu, index as i32), 6000 + index as u32);
        }
        DestroyMenu(popup);
    });
    fixture.command(Command::MarkUnread);
    fixture.command(Command::ToggleFlag);
    fixture.command(Command::Refresh);
    fixture.host().with_window(|window| unsafe {
        let id = window.app.selected_id().unwrap();
        let entry = window.app.message_entry(id).unwrap();
        assert!(
            !entry.read && !entry.flags.seen,
            "Refreshing list selection must not read the message again"
        );
        assert!(entry.flags.flagged);
        assert!(!window.app.busy());
        let mut item = LVITEMW {
            mask: LVIF_IMAGE,
            iItem: 0,
            ..Default::default()
        };
        assert_ne!(
            SendMessageW(
                window.controls.list,
                LVM_GETITEMW,
                0,
                &mut item as *mut _ as isize
            ),
            0
        );
        assert_eq!(item.iImage, FolderIcon::UnreadMessage as i32);
        let mut buffer = [0u16; 16];
        let mut item = LVITEMW {
            iSubItem: 2,
            pszText: buffer.as_mut_ptr(),
            cchTextMax: buffer.len() as i32,
            ..Default::default()
        };
        let count = SendMessageW(
            window.controls.list,
            LVM_GETITEMTEXTW,
            0,
            &mut item as *mut _ as isize,
        );
        assert_eq!(String::from_utf16_lossy(&buffer[..count as usize]), "⚑");
    });
}

#[test]
fn pending_server_deletion_keeps_cached_message_restore_and_honest_native_status() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let fixture = Fixture::new("pending");
    fixture.select_mailbox(&fixture.first, "Trash");
    fixture.select_first_message();
    fixture.host().with_window(|window| unsafe {
        assert_eq!(window.app.selected_id().unwrap().as_str(), fixture.deferred);
        assert!(
            window
                .app
                .message_entry(window.app.selected_id().unwrap())
                .unwrap()
                .pending_delete
        );
        let status = status_part(window.controls.status, 2);
        assert!(status.contains("awaiting server cleanup"));
        assert!(status.contains("Restore"));
        let menu = window.menu();
        let state = GetMenuState(menu, command_id(Command::Restore) as u32, MF_BYCOMMAND);
        assert_ne!(state, u32::MAX);
        assert_eq!(state & MF_GRAYED, 0);
        assert_ne!(
            GetMenuState(
                menu,
                command_id(Command::DeletePermanently) as u32,
                MF_BYCOMMAND
            ) & MF_GRAYED,
            0
        );
        assert_eq!(
            SendMessageW(
                window.controls.toolbar,
                TB_ISBUTTONENABLED,
                command_id(Command::Delete) as usize,
                0
            ),
            0
        );
        let context = window.message_context_menu();
        assert_eq!(
            GetMenuState(context, command_id(Command::Restore) as u32, MF_BYCOMMAND) & MF_GRAYED,
            0
        );
        assert_ne!(
            GetMenuState(context, command_id(Command::Delete) as u32, MF_BYCOMMAND) & MF_GRAYED,
            0
        );
        DestroyMenu(context);
        window.mail_completed(MailCompletion::Changed {
            account: Some(fixture.first.clone()),
            action: DeleteAction::PermanentDelete,
            count: 1,
            deferred: 1,
            local_update: Ok(()),
        });
        let status = status_part(window.controls.status, 2);
        assert!(status.contains("awaiting server cleanup"));
        assert!(!status.contains("permanently deleted"));
        assert!(
            window
                .app
                .message_entry(window.app.selected_id().unwrap())
                .unwrap()
                .pending_delete
        );
    });
}

#[test]
fn cached_inbox_spelling_keeps_legacy_uid_messages_and_excludes_the_same_move_target() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let fixture = Fixture::with_inbox_cache("inbox-case", "inbox");
    fixture.select_mailbox(&fixture.first, "inbox");
    fixture.host().with_window(|window| {
        assert_eq!(
            window.app.folder_selection(),
            &FolderSelection::ServerMailbox("inbox".into())
        );
        let rows = window.app.list_messages(ReadFilter::All, "");
        assert_eq!(
            rows.len(),
            1,
            "INBOX wire spelling must not hide an existing UID cache"
        );
        assert_eq!(rows[0].remote.as_ref().unwrap().mailbox, "INBOX");
    });
    fixture.select_first_message();
    fixture.host().with_window(|window| {
        let targets = window.move_destinations();
        assert!(targets.iter().any(|(mailbox, _)| mailbox == "Archive"));
        assert!(
            targets
                .iter()
                .all(|(mailbox, _)| !mailbox.eq_ignore_ascii_case("INBOX")),
            "Moving into the same INBOX must not be offered when its spelling changes"
        );
    });
}
