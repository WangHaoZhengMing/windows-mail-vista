use super::{
    AppState, ComposerId, DataScope, DeleteAction, Folder, MailCompletion, ReadFilter, ReplyKind,
    Selection, SortColumn, SortDirection, SortSpec, mail::WorkerResult,
};
use mail_core::{
    account::{Account, Incoming, Security},
    message::{self, Draft},
    network::Download,
    store::Store,
};
use std::{path::PathBuf, sync::mpsc};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("winmail-app-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn store(&self) -> Store {
        Store::open(&self.0.join("store")).unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn raw(from: &str, subject: &str, date: &str) -> Vec<u8> {
    format!("From: {from}\r\nTo: to@example.com\r\nSubject: {subject}\r\nDate: {date}\r\n\r\nbody")
        .into_bytes()
}

fn account(email: &str) -> Account {
    Account {
        email: email.into(),
        username: email.into(),
        auth: Default::default(),
        password_dpapi: String::new(),
        incoming: Incoming::Pop3,
        incoming_host: "127.0.0.1".into(),
        incoming_port: 110,
        incoming_security: Security::Plain,
        smtp_host: "127.0.0.1".into(),
        smtp_port: 25,
        smtp_security: Security::Plain,
    }
}

#[test]
fn filtering_and_selection_use_visible_message_identity() {
    let temp = Temp::new();
    let mut store = temp.store();
    let first = store
        .import(&raw("ALICE@example.com", "Alpha", ""), "Inbox", None)
        .unwrap();
    let second = store
        .import(&raw("bob@example.com", "Beta", ""), "Inbox", None)
        .unwrap();
    store.mark_read(&second).unwrap();
    store
        .import(&raw("other@example.com", "Elsewhere", ""), "Trash", None)
        .unwrap();
    let mut app = AppState::new(store);

    app.set_folder(Folder::Inbox);
    let entries = app.list_messages(ReadFilter::Unread, "ALIce");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, first);
    let Selection::Message(selected) = app.select_message(0).unwrap() else {
        panic!("Expected a regular message");
    };
    assert_eq!(selected.summary.subject, "Alpha");
    assert_eq!(app.selected_id().unwrap().as_str(), first);
    assert!(app.list_messages(ReadFilter::Unread, "").is_empty());
    assert_eq!(app.list_messages(ReadFilter::Read, "BETA")[0].id, second);

    app.set_folder(Folder::Trash);
    assert!(app.selected_id().is_none());
    assert!(
        app.select_message(0).is_err(),
        "Changing folder invalidates old row identities"
    );
    assert_eq!(
        app.list_messages(ReadFilter::All, "")[0].summary.subject,
        "Elsewhere"
    );
}

#[test]
fn sorting_preserves_column_clicks_and_undated_import_order() {
    let temp = Temp::new();
    let mut store = temp.store();
    store
        .import(
            &raw(
                "alice@example.com",
                "Zebra",
                "Mon, 1 Jan 2024 10:00:00 +0000",
            ),
            "Inbox",
            None,
        )
        .unwrap();
    store
        .import(
            &raw("bob@example.com", "Apple", "Tue, 2 Jan 2024 10:00:00 +0000"),
            "Inbox",
            None,
        )
        .unwrap();
    let mut app = AppState::new(store);
    assert_eq!(
        app.sort(),
        SortSpec {
            column: SortColumn::Received,
            direction: SortDirection::Descending
        }
    );
    assert_eq!(
        app.list_messages(ReadFilter::All, "")[0].summary.subject,
        "Apple"
    );
    app.sort_by(SortColumn::Received);
    assert_eq!(app.sort().direction, SortDirection::Ascending);
    assert_eq!(
        app.list_messages(ReadFilter::All, "")[0].summary.subject,
        "Zebra"
    );
    app.sort_by(SortColumn::Subject);
    assert_eq!(app.sort().direction, SortDirection::Ascending);
    assert_eq!(
        app.list_messages(ReadFilter::All, "")[0].summary.subject,
        "Apple"
    );
    app.sort_by(SortColumn::Subject);
    assert_eq!(
        app.list_messages(ReadFilter::All, "")[0].summary.subject,
        "Zebra"
    );
    app.sort_by(SortColumn::Priority);
    assert_eq!(
        app.list_messages(ReadFilter::All, "")[0].summary.subject,
        "Zebra"
    );

    // A separate undated catalog verifies the import-order fallback independently
    // of date ordering and subject ordering.
    let mut store = Store::open(&temp.0.join("undated-store")).unwrap();
    store
        .import(&raw("from@example.com", "First", ""), "Inbox", None)
        .unwrap();
    store
        .import(&raw("from@example.com", "Second", ""), "Inbox", None)
        .unwrap();
    let mut undated_app = AppState::new(store);
    assert_eq!(
        undated_app.list_messages(ReadFilter::All, "")[0]
            .summary
            .subject,
        "Second"
    );
    undated_app.sort_by(SortColumn::Received);
    assert_eq!(
        undated_app.list_messages(ReadFilter::All, "")[0]
            .summary
            .subject,
        "Second"
    );
}

