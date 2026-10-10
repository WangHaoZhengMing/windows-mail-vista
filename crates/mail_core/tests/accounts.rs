use mail_core::{
    account::{Account, Incoming, Security},
    message::{self, Draft},
    network::ImapLocation,
    store::Store,
};
use std::path::PathBuf;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!("winmail-accounts-{}-{nonce}", std::process::id())))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn account(email: &str) -> Account {
    Account {
        email: email.into(),
        username: email.into(),
        auth: Default::default(),
        password_dpapi: String::new(),
        incoming: Incoming::Imap,
        incoming_host: "127.0.0.1".into(),
        incoming_port: 143,
        incoming_security: Security::Plain,
        smtp_host: "127.0.0.1".into(),
        smtp_port: 25,
        smtp_security: Security::Plain,
    }
}
const RAW: &[u8] = b"From: sender@example.com\r\nSubject: shared bytes\r\n\r\nbody";

#[test]
fn legacy_catalog_migrates_remote_ownership_and_keeps_local_messages() {
    let temp = Temp::new();
    std::fs::create_dir_all(temp.0.join("messages")).unwrap();
    let account = account("legacy@example.com");
    let raw_local = b"Subject: manual import\r\n\r\nlocal";
    let remote_id = message::hash(RAW);
    let local_id = message::hash(raw_local);
    for (id, raw) in [(&remote_id, RAW), (&local_id, raw_local.as_slice())] {
        std::fs::write(temp.0.join("messages").join(format!("{id}.eml")), raw).unwrap();
    }
    let key = format!("{}:imap:456:7", account.id());
    let legacy = serde_json::json!({
        "account": account,
        "messages": [
            { "id":remote_id, "folder":"Inbox", "summary":message::parse(RAW).unwrap().summary, "read":true, "remote_key":key },
            { "id":local_id, "folder":"Inbox", "summary":message::parse(raw_local).unwrap().summary, "read":false, "remote_key":null }
        ],
        "remote_seen": [key], "drafts": {}
    });
    std::fs::write(
        temp.0.join("index.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    let stable = {
        let store = Store::open(&temp.0).unwrap();
        assert_eq!(store.accounts().len(), 1);
        let id = store.default_account_id().unwrap().clone();
        assert_eq!(store.accounts()[0].name, "legacy@example.com");
        let entry = store.entry(&remote_id).unwrap();
        assert_eq!(entry.account_id.as_ref(), Some(&id));
        assert_eq!(entry.remote.as_ref().unwrap().uid, 7);
        assert!(entry.read);
        assert!(store.entry(&local_id).unwrap().account_id.is_none());
        assert_eq!(store.known_remote_for(&id), [key]);
        id
    };
    let store = Store::open(&temp.0).unwrap();
    assert_eq!(store.default_account_id(), Some(&stable));
    assert_eq!(store.raw(&local_id).unwrap(), raw_local);
}

#[test]
fn identical_accounts_and_messages_remain_independent_after_settings_changes() {
    let temp = Temp::new();
    let mut store = Store::open(&temp.0).unwrap();
    let config = account("same@example.com");
    let first = store
        .add_account_named("Personal".into(), config.clone())
        .unwrap();
    let second = store
        .add_account_named("Work".into(), config.clone())
        .unwrap();
    assert_ne!(first, second);
    let key = format!("{}:imap:456:7", config.id());
    let one = store
        .import_for(Some(&first), RAW, "Inbox", Some(key.clone()))
        .unwrap();
    assert!(store.known_remote_for(&second).is_empty());
    let two = store
        .import_for(Some(&second), RAW, "Inbox", Some(key.clone()))
        .unwrap();
    assert_ne!(one, two);
    store.mark_read(&one).unwrap();
    store.trash(&one, None).unwrap();
    assert!(!store.entry(&two).unwrap().read);
    assert_eq!(store.entry(&two).unwrap().folder, "Inbox");
    let mut changed = config;
    changed.incoming_host = "new.example.com".into();
    store
        .update_account_named(&first, "Renamed".into(), changed)
        .unwrap();
    assert_eq!(store.entry(&one).unwrap().account_id.as_ref(), Some(&first));
    store.set_default_account(&second).unwrap();
    drop(store);
    let store = Store::open(&temp.0).unwrap();
    assert_eq!(store.default_account_id(), Some(&second));
    assert_eq!(store.accounts()[0].name, "Renamed");
    assert_eq!(store.known_remote_for(&first), [key]);
}

#[test]
fn removing_account_preserves_downloads_and_saved_draft_attachments_as_local() {
    let temp = Temp::new();
    let mut store = Store::open(&temp.0).unwrap();
    let config = account("first@example.com");
    let first = store.add_account(config.clone()).unwrap();
    let second = store.add_account(account("second@example.com")).unwrap();
    let key = format!("{}:imap:1:1", config.id());
    let id = store
        .import_for(Some(&first), RAW, "Inbox", Some(key))
        .unwrap();
    let source = temp.0.join("attachment.bin");
    std::fs::write(&source, b"attachment bytes").unwrap();
    let draft = Draft {
        attachments: vec![source.clone()],
        ..Draft::default()
    };
    let draft_id = store
        .save_draft_for(Some(&first), &config.email, &draft)
        .unwrap();
    std::fs::remove_file(source).unwrap();
    store.remove_account(&first).unwrap();
    assert_eq!(store.default_account_id(), Some(&second));
    assert!(
        store
            .entries()
            .iter()
            .all(|entry| entry.account_id.is_none() && entry.remote.is_none())
    );
    assert!(store.known_remote_for(&first).is_empty());
    assert_eq!(store.raw(&id).unwrap(), RAW);
    assert_eq!(
        std::fs::read(&store.draft(&draft_id).unwrap().attachments[0]).unwrap(),
        b"attachment bytes"
    );
}

#[test]
fn trash_restore_tracks_remote_uid_and_permanent_deletion_cannot_recover() {
    let temp = Temp::new();
    let mut store = Store::open(&temp.0).unwrap();
    let config = account("first@example.com");
    let account = store.add_account(config.clone()).unwrap();
    let id = store
        .import_for(
            Some(&account),
            RAW,
            "Inbox",
            Some(format!("{}:imap:1:7", config.id())),
        )
        .unwrap();
    let moved = ImapLocation {
        connection: config.id(),
        mailbox: "Trash".into(),
        uid_validity: 2,
        uid: 11,
    };
    store.trash(&id, Some(moved)).unwrap();
    assert_eq!(
        store.entry(&id).unwrap().restore_mailbox.as_deref(),
        Some("INBOX")
    );
    let restored = ImapLocation {
        connection: config.id(),
        mailbox: "INBOX".into(),
        uid_validity: 1,
        uid: 15,
    };
    store.restore(&id, Some(restored)).unwrap();
    assert_eq!(store.entry(&id).unwrap().folder, "Inbox");
    assert!(
        store
            .known_remote_for(&account)
            .contains(&format!("{}:imap:1:15", config.id()))
    );
    assert!(store.permanently_delete(&id).is_err());
    store.trash(&id, None).unwrap();
    store.permanently_delete(&id).unwrap();
    assert!(store.raw(&id).is_err());
    // Simulate cleanup interrupted after the catalog was committed.
    std::fs::write(temp.0.join("messages").join(format!("{id}.eml")), RAW).unwrap();
    drop(store);
    let store = Store::open(&temp.0).unwrap();
    assert!(store.entries().is_empty());
}

#[test]
fn failed_catalog_write_does_not_publish_half_added_account() {
    let temp = Temp::new();
    let mut store = Store::open(&temp.0).unwrap();
    std::fs::create_dir(temp.0.join("index.json")).unwrap();
    assert!(
        store
            .add_account_named("Unsaved".into(), account("first@example.com"))
            .is_err()
    );
    assert!(store.accounts().is_empty());
    assert!(store.default_account_id().is_none());
}

#[test]
fn compatibility_outbox_sent_and_draft_use_the_default_account() {
    let temp = Temp::new();
    let mut store = Store::open(&temp.0).unwrap();
    let config = account("same@example.com");
    let first = store.add_account(config.clone()).unwrap();
    let default = store.add_account(config.clone()).unwrap();
    store.set_default_account(&default).unwrap();
    let outbox = store.import(RAW, "Outbox", None).unwrap();
    store.move_to(&outbox, "Sent").unwrap();
    assert_eq!(
        store.entry(&outbox).unwrap().account_id.as_ref(),
        Some(&default)
    );
    let draft = store.save_draft(&config.email, &Draft::default()).unwrap();
    assert_eq!(
        store.entry(&draft).unwrap().account_id.as_ref(),
        Some(&default)
    );
    let received = store
        .import(RAW, "Inbox", Some(format!("{}:imap:1:7", config.id())))
        .unwrap();
    assert_eq!(
        store.entry(&received).unwrap().account_id.as_ref(),
        Some(&default)
    );
    assert!(store.known_remote_for(&first).is_empty());
}
