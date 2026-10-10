use mail_core::{
    account::Account,
    network::{self, ImapFlags, ImapSyncReport},
    store::Store,
};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[derive(Clone)]
struct Mailbox {
    attributes: String,
    validity: u32,
    messages: BTreeMap<u32, (Vec<u8>, ImapFlags)>,
    reject_flags: bool,
}
struct Fixture {
    port: u16,
    folders: Arc<Mutex<BTreeMap<String, Mailbox>>>,
    commands: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new(folders: BTreeMap<String, Mailbox>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let folders = Arc::new(Mutex::new(folders));
        let commands = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_folders = folders.clone();
        let worker_commands = commands.clone();
        let worker_stop = stop.clone();
        let thread = thread::spawn(move || {
            for stream in listener.incoming() {
                if worker_stop.load(Ordering::SeqCst) {
                    break;
                }
                let stream = stream.unwrap();
                stream.set_nodelay(true).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut stream = BufReader::new(stream);
                write(&mut stream, b"* OK fixture\r\n");
                let mut selected = String::new();
                loop {
                    let mut line = String::new();
                    match stream.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => (),
                        Err(_) => break,
                    }
                    let line = line.trim_end_matches(['\r', '\n']);
                    let (tag, command) = line.split_once(' ').unwrap();
                    worker_commands.lock().unwrap().push(command.into());
                    let mut folders = worker_folders.lock().unwrap();
                    if command.starts_with("LOGIN ") {
                        okay(&mut stream, tag);
                    } else if command == "CAPABILITY" {
                        write(&mut stream, b"* CAPABILITY IMAP4rev1 SPECIAL-USE\r\n");
                        okay(&mut stream, tag);
                    } else if command.starts_with("LIST ") {
                        for (name, mailbox) in &*folders {
                            write(
                                &mut stream,
                                format!("* LIST ({}) \"/\" \"{}\"\r\n", mailbox.attributes, name)
                                    .as_bytes(),
                            );
                        }
                        okay(&mut stream, tag);
                    } else if let Some(mailbox) = command.strip_prefix("SELECT ") {
                        selected = mailbox.trim_matches('"').into();
                        let mailbox = folders.get(&selected).unwrap();
                        let next = mailbox
                            .messages
                            .last_key_value()
                            .map_or(1, |(uid, _)| uid + 1);
                        write(&mut stream, format!("* {} EXISTS\r\n* OK [UIDVALIDITY {}] valid\r\n* OK [UIDNEXT {next}] next\r\n", mailbox.messages.len(), mailbox.validity).as_bytes());
                        okay(&mut stream, tag);
                    } else if command == "UID SEARCH ALL" {
                        let mailbox = folders.get(&selected).unwrap();
                        let values = mailbox
                            .messages
                            .keys()
                            .map(u32::to_string)
                            .collect::<Vec<_>>()
                            .join(" ");
                        write(&mut stream, format!("* SEARCH {values}\r\n").as_bytes());
                        okay(&mut stream, tag);
                    } else if let Some(fetch) = command.strip_prefix("UID FETCH ") {
                        let (set, attributes) = fetch.split_once(' ').unwrap();
                        let mailbox = folders.get(&selected).unwrap();
                        if mailbox.reject_flags && attributes == "(UID FLAGS)" {
                            write(&mut stream, format!("{tag} NO refused\r\n").as_bytes());
                            continue;
                        }
                        for uid in set.split(',').map(|value| value.parse::<u32>().unwrap()) {
                            if let Some((raw, flags)) = mailbox.messages.get(&uid) {
                                if attributes == "(UID FLAGS)" {
                                    write(
                                        &mut stream,
                                        format!(
                                            "* 1 FETCH (UID {uid} FLAGS ({}))\r\n",
                                            wire_flags(flags)
                                        )
                                        .as_bytes(),
                                    );
                                } else {
                                    assert_eq!(attributes, "(UID BODY.PEEK[])");
                                    // UID after body literal exercises real server ordering.
                                    write(
                                        &mut stream,
                                        format!("* 1 FETCH (BODY[] {{{}}}\r\n", raw.len())
                                            .as_bytes(),
                                    );
                                    write(&mut stream, raw);
                                    write(&mut stream, format!(" UID {uid})\r\n").as_bytes());
                                }
                            }
                        }
                        okay(&mut stream, tag);
                    } else if let Some(store) = command.strip_prefix("UID STORE ") {
                        let mut pieces = store.splitn(3, ' ');
                        let uid = pieces.next().unwrap().parse::<u32>().unwrap();
                        let operation = pieces.next().unwrap();
                        let flags = pieces.next().unwrap().trim_matches(['(', ')']);
                        if let Some((_, current)) =
                            folders.get_mut(&selected).unwrap().messages.get_mut(&uid)
                        {
                            for flag in flags.split_whitespace() {
                                let enabled = operation == "+FLAGS.SILENT";
                                match flag {
                                    "\\Seen" => current.seen = enabled,
                                    "\\Flagged" => current.flagged = enabled,
                                    "\\Answered" => current.answered = enabled,
                                    "\\Draft" => current.draft = enabled,
                                    keyword if enabled => current.keywords.push(keyword.into()),
                                    keyword => current.keywords.retain(|value| value != keyword),
                                }
                            }
                        }
                        okay(&mut stream, tag);
                    } else if command == "LOGOUT" {
                        break;
                    } else {
                        panic!("Unexpected command: {command}");
                    }
                }
            }
        });
        Self {
            port,
            folders,
            commands,
            stop,
            thread: Some(thread),
        }
    }
    fn account(&self) -> Account {
        let mut account: Account = serde_json::from_value(serde_json::json!({
            "email":"sender@example.com", "username":"user", "password_dpapi":"",
            "incoming":"imap", "incoming_host":"127.0.0.1", "incoming_port":self.port,
            "incoming_security":"plain", "smtp_host":"127.0.0.1", "smtp_port":self.port, "smtp_security":"plain"
        })).unwrap();
        account.set_password("test-pass").unwrap();
        account
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        self.thread.take().unwrap().join().unwrap();
    }
}
fn write(stream: &mut BufReader<TcpStream>, bytes: &[u8]) {
    stream.get_mut().write_all(bytes).unwrap();
}
fn okay(stream: &mut BufReader<TcpStream>, tag: &str) {
    write(stream, format!("{tag} OK done\r\n").as_bytes());
}
fn wire_flags(flags: &ImapFlags) -> String {
    let mut values = flags.keywords.clone();
    for (enabled, flag) in [
        (flags.seen, "\\Seen"),
        (flags.flagged, "\\Flagged"),
        (flags.answered, "\\Answered"),
        (flags.deleted, "\\Deleted"),
        (flags.draft, "\\Draft"),
    ] {
        if enabled {
            values.push(flag.into());
        }
    }
    values.join(" ")
}
fn raw(subject: &str) -> Vec<u8> {
    format!("From: test@example.com\r\nTo: sender@example.com\r\nSubject: {subject}\r\n\r\nBody {subject}\r\n").into_bytes()
}
fn mailbox(attributes: &str, values: &[(u32, &str, ImapFlags)]) -> Mailbox {
    Mailbox {
        attributes: attributes.into(),
        validity: 41,
        reject_flags: false,
        messages: values
            .iter()
            .map(|(uid, subject, flags)| (*uid, (raw(subject), flags.clone())))
            .collect(),
    }
}
fn store() -> (std::path::PathBuf, Store) {
    let root = std::env::temp_dir().join(format!(
        "winmail-imap-sync-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let store = Store::open(&root).unwrap();
    (root, store)
}
fn sync(
    store: &mut Store,
    id: &mail_core::account::AccountId,
    account: &Account,
) -> mail_core::store::SyncSummary {
    let mut events = Vec::new();
    let report = network::sync_imap_with_progress(
        account,
        &store.imap_sync_request(id).unwrap(),
        &mut |event| events.push(event),
    )
    .unwrap();
    assert!(events.iter().any(|event| event.contains("Connecting")));
    assert!(events.iter().any(|event| event.contains("Checked")));
    if !report.downloads.is_empty() {
        assert!(events.iter().any(|event| event.contains(" KB")));
    }
    store.apply_imap_sync(id, report).unwrap()
}

#[test]
fn cancelled_sync_keeps_each_completed_body_and_restart_does_not_fetch_it_again() {
    let fixture = Fixture::new(BTreeMap::from([(
        "INBOX".into(),
        mailbox(
            "",
            &[
                (1, "first", ImapFlags::default()),
                (2, "second", ImapFlags::default()),
            ],
        ),
    )]));
    let account = fixture.account();
    let (root, mut store) = store();
    let id = store.add_account(account.clone()).unwrap();
    let directory = root.join("checkpoints");
    let control = network::SyncControl::new(directory.clone());
    let cancelled = control.clone();
    let request = store.imap_sync_request(&id).unwrap();
    let result = network::sync_imap_controlled(
        &account,
        &request,
        &mut |event| {
            if event.contains("downloaded 1 new messages") {
                cancelled.cancel();
            }
        },
        &control,
    );
    assert!(result.is_err());
    assert!(
        store.entries().is_empty(),
        "An interrupted snapshot must not reconcile the catalog"
    );
    assert_eq!(
        std::fs::read_dir(&directory).unwrap().count(),
        1,
        "The fully downloaded message must already be durable"
    );
    drop(store);
    let mut store = Store::open(&root).unwrap();
    let resumed = network::SyncControl::new(directory);
    let report = network::sync_imap_controlled(
        &account,
        &store.imap_sync_request(&id).unwrap(),
        &mut |_| {},
        &resumed,
    )
    .unwrap();
    let keys: Vec<_> = report
        .downloads
        .iter()
        .map(|download| download.location.remote_key())
        .collect();
    assert_eq!(store.apply_imap_sync(&id, report).unwrap().downloaded, 2);
    for key in keys {
        resumed.committed(&key);
    }
    assert_eq!(
        fixture
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| command.as_str() == "UID FETCH 2 (UID BODY.PEEK[])")
            .count(),
        1
    );
    assert_eq!(store.entries().len(), 2);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn nested_unicode_folders_flags_and_read_outbox_survive_reopen() {
    let fixture = Fixture::new(BTreeMap::from([
        (
            "INBOX".into(),
            mailbox("", &[(1, "inbox", ImapFlags::default())]),
        ),
        ("Projects".into(), mailbox("\\Noselect", &[])),
        (
            "Projects/&U,BTFw-".into(),
            mailbox(
                "",
                &[(
                    9,
                    "nested",
                    ImapFlags {
                        seen: true,
                        flagged: true,
                        answered: true,
                        keywords: vec!["$Label1".into()],
                        ..Default::default()
                    },
                )],
            ),
        ),
        (
            "Deleted Items".into(),
            mailbox("\\Trash", &[(7, "trash", ImapFlags::default())]),
        ),
    ]));
    let account = fixture.account();
    let (root, mut store) = store();
    let id = store.add_account(account.clone()).unwrap();
    assert_eq!(sync(&mut store, &id, &account).downloaded, 3);
    assert_eq!(store.server_folders(&id).len(), 4);
    assert!(
        store
            .server_folders(&id)
            .iter()
            .any(|folder| folder.display_name == "Projects/台北")
    );
    let nested = store
        .entries()
        .iter()
        .find(|entry| entry.summary.subject == "nested")
        .unwrap();
    assert!(nested.read && nested.flags.flagged && nested.flags.answered);
    let inbox = store
        .entries()
        .iter()
        .find(|entry| entry.summary.subject == "inbox")
        .unwrap()
        .id
        .clone();
    store.mark_read(&inbox).unwrap();
    store.mark_flagged(&inbox, true).unwrap();
    drop(store);
    let mut store = Store::open(&root).unwrap();
    assert_eq!(store.imap_sync_request(&id).unwrap().pending_flags.len(), 1);
    assert_eq!(sync(&mut store, &id, &account).pending, 0);
    let state = fixture.folders.lock().unwrap();
    let flags = &state["INBOX"].messages[&1].1;
    assert!(flags.seen && flags.flagged);
    drop(state);
    store.mark_unread(&inbox).unwrap();
    sync(&mut store, &id, &account);
    assert!(!fixture.folders.lock().unwrap()["INBOX"].messages[&1].1.seen);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bounded_downloads_continue_past_two_hundred_without_repeats() {
    let mut inbox = mailbox("", &[]);
    for uid in 1..=205 {
        inbox
            .messages
            .insert(uid, (raw(&format!("message-{uid}")), ImapFlags::default()));
    }
    let fixture = Fixture::new(BTreeMap::from([
        ("INBOX".into(), inbox),
        (
            "Sent".into(),
            mailbox(
                "\\Sent",
                &[(
                    1,
                    "sent",
                    ImapFlags {
                        seen: true,
                        ..Default::default()
                    },
                )],
            ),
        ),
    ]));
    let account = fixture.account();
    let (root, mut store) = store();
    let id = store.add_account(account.clone()).unwrap();
    let first = sync(&mut store, &id, &account);
    assert_eq!(first.downloaded, 200);
    assert!(first.has_more);
    let next = sync(&mut store, &id, &account);
    assert_eq!(next.downloaded, 6);
    assert!(!next.has_more);
    assert_eq!(sync(&mut store, &id, &account).downloaded, 0);
    assert_eq!(store.entries().len(), 206);
    let commands = fixture.commands.lock().unwrap();
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.contains("BODY.PEEK[]"))
            .count(),
        206
    );
    drop(commands);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn removed_mail_and_uidvalidity_reset_preserve_raw_copies_and_failed_folder_is_untouched() {
    let fixture = Fixture::new(BTreeMap::from([
        (
            "INBOX".into(),
            mailbox(
                "",
                &[
                    (1, "removed", ImapFlags::default()),
                    (2, "reset", ImapFlags::default()),
                ],
            ),
        ),
        (
            "Archive".into(),
            mailbox("\\Archive", &[(3, "untouched", ImapFlags::default())]),
        ),
    ]));
    let account = fixture.account();
    let (root, mut store) = store();
    let id = store.add_account(account.clone()).unwrap();
    sync(&mut store, &id, &account);
    let originals: Vec<_> = store
        .entries()
        .iter()
        .filter(|entry| entry.folder == "Inbox")
        .map(|entry| entry.id.clone())
        .collect();
    {
        let mut state = fixture.folders.lock().unwrap();
        state.get_mut("INBOX").unwrap().messages.remove(&1);
        state.get_mut("Archive").unwrap().reject_flags = true;
    }
    let result = sync(&mut store, &id, &account);
    assert_eq!(result.recovered, 1);
    assert!(!result.warnings.is_empty());
    assert_eq!(
        store
            .entries()
            .iter()
            .find(|entry| entry.summary.subject == "untouched")
            .unwrap()
            .folder,
        "Archive"
    );
    {
        let mut state = fixture.folders.lock().unwrap();
        state.get_mut("INBOX").unwrap().validity = 99;
    }
    let result = sync(&mut store, &id, &account);
    assert_eq!(result.recovered, 1);
    assert_eq!(result.downloaded, 1);
    for id in originals {
        assert!(!store.raw(&id).unwrap().is_empty());
        assert_eq!(store.entry(&id).unwrap().folder, "Recovered");
    }
    assert_eq!(
        store
            .entries()
            .iter()
            .filter(|entry| entry.folder == "Inbox")
            .count(),
        1
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn post_request_local_flags_override_stale_snapshot_and_remain_pending() {
    let fixture = Fixture::new(BTreeMap::from([(
        "INBOX".into(),
        mailbox("", &[(1, "race", ImapFlags::default())]),
    )]));
    let account = fixture.account();
    let (root, mut store) = store();
    let id = store.add_account(account.clone()).unwrap();
    sync(&mut store, &id, &account);
    let message = store.entries()[0].id.clone();
    store.mark_read(&message).unwrap();
    let request = store.imap_sync_request(&id).unwrap();
    let report = network::sync_imap(&account, &request).unwrap();
    store.mark_unread(&message).unwrap();
    store.mark_flagged(&message, true).unwrap();
    let applied = store.apply_imap_sync(&id, report).unwrap();
    assert_eq!(applied.pending, 1);
    assert!(!store.entry(&message).unwrap().read);
    assert!(store.entry(&message).unwrap().flags.flagged);
    sync(&mut store, &id, &account);
    assert!(!fixture.folders.lock().unwrap()["INBOX"].messages[&1].1.seen);
    assert!(
        fixture.folders.lock().unwrap()["INBOX"].messages[&1]
            .1
            .flagged
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn deferred_deletion_retains_trash_until_actual_uid_disappearance() {
    let fixture = Fixture::new(BTreeMap::from([
        ("INBOX".into(), mailbox("", &[])),
        (
            "Trash".into(),
            mailbox("\\Trash", &[(7, "retained", ImapFlags::default())]),
        ),
    ]));
    let account = fixture.account();
    let (root, mut store) = store();
    let id = store.add_account(account.clone()).unwrap();
    sync(&mut store, &id, &account);
    let message = store.entries()[0].id.clone();
    let source = store.entries()[0].remote.clone().unwrap();
    store.defer_permanent_delete(&message, source).unwrap();
    fixture
        .folders
        .lock()
        .unwrap()
        .get_mut("Trash")
        .unwrap()
        .messages
        .get_mut(&7)
        .unwrap()
        .1
        .deleted = true;
    let retained = sync(&mut store, &id, &account);
    assert_eq!(retained.pending, 1);
    assert_eq!(retained.purged, 0);
    assert!(store.entry(&message).unwrap().pending_delete);
    assert!(!store.raw(&message).unwrap().is_empty());
    fixture
        .folders
        .lock()
        .unwrap()
        .get_mut("Trash")
        .unwrap()
        .messages
        .remove(&7);
    let removed = sync(&mut store, &id, &account);
    assert_eq!(removed.pending, 0);
    assert_eq!(removed.purged, 1);
    assert!(store.entry(&message).is_none());
    drop(store);
    let reopened = Store::open(&root).unwrap();
    assert!(reopened.entry(&message).is_none());
    drop(reopened);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_or_failed_sync_never_reconciles_an_empty_folder_snapshot() {
    let fixture = Fixture::new(BTreeMap::from([(
        "INBOX".into(),
        mailbox("", &[(1, "safe", ImapFlags::default())]),
    )]));
    let account = fixture.account();
    let (root, mut store) = store();
    let id = store.add_account(account.clone()).unwrap();
    sync(&mut store, &id, &account);
    let message = store.entries()[0].id.clone();
    assert!(
        store
            .apply_imap_sync(&id, ImapSyncReport::default())
            .is_err()
    );
    assert_eq!(store.entry(&message).unwrap().folder, "Inbox");
    assert!(store.entry(&message).unwrap().remote.is_some());
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn server_move_reuses_local_identity_and_transfers_unacknowledged_read_intent() {
    let fixture = Fixture::new(BTreeMap::from([
        (
            "INBOX".into(),
            mailbox("", &[(1, "moved", ImapFlags::default())]),
        ),
        ("Archive".into(), mailbox("\\Archive", &[])),
    ]));
    let account = fixture.account();
    let (root, mut store) = store();
    let id = store.add_account(account.clone()).unwrap();
    sync(&mut store, &id, &account);
    let message = store.entries()[0].id.clone();
    store.mark_read(&message).unwrap();
    {
        let mut state = fixture.folders.lock().unwrap();
        let original = state.get_mut("INBOX").unwrap().messages.remove(&1).unwrap();
        state
            .get_mut("Archive")
            .unwrap()
            .messages
            .insert(12, original);
    }
    let result = sync(&mut store, &id, &account);
    assert_eq!(result.recovered, 0);
    assert_eq!(result.downloaded, 1);
    assert_eq!(store.entries().len(), 1);
    assert_eq!(store.entries()[0].id, message);
    assert_eq!(store.entries()[0].folder, "Archive");
    assert!(store.entries()[0].read);
    assert_eq!(result.pending, 1);
    sync(&mut store, &id, &account);
    assert!(
        fixture.folders.lock().unwrap()["Archive"].messages[&12]
            .1
            .seen
    );
    drop(store);
    let reopened = Store::open(&root).unwrap();
    assert_eq!(reopened.entries().len(), 1);
    drop(reopened);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn deferred_move_does_not_duplicate_the_deleted_source_and_keeps_target() {
    let fixture = Fixture::new(BTreeMap::from([
        (
            "INBOX".into(),
            mailbox("", &[(1, "copied", ImapFlags::default())]),
        ),
        ("Trash".into(), mailbox("\\Trash", &[])),
    ]));
    let account = fixture.account();
    let (root, mut store) = store();
    let id = store.add_account(account.clone()).unwrap();
    sync(&mut store, &id, &account);
    let message = store.entries()[0].id.clone();
    let source = store.entries()[0].remote.clone().unwrap();
    let destination = network::ImapLocation {
        mailbox: "Trash".into(),
        uid: 8,
        ..source.clone()
    };
    {
        let mut state = fixture.folders.lock().unwrap();
        let original = state["INBOX"].messages[&1].clone();
        state.get_mut("Trash").unwrap().messages.insert(8, original);
        state
            .get_mut("INBOX")
            .unwrap()
            .messages
            .get_mut(&1)
            .unwrap()
            .1
            .deleted = true;
    }
    store.trash_deferred(&message, destination, source).unwrap();
    let result = sync(&mut store, &id, &account);
    assert_eq!(result.downloaded, 0);
    assert_eq!(result.pending, 1);
    assert_eq!(store.entries().len(), 1);
    assert_eq!(store.entries()[0].folder, "Trash");
    fixture
        .folders
        .lock()
        .unwrap()
        .get_mut("INBOX")
        .unwrap()
        .messages
        .remove(&1);
    let result = sync(&mut store, &id, &account);
    assert_eq!(result.pending, 0);
    assert_eq!(result.purged, 0);
    assert_eq!(store.entries()[0].id, message);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn account_connection_edits_preserve_owned_cache_and_retire_old_server_outbox() {
    let fixture = Fixture::new(BTreeMap::from([(
        "INBOX".into(),
        mailbox("", &[(1, "previous server", ImapFlags::default())]),
    )]));
    let account = fixture.account();
    let (root, mut store) = store();
    let id = store.add_account(account.clone()).unwrap();
    sync(&mut store, &id, &account);
    let message = store.entries()[0].id.clone();
    store.mark_read(&message).unwrap();
    let mut changed = account.clone();
    changed.username = "another-user".into();
    store.update_account(&id, changed.clone()).unwrap();
    let previous = store.entry(&message).unwrap();
    assert_eq!(previous.folder, "Recovered");
    assert!(previous.remote.is_none());
    assert_eq!(previous.account_id.as_ref(), Some(&id));
    assert!(store.server_folders(&id).is_empty());
    assert!(
        store
            .imap_sync_request(&id)
            .unwrap()
            .pending_flags
            .is_empty()
    );
    assert!(!store.raw(&message).unwrap().is_empty());
    sync(&mut store, &id, &changed);
    assert_eq!(store.entries().len(), 2);
    assert_eq!(store.entry(&message).unwrap().folder, "Recovered");
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn large_uid_search_line_is_bounded_without_the_old_sixty_four_kib_cutoff() {
    let mut inbox = mailbox("", &[]);
    for uid in 1..=16000 {
        inbox
            .messages
            .insert(uid, (raw("large metadata"), ImapFlags::default()));
    }
    let fixture = Fixture::new(BTreeMap::from([("INBOX".into(), inbox)]));
    let account = fixture.account();
    let request = network::ImapSyncRequest {
        known: (1..=16000)
            .map(|uid| network::ImapLocation {
                connection: account.id(),
                mailbox: "INBOX".into(),
                uid_validity: 41,
                uid,
            })
            .collect(),
        ..Default::default()
    };
    let report = network::sync_imap(&account, &request).unwrap();
    assert!(report.warnings.is_empty());
    assert_eq!(report.mailboxes[0].messages.len(), 16000);
    assert!(report.downloads.is_empty());
    assert!(!report.has_more);
}

#[test]
fn another_clients_undelete_cancels_pending_purge_and_restores_deferred_source_without_id_collision()
 {
    let fixture = Fixture::new(BTreeMap::from([
        (
            "INBOX".into(),
            mailbox("", &[(1, "undeleted source", ImapFlags::default())]),
        ),
        ("Trash".into(), mailbox("\\Trash", &[])),
    ]));
    let account = fixture.account();
    let (root, mut store) = store();
    let id = store.add_account(account.clone()).unwrap();
    sync(&mut store, &id, &account);
    let message = store.entries()[0].id.clone();
    let source = store.entries()[0].remote.clone().unwrap();
    let destination = network::ImapLocation {
        mailbox: "Trash".into(),
        uid: 8,
        ..source.clone()
    };
    {
        let mut state = fixture.folders.lock().unwrap();
        let original = state["INBOX"].messages[&1].clone();
        state.get_mut("Trash").unwrap().messages.insert(8, original);
        state
            .get_mut("INBOX")
            .unwrap()
            .messages
            .get_mut(&1)
            .unwrap()
            .1
            .deleted = true;
    }
    store
        .trash_deferred(&message, destination.clone(), source)
        .unwrap();
    sync(&mut store, &id, &account);
    fixture
        .folders
        .lock()
        .unwrap()
        .get_mut("INBOX")
        .unwrap()
        .messages
        .get_mut(&1)
        .unwrap()
        .1
        .deleted = false;
    let result = sync(&mut store, &id, &account);
    assert_eq!(result.downloaded, 1);
    assert_eq!(result.pending, 0);
    assert_eq!(store.entries().len(), 2);
    assert_ne!(store.entries()[0].id, store.entries()[1].id);
    assert_eq!(store.entry(&message).unwrap().folder, "Trash");
    store.defer_permanent_delete(&message, destination).unwrap();
    fixture
        .folders
        .lock()
        .unwrap()
        .get_mut("Trash")
        .unwrap()
        .messages
        .get_mut(&8)
        .unwrap()
        .1
        .deleted = false;
    let result = sync(&mut store, &id, &account);
    assert_eq!(result.downloaded, 0);
    assert_eq!(result.pending, 0);
    assert_eq!(result.purged, 0);
    assert!(!store.entry(&message).unwrap().pending_delete);
    assert_eq!(store.entries().len(), 2);
    drop(store);
    let reopened = Store::open(&root).unwrap();
    assert_eq!(reopened.entries().len(), 2);
    drop(reopened);
    std::fs::remove_dir_all(root).unwrap();
}