#[test]
fn draft_selection_restores_private_fields_without_marking_read() {
    let temp = Temp::new();
    let path = temp.0.join("original.bin");
    std::fs::write(&path, b"draft attachment").unwrap();
    let draft = Draft {
        bcc: "hidden@example.com".into(),
        attachments: vec![path.clone()],
        ..Draft::default()
    };
    let mut app = AppState::new(temp.store());
    let id = app.save_draft(&draft).unwrap();
    std::fs::remove_file(path).unwrap();
    app.set_folder(Folder::Drafts);
    app.list_messages(ReadFilter::All, "");
    let Selection::Draft(restored) = app.select_message(0).unwrap() else {
        panic!("Expected a saved draft");
    };
    assert_eq!(restored.bcc, draft.bcc);
    assert_eq!(
        std::fs::read(&restored.attachments[0]).unwrap(),
        b"draft attachment"
    );
    assert_eq!(app.selected_id().unwrap(), &id);
    assert_eq!(
        app.selected_message().unwrap().summary.from,
        "draft@winmail-rust.local"
    );
    assert_eq!(app.list_messages(ReadFilter::Unread, "").len(), 1);
}

#[test]
fn reply_and_forward_preserve_recipients_body_and_attachment_data() {
    let temp = Temp::new();
    let path = temp.0.join("original.bin");
    std::fs::write(&path, [0, 255, 42]).unwrap();
    let draft = Draft {
        to: "to@example.com".into(),
        cc: "cc@example.com".into(),
        subject: "Original subject".into(),
        body: "first\nsecond".into(),
        attachments: vec![path.clone()],
        ..Draft::default()
    };
    let mut bytes = b"Reply-To: reply@example.com\r\n".to_vec();
    bytes.extend(draft.build("from@example.com").unwrap());
    let mut store = temp.store();
    store.import(&bytes, "Inbox", None).unwrap();
    std::fs::remove_file(path).unwrap();
    let mut app = AppState::new(store);
    app.list_messages(ReadFilter::All, "");
    app.select_message(0).unwrap();

    let reply = app.reply(ReplyKind::Sender).unwrap();
    assert_eq!(reply.to, "reply@example.com");
    assert_eq!(reply.subject, "Re: Original subject");
    assert!(reply.body.contains("> first\r\n> second"));
    assert!(reply.cc.is_empty());
    let reply_all = app.reply(ReplyKind::All).unwrap();
    assert_eq!(reply_all.cc, "to@example.com,cc@example.com");
    let forward = app.reply(ReplyKind::Forward).unwrap();
    assert_eq!(forward.subject, "Fwd: Original subject");
    assert!(forward.to.is_empty());
    assert!(forward.body.contains("From: from@example.com"));
    assert_eq!(
        std::fs::read(&forward.attachments[0]).unwrap(),
        [0, 255, 42]
    );
    let forwarded = message::parse(&forward.build_draft("from@example.com").unwrap()).unwrap();
    assert_eq!(forwarded.attachments[0].bytes, [0, 255, 42]);
}

