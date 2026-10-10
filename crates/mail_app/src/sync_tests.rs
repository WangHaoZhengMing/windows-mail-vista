use super::*;
use mail_core::{
    account::{Account, Authentication, Incoming, Security},
    network::*,
    oauth::*,
    store::Store,
};
use std::{
    collections::BTreeMap,
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
                "winmail-app-sync-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )))
    }
    fn store(&self) -> Store {
        Store::open(&self.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn account() -> Account {
    Account {
        auth: Authentication::Password,
        email: "me@example.com".into(),
        username: "me@example.com".into(),
        password_dpapi: String::new(),
        incoming: Incoming::Imap,
        incoming_host: "localhost".into(),
        incoming_port: 993,
        incoming_security: Security::Tls,
        smtp_host: "localhost".into(),
        smtp_port: 587,
        smtp_security: Security::StartTls,
    }
}
fn folder(mailbox: &str, role: Option<ImapFolderRole>) -> ImapFolder {
    ImapFolder {
        mailbox: mailbox.into(),
        display_name: mailbox.into(),
        delimiter: Some('/'),
        selectable: true,
        role,
    }
}
fn report(account: &Account) -> ImapSyncReport {
    let folders = vec![
        folder("INBOX", Some(ImapFolderRole::Inbox)),
        folder("Work/Inbox", Some(ImapFolderRole::Inbox)),
        folder("Drafts", Some(ImapFolderRole::Drafts)),
        folder("Archive", Some(ImapFolderRole::Archive)),
        folder("Trash", Some(ImapFolderRole::Trash)),
    ];
    let mut result = ImapSyncReport {
        folders,
        ..Default::default()
    };
    for (index, mailbox) in ["INBOX", "Work/Inbox", "Drafts", "Trash"]
        .into_iter()
        .enumerate()
    {
        let location = ImapLocation {
            connection: account.id(),
            mailbox: mailbox.into(),
            uid_validity: 17,
            uid: index as u32 + 1,
        };
        let flags = ImapFlags::default();
        result.mailboxes.push(ImapMailboxSnapshot {
            mailbox: mailbox.into(),
            uid_validity: 17,
            uid_next: Some(index as u32 + 2),
            messages: BTreeMap::from([(location.uid, flags.clone())]),
        });
        result.downloads.push(ImapDownload {
            location,
            flags,
            raw: format!("From: from@example.com\r\nSubject: {mailbox}\r\n\r\nServer MIME body")
                .into_bytes(),
        });
    }
    result.mailboxes.push(ImapMailboxSnapshot {
        mailbox: "Archive".into(),
        uid_validity: 19,
        uid_next: Some(1),
        messages: BTreeMap::new(),
    });
    result
}
#[test]
fn server_selection_is_exact_and_remote_drafts_are_regular_mime() {
    let fixture = Fixture::new();
    let config = account();
    let mut store = fixture.store();
    let owner = store.add_account(config.clone()).unwrap();
    store.apply_imap_sync(&owner, report(&config)).unwrap();
    let mut app = AppState::new(store);
    app.set_scope(DataScope::Account(owner)).unwrap();
    app.set_folder_selection(FolderSelection::ServerMailbox("Work/Inbox".into()));
    let entries = app.list_messages(ReadFilter::All, "");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].summary.subject, "Work/Inbox");
    app.set_folder_selection(FolderSelection::ServerMailbox("Drafts".into()));
    app.list_messages(ReadFilter::All, "");
    assert!(matches!(
        app.select_message(0).unwrap(),
        Selection::Message(_)
    ));
    app.mark_selected_read(false).unwrap();
    app.toggle_selected_flag().unwrap();
    let selected = app.message_entry(app.selected_id().unwrap()).unwrap();
    assert!(!selected.read);
    assert!(selected.flags.flagged);
    let changes = app
        .store
        .imap_sync_request(app.active_account_id().unwrap())
        .unwrap()
        .pending_flags;
    assert!(
        changes
            .iter()
            .any(|change| change.add.contains(&"\\Flagged".into())
                && change.remove.contains(&"\\Seen".into()))
    );
}
#[test]
fn oauth_credentials_are_saved_before_ack_even_if_protocol_later_fails() {
    let fixture = Fixture::new();
    let mut config = account();
    config.auth = Authentication::OAuth2(OAuthAccount {
        config: OAuthConfig::microsoft("desktop-client-id"),
        tokens: None,
    });
    let mut store = fixture.store();
    let owner = store.add_account(config.clone()).unwrap();
    let Authentication::OAuth2(auth) = &mut config.auth else {
        unreachable!()
    };
    auth.tokens = Some(OAuthTokens {
        protected: mail_core::account::protect(
            br#"{"access_token":"fixture-access","refresh_token":"fixture-refresh"}"#,
        )
        .unwrap(),
        expires_at: u64::MAX,
    });
    let mut app = AppState::new(store);
    let (sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    let worker_owner = owner.clone();
    let worker = std::thread::spawn(move || {
        super::sync::refresh_credentials(&worker_owner, &mut config, &sender).unwrap();
        sender
            .send(super::mail::WorkerResult::Synced {
                account: worker_owner,
                result: Err("IMAP unavailable".into()),
            })
            .unwrap();
    });
    let deadline = SystemTime::now() + Duration::from_secs(5);
    loop {
        if let Some(MailCompletion::Failed(error)) = app.poll() {
            assert!(error.contains("IMAP unavailable"));
            break;
        }
        assert!(SystemTime::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    worker.join().unwrap();
    drop(app);
    let reopened = fixture.store();
    let Authentication::OAuth2(saved) = &reopened.account_record(&owner).unwrap().account.auth
    else {
        panic!("OAuth lost");
    };
    assert_eq!(saved.access_token().unwrap(), "fixture-access");
}
#[test]
fn stale_credential_event_is_rejected_and_preserves_current_account() {
    let fixture = Fixture::new();
    let mut app = AppState::new(fixture.store());
    let owner = app.add_account(account()).unwrap();
    let (sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    let (reply, ack) = mpsc::channel();
    sender
        .send(super::mail::WorkerResult::Credentials {
            account: owner.clone(),
            connection: "old-server".into(),
            authentication: Authentication::OAuth2(OAuthAccount {
                config: OAuthConfig::google("another-client"),
                tokens: None,
            }),
            reply,
        })
        .unwrap();
    assert!(app.poll().is_none());
    assert!(ack.recv().unwrap().is_err());
    assert!(matches!(
        app.store.account_record(&owner).unwrap().account.auth,
        Authentication::Password
    ));
}
#[test]
fn deferred_deletion_and_generic_move_keep_stable_local_id() {
    let fixture = Fixture::new();
    let config = account();
    let mut store = fixture.store();
    let owner = store.add_account(config.clone()).unwrap();
    store.apply_imap_sync(&owner, report(&config)).unwrap();
    let mut app = AppState::new(store);
    let trash = app
        .store
        .entries()
        .iter()
        .find(|entry| entry.folder == "Trash")
        .unwrap();
    let id = MessageId::from_stored(trash.id.clone());
    let source = trash.remote.clone().unwrap();
    let (sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    sender
        .send(super::mail::WorkerResult::Changed {
            account: Some(owner),
            action: DeleteAction::PermanentDelete,
            completed: vec![],
            uncertain: vec![],
            deferred: vec![(id.clone(), ImapChange::MarkedDeleted(source.clone()))],
            error: None,
        })
        .unwrap();
    assert!(matches!(
        app.poll(),
        Some(MailCompletion::Changed {
            count: 1,
            deferred: 1,
            local_update: Ok(()),
            ..
        })
    ));
    assert!(app.message_entry(&id).unwrap().pending_delete);
    assert!(
        app.permanently_delete_message(&id)
            .unwrap_err()
            .contains("already marked")
    );
    let destination = ImapLocation {
        mailbox: "Archive".into(),
        uid_validity: 19,
        uid: 7,
        ..source.clone()
    };
    assert!(matches!(
        app.complete_move(
            id.clone(),
            Ok(ImapChange::MovedDeferred {
                destination: destination.clone(),
                original: source
            })
        ),
        MailCompletion::Moved {
            deferred: true,
            local_update: Ok(()),
            ..
        }
    ));
    assert_eq!(
        app.message_entry(&id).unwrap().remote.as_ref(),
        Some(&destination)
    );
    assert!(!app.message_entry(&id).unwrap().pending_delete);
    assert!(
        app.store
            .raw(id.as_str())
            .unwrap()
            .ends_with(b"Server MIME body")
    );
}