#[test]
fn application_import_export_and_trash_keep_original_bytes() {
    let temp = Temp::new();
    let source = temp.0.join("source.eml");
    let bytes = raw("from@example.com", "Keep original bytes", "");
    std::fs::write(&source, &bytes).unwrap();
    let mut app = AppState::new(temp.store());
    assert_eq!(app.import_path(&source).unwrap(), 1);
    app.list_messages(ReadFilter::All, "");
    app.select_message(0).unwrap();
    let exported = temp.0.join("exported.eml");
    app.export_selected(&exported).unwrap();
    assert_eq!(std::fs::read(exported).unwrap(), bytes);
    app.delete_selected().unwrap();
    app.set_folder(Folder::Inbox);
    assert!(app.list_messages(ReadFilter::All, "").is_empty());
    app.set_folder(Folder::Trash);
    assert_eq!(app.list_messages(ReadFilter::All, "").len(), 1);
}

#[test]
fn worker_completion_updates_store_and_reports_the_original_composer() {
    let temp = Temp::new();
    let mut app = AppState::new(temp.store());
    let account = app.add_account(account("account@example.com")).unwrap();
    let (sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    assert!(app.busy());
    assert!(app.poll().is_none());
    sender
        .send(WorkerResult::Inbox {
            account: account.clone(),
            result: Ok(vec![Download {
                key: "remote:1".into(),
                raw: raw("from@example.com", "Downloaded", ""),
            }]),
        })
        .unwrap();
    assert!(matches!(
        app.poll(),
        Some(MailCompletion::Received { count: 1, .. })
    ));
    assert!(!app.busy());
    assert_eq!(app.store.known_remote(), ["remote:1"]);

    let id = app
        .store
        .import(&raw("from@example.com", "Accepted", ""), "Outbox", None)
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    sender
        .send(WorkerResult::Sent {
            account: account.clone(),
            id: super::MessageId::from_stored(id.clone()),
            composer: ComposerId::new(3),
            result: Ok(()),
        })
        .unwrap();
    let Some(MailCompletion::Sent {
        composer,
        local_update,
        ..
    }) = app.poll()
    else {
        panic!("Expected send completion");
    };
    assert_eq!(composer.index(), 3);
    local_update.unwrap();
    assert!(
        app.store
            .entries()
            .iter()
            .any(|entry| entry.id == id && entry.folder == "Sent")
    );

    let failed = app
        .store
        .import(&raw("from@example.com", "Retry later", ""), "Outbox", None)
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    sender
        .send(WorkerResult::Sent {
            account,
            id: super::MessageId::from_stored(failed.clone()),
            composer: ComposerId::new(4),
            result: Err("SMTP rejected".into()),
        })
        .unwrap();
    assert!(
        matches!(app.poll(), Some(MailCompletion::Failed(error)) if error == "Operation failed: SMTP rejected")
    );
    assert!(
        app.store
            .entries()
            .iter()
            .any(|entry| entry.id == failed && entry.folder == "Outbox")
    );

    let (sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    drop(sender);
    assert!(matches!(app.poll(), Some(MailCompletion::WorkerStopped)));
    assert!(!app.busy());
}

#[test]
fn scopes_and_composer_account_survive_selection_changes() {
    let temp = Temp::new();
    let mut app = AppState::new(temp.store());
    let first = app
        .add_account_named("Personal".into(), account("first@example.com"))
        .unwrap();
    let second = app
        .add_account_named("Work".into(), account("second@example.com"))
        .unwrap();
    let local = app
        .store
        .import(&raw("local@example.com", "Imported", ""), "Inbox", None)
        .unwrap();
    let one = app
        .store
        .import_for(
            Some(&first),
            &raw("from@example.com", "First", ""),
            "Inbox",
            None,
        )
        .unwrap();
    app.store
        .import_for(
            Some(&second),
            &raw("from@example.com", "Second", ""),
            "Inbox",
            None,
        )
        .unwrap();
    app.set_scope(DataScope::Account(first.clone())).unwrap();
    app.set_folder(Folder::Inbox);
    assert_eq!(app.list_messages(ReadFilter::All, "").len(), 1);
    app.select_message(0).unwrap();
    assert_eq!(app.selected_account_id(), Some(&first));
    assert_eq!(app.selected_id().unwrap().as_str(), one);
    // The composer retains first even when the main window changes to second.
    app.set_scope(DataScope::Account(second.clone())).unwrap();
    assert_eq!(app.active_account_id(), Some(&second));
    let draft = Draft {
        bcc: "private@example.com".into(),
        ..Draft::default()
    };
    let draft_id = app.save_draft_for(Some(&first), &draft).unwrap();
    assert_eq!(
        app.store
            .entry(draft_id.as_str())
            .unwrap()
            .account_id
            .as_ref(),
        Some(&first)
    );
    assert_eq!(
        app.store.entry(draft_id.as_str()).unwrap().summary.from,
        "first@example.com"
    );
    app.set_scope(DataScope::Local).unwrap();
    assert_eq!(app.list_messages(ReadFilter::All, "")[0].id, local);
    app.select_message(0).unwrap();
    assert!(app.selected_account_id().is_none());
    assert!(app.has_local_messages());
    app.remove_account(&first).unwrap();
    assert!(app.store.entry(&one).unwrap().account_id.is_none());
    assert_eq!(
        app.store.draft(draft_id.as_str()).unwrap().bcc,
        "private@example.com"
    );
}

#[test]
fn pop3_trash_restore_and_empty_trash_are_local_and_scoped() {
    let temp = Temp::new();
    let mut app = AppState::new(temp.store());
    let first = app.add_account(account("first@example.com")).unwrap();
    let second = app.add_account(account("second@example.com")).unwrap();
    let one = app
        .store
        .import_for(
            Some(&first),
            &raw("from@example.com", "First", ""),
            "Inbox",
            Some("pop3:first".into()),
        )
        .unwrap();
    let two = app
        .store
        .import_for(
            Some(&second),
            &raw("from@example.com", "Second", ""),
            "Inbox",
            Some("pop3:second".into()),
        )
        .unwrap();
    app.set_scope(DataScope::Account(first)).unwrap();
    app.set_folder(Folder::Inbox);
    app.list_messages(ReadFilter::All, "");
    app.select_message(0).unwrap();
    app.delete_selected().unwrap();
    assert!(!app.busy());
    app.set_folder(Folder::Trash);
    app.list_messages(ReadFilter::All, "");
    app.select_message(0).unwrap();
    app.restore_selected().unwrap();
    assert_eq!(app.store.entry(&one).unwrap().folder, "Inbox");
    app.set_folder(Folder::Inbox);
    app.list_messages(ReadFilter::All, "");
    app.select_message(0).unwrap();
    app.delete_selected().unwrap();
    app.store.trash(&two, None).unwrap();
    app.empty_trash().unwrap();
    assert!(app.store.entry(&one).is_none());
    assert_eq!(app.store.entry(&two).unwrap().folder, "Trash");
}

#[test]
fn failed_remote_change_leaves_source_and_busy_account_mutation_is_rejected() {
    let temp = Temp::new();
    let mut app = AppState::new(temp.store());
    let owner = app.add_account(account("first@example.com")).unwrap();
    let id = app
        .store
        .import_for(
            Some(&owner),
            &raw("from@example.com", "Keep me", ""),
            "Inbox",
            None,
        )
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    assert!(app.remove_account(&owner).is_err());
    assert!(
        app.update_account_named(&owner, "Changed".into(), account("changed@example.com"))
            .is_err()
    );
    sender
        .send(WorkerResult::Changed {
            account: Some(owner.clone()),
            action: DeleteAction::Trash,
            completed: Vec::new(),
            uncertain: Vec::new(),
            deferred: Vec::new(),
            error: Some("IMAP MOVE rejected".into()),
        })
        .unwrap();
    let Some(MailCompletion::Changed {
        count,
        local_update,
        account,
        ..
    }) = app.poll()
    else {
        panic!("Expected remote change completion");
    };
    assert_eq!(account, Some(owner));
    assert_eq!(count, 0);
    assert!(local_update.unwrap_err().contains("IMAP MOVE rejected"));
    assert_eq!(app.store.entry(&id).unwrap().folder, "Inbox");
}

#[test]
fn filtering_out_selection_cannot_delete_a_hidden_message() {
    let temp = Temp::new();
    let mut store = temp.store();
    let id = store
        .import(&raw("from@example.com", "Selected", ""), "Inbox", None)
        .unwrap();
    store
        .import(&raw("from@example.com", "Other", ""), "Inbox", None)
        .unwrap();
    let mut app = AppState::new(store);
    app.list_messages(ReadFilter::All, "Selected");
    app.select_message(0).unwrap();
    app.sort_by(SortColumn::Subject);
    app.list_messages(ReadFilter::All, "Selected");
    assert_eq!(
        app.selected_id().unwrap().as_str(),
        id,
        "Sorting retains identity"
    );
    assert_eq!(app.list_messages(ReadFilter::All, "Other").len(), 1);
    assert!(app.selected_id().is_none());
    assert!(app.delete_selected().is_err());
    assert_eq!(app.store.entry(&id).unwrap().folder, "Inbox");
}

#[test]
fn uncertain_move_preserves_body_and_blocks_stale_uid_after_reopen() {
    let temp = Temp::new();
    let mut store = temp.store();
    let mut config = account("first@example.com");
    config.incoming = Incoming::Imap;
    let owner = store.add_account(config.clone()).unwrap();
    let bytes = raw("from@example.com", "Uncertain move", "");
    let id = store
        .import_for(
            Some(&owner),
            &bytes,
            "Inbox",
            Some(format!("{}:imap:1:7", config.id())),
        )
        .unwrap();
    let mut app = AppState::new(store);
    let (sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    let reason = "Server may already have moved the message; destination UID is unavailable";
    sender
        .send(WorkerResult::Changed {
            account: Some(owner),
            action: DeleteAction::Trash,
            completed: Vec::new(),
            deferred: Vec::new(),
            uncertain: vec![(super::MessageId::from_stored(id.clone()), reason.into())],
            error: Some(reason.into()),
        })
        .unwrap();
    let Some(MailCompletion::Changed {
        count: 0,
        local_update: Err(error),
        ..
    }) = app.poll()
    else {
        panic!("Expected uncertain move warning");
    };
    assert!(error.contains("may already have moved"));
    assert_eq!(app.store.entry(&id).unwrap().folder, "Inbox");
    assert!(app.store.entry(&id).unwrap().remote.is_none());
    drop(app);
    let mut app = AppState::new(temp.store());
    app.list_messages(ReadFilter::All, "");
    app.select_message(0).unwrap();
    assert!(
        app.delete_selected()
            .unwrap_err()
            .contains("destination UID is unavailable")
    );
    assert!(!app.busy());
    assert_eq!(app.store.raw(&id).unwrap(), bytes);
}

#[test]
fn reader_identity_actions_do_not_follow_later_list_selection() {
    let temp = Temp::new();
    let mut store = temp.store();
    let first = store
        .import(&raw("first@example.com", "First", ""), "Inbox", None)
        .unwrap();
    let second = store
        .import(&raw("second@example.com", "Second", ""), "Inbox", None)
        .unwrap();
    let mut app = AppState::new(store);
    app.list_messages(ReadFilter::All, "");
    let first_id = app.visible_message_id(1).unwrap().clone();
    let second_id = app.visible_message_id(0).unwrap().clone();
    assert_eq!(first_id.as_str(), first);
    assert_eq!(second_id.as_str(), second);
    app.select_message(0).unwrap();
    let Selection::Message(message) = app.read_message(&first_id).unwrap() else {
        panic!("Expected mail");
    };
    assert_eq!(message.summary.subject, "First");
    assert_eq!(app.selected_id(), Some(&second_id));
    app.set_scope(DataScope::Local).unwrap();
    app.list_messages(ReadFilter::All, "Second");
    app.select_message(0).unwrap();
    let draft = app.reply_message(&first_id, ReplyKind::Sender).unwrap();
    assert_eq!(draft.to, "first@example.com");
    app.delete_message(&first_id).unwrap();
    assert_eq!(app.message_entry(&first_id).unwrap().folder, "Trash");
    assert_eq!(app.message_entry(&second_id).unwrap().folder, "Inbox");
    assert_eq!(app.selected_id(), Some(&second_id));
    assert!(app.message(&first_id).unwrap().body.contains("body"));
}
