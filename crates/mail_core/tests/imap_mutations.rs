//! Real loopback protocol transcripts: every command is checked and no live
//! account, global EXPUNGE, or destructive CLOSE can be reached by these fixtures.
use mail_core::{
    account::{Account, Incoming, Security},
    network::{self, ImapChange, ImapLocation},
};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

const RAW: &[u8] =
    b"From: same@example.com\r\nMessage-ID: <repeated-id>\r\nSubject: ours\r\n\r\nExact body";
const DIFFERENT: &[u8] =
    b"From: same@example.com\r\nMessage-ID: <repeated-id>\r\nSubject: other\r\n\r\nDifferent body";
fn config(port: u16) -> Account {
    let mut result = Account {
        auth: Default::default(),
        email: "sender@example.com".into(),
        username: "user".into(),
        password_dpapi: String::new(),
        incoming: Incoming::Imap,
        incoming_host: "127.0.0.1".into(),
        incoming_port: port,
        incoming_security: Security::Plain,
        smtp_host: "127.0.0.1".into(),
        smtp_port: port,
        smtp_security: Security::Plain,
    };
    result.set_password("test-pass").unwrap();
    result
}
fn server() -> (TcpListener, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}
fn connected(listener: TcpListener, capabilities: &str) -> BufReader<TcpStream> {
    let (stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut s = BufReader::new(stream);
    send(&mut s, "* OK fixture\r\n");
    let tag = expect(&mut s, "LOGIN \"user\" \"test-pass\"");
    ok(&mut s, &tag);
    let tag = expect(&mut s, "CAPABILITY");
    send(&mut s, &format!("* CAPABILITY {capabilities}\r\n"));
    ok(&mut s, &tag);
    s
}
fn read(s: &mut BufReader<TcpStream>) -> String {
    let mut result = String::new();
    s.read_line(&mut result).unwrap();
    result.trim_end_matches(['\r', '\n']).into()
}
fn expect(s: &mut BufReader<TcpStream>, expected: &str) -> String {
    let text = read(s);
    let (tag, command) = text
        .split_once(' ')
        .unwrap_or_else(|| panic!("Missing command: {text}"));
    assert_eq!(command, expected);
    tag.into()
}
fn send(s: &mut BufReader<TcpStream>, text: &str) {
    s.get_mut().write_all(text.as_bytes()).unwrap();
}
fn ok(s: &mut BufReader<TcpStream>, tag: &str) {
    send(s, &format!("{tag} OK completed\r\n"));
}
fn location(account: &Account, mailbox: &str, validity: u32, uid: u32) -> ImapLocation {
    ImapLocation {
        connection: account.id(),
        mailbox: mailbox.into(),
        uid_validity: validity,
        uid,
    }
}
fn select(s: &mut BufReader<TcpStream>, command: &str, validity: u32) {
    let tag = expect(s, command);
    send(s, &format!("* OK [UIDVALIDITY {validity}] epoch\r\n"));
    ok(s, &tag);
}
fn fetch(s: &mut BufReader<TcpStream>, uid: u32, raw: &[u8]) {
    let tag = expect(s, &format!("UID FETCH {uid} (UID BODY.PEEK[])"));
    // Deliberately put UID after the literal; valid IMAP is not order-dependent.
    send(s, &format!("* 2 FETCH (BODY[] {{{}}}\r\n", raw.len()));
    s.get_mut().write_all(raw).unwrap();
    send(s, &format!(" UID {uid})\r\n"));
    ok(s, &tag);
}
fn preflight(s: &mut BufReader<TcpStream>) {
    select(s, "SELECT \"INBOX\"", 456);
    let tag = expect(s, "STATUS \"Archive\" (UIDVALIDITY UIDNEXT)");
    send(s, "* STATUS \"Archive\" (UIDNEXT 42 UIDVALIDITY 999)\r\n");
    ok(s, &tag);
    fetch(s, 7, RAW);
}
fn candidates(s: &mut BufReader<TcpStream>, values: &str) {
    select(s, "EXAMINE \"Archive\"", 999);
    let tag = expect(s, "UID SEARCH UID 42:*");
    send(s, &format!("* SEARCH {values}\r\n"));
    ok(s, &tag);
}
fn logout(s: &mut BufReader<TcpStream>) {
    assert_eq!(read(s), "R9999 LOGOUT");
}

#[test]
fn move_without_uidplus_proves_exact_new_content_not_message_id() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1 MOVE");
        preflight(&mut s);
        let tag = expect(&mut s, "UID MOVE 7 \"Archive\"");
        ok(&mut s, &tag);
        candidates(&mut s, "20 42 43"); // old n:* tail is ignored, not considered a new copy.
        fetch(&mut s, 42, DIFFERENT); // same Message-ID is insufficient.
        fetch(&mut s, 43, RAW);
        logout(&mut s);
    });
    let account = config(port);
    let result =
        network::imap_move(&account, &location(&account, "INBOX", 456, 7), "Archive").unwrap();
    let ImapChange::Moved(destination) = result else {
        panic!("Expected proven destination");
    };
    assert_eq!(destination, location(&account, "Archive", 999, 43));
    worker.join().unwrap();
}
#[test]
fn copy_without_uidplus_marks_only_source_and_retains_deferred_cleanup() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1");
        preflight(&mut s);
        let tag = expect(&mut s, "UID COPY 7 \"Archive\"");
        ok(&mut s, &tag);
        candidates(&mut s, "42");
        fetch(&mut s, 42, RAW);
        select(&mut s, "SELECT \"INBOX\"", 456);
        let tag = expect(&mut s, "UID STORE 7 +FLAGS.SILENT (\\Deleted)");
        ok(&mut s, &tag);
        logout(&mut s); // Never EXPUNGE/CLOSE other clients' Deleted messages.
    });
    let account = config(port);
    let original = location(&account, "INBOX", 456, 7);
    let ImapChange::MovedDeferred {
        destination,
        original: pending,
    } = network::imap_move(&account, &original, "Archive").unwrap()
    else {
        panic!("The original is only flagged, so cleanup must remain pending");
    };
    assert_eq!(pending, original);
    assert_eq!(destination, location(&account, "Archive", 999, 42));
    worker.join().unwrap();
}
#[test]
fn two_identical_new_copies_are_ambiguous_and_never_delete_the_source() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1");
        preflight(&mut s);
        let tag = expect(&mut s, "UID COPY 7 \"Archive\"");
        ok(&mut s, &tag);
        candidates(&mut s, "42 43");
        fetch(&mut s, 42, RAW);
        fetch(&mut s, 43, RAW);
        logout(&mut s);
    });
    let account = config(port);
    let ImapChange::Uncertain(reason) =
        network::imap_move(&account, &location(&account, "INBOX", 456, 7), "Archive").unwrap()
    else {
        panic!("An ambiguous successful copy must block duplicate retries");
    };
    assert!(reason.contains("ambiguous"));
    assert!(reason.contains("original was not marked deleted"));
    worker.join().unwrap();
}
#[test]
fn target_epoch_change_after_move_is_uncertain_without_guessing() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1 MOVE");
        preflight(&mut s);
        let tag = expect(&mut s, "UID MOVE 7 \"Archive\"");
        ok(&mut s, &tag);
        select(&mut s, "EXAMINE \"Archive\"", 1000);
        logout(&mut s);
    });
    let account = config(port);
    let ImapChange::Uncertain(reason) =
        network::imap_move(&account, &location(&account, "INBOX", 456, 7), "Archive").unwrap()
    else {
        panic!("Destination epoch changed");
    };
    assert!(reason.contains("UIDVALIDITY changed"));
    worker.join().unwrap();
}
#[test]
fn source_epoch_is_revalidated_after_copy_identity_scan() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1");
        preflight(&mut s);
        let tag = expect(&mut s, "UID COPY 7 \"Archive\"");
        ok(&mut s, &tag);
        candidates(&mut s, "42");
        fetch(&mut s, 42, RAW);
        select(&mut s, "SELECT \"INBOX\"", 457);
        logout(&mut s);
    });
    let account = config(port);
    let ImapChange::Uncertain(reason) =
        network::imap_move(&account, &location(&account, "INBOX", 456, 7), "Archive").unwrap()
    else {
        panic!("A new source epoch must not flag a reused UID");
    };
    assert!(reason.contains("original mailbox identity changed"));
    worker.join().unwrap();
}
#[test]
fn missing_source_uid_aborts_before_any_copy() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1 MOVE");
        select(&mut s, "SELECT \"INBOX\"", 456);
        let tag = expect(&mut s, "STATUS \"Archive\" (UIDVALIDITY UIDNEXT)");
        send(
            &mut s,
            "* STATUS \"Archive\" (UIDNEXT 42 UIDVALIDITY 999)\r\n",
        );
        ok(&mut s, &tag);
        let tag = expect(&mut s, "UID FETCH 7 (UID BODY.PEEK[])");
        ok(&mut s, &tag);
        assert_eq!(
            read(&mut s),
            "R9999 LOGOUT",
            "Missing source must never COPY/MOVE"
        );
    });
    let account = config(port);
    assert!(
        network::imap_move(&account, &location(&account, "INBOX", 456, 7), "Archive")
            .unwrap_err()
            .contains("disappeared")
    );
    worker.join().unwrap();
}
#[test]
fn lost_copy_acceptance_blocks_a_second_copy_even_without_uidplus() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1");
        preflight(&mut s);
        let _ = expect(&mut s, "UID COPY 7 \"Archive\""); // Connection drops at acceptance point.
    });
    let account = config(port);
    assert!(matches!(
        network::imap_move(&account, &location(&account, "INBOX", 456, 7), "Archive"),
        Ok(ImapChange::Uncertain(_))
    ));
    worker.join().unwrap();
}
#[test]
fn rejected_move_is_conservatively_uncertain_per_rfc6851() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1 UIDPLUS MOVE");
        select(&mut s, "SELECT \"INBOX\"", 456);
        let tag = expect(&mut s, "UID MOVE 7 \"Archive\"");
        send(
            &mut s,
            &format!("{tag} NO move may have partially completed\r\n"),
        );
        logout(&mut s);
    });
    let account = config(port);
    assert!(matches!(
        network::imap_move(&account, &location(&account, "INBOX", 456, 7), "Archive"),
        Ok(ImapChange::Uncertain(_))
    ));
    worker.join().unwrap();
}
#[test]
fn imap4rev2_includes_move_and_targeted_expunge_without_extension_tokens() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev2");
        select(&mut s, "SELECT \"INBOX\"", 456);
        let tag = expect(&mut s, "UID MOVE 7 \"Archive\"");
        send(&mut s, &format!("{tag} OK [COPYUID 999 7 42] moved\r\n"));
        logout(&mut s);
    });
    let account = config(port);
    assert!(matches!(
        network::imap_move(&account, &location(&account, "INBOX", 456, 7), "Archive"),
        Ok(ImapChange::Moved(_))
    ));
    worker.join().unwrap();
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev2");
        select(&mut s, "SELECT \"Trash\"", 999);
        let tag = expect(&mut s, "UID STORE 42 +FLAGS.SILENT (\\Deleted)");
        ok(&mut s, &tag);
        let tag = expect(&mut s, "UID EXPUNGE 42");
        ok(&mut s, &tag);
        logout(&mut s);
    });
    let account = config(port);
    assert!(matches!(
        network::imap_permanently_delete(&account, &location(&account, "Trash", 999, 42)),
        Ok(ImapChange::Deleted)
    ));
    worker.join().unwrap();
}
#[test]
fn deleting_to_trash_creates_a_missing_dedicated_mailbox() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1 UIDPLUS MOVE");
        let tag = expect(&mut s, "LIST \"\" \"*\"");
        send(&mut s, "* LIST () \"/\" \"INBOX\"\r\n");
        ok(&mut s, &tag);
        let tag = expect(&mut s, "CREATE \"Trash\"");
        ok(&mut s, &tag);
        select(&mut s, "SELECT \"INBOX\"", 456);
        let tag = expect(&mut s, "UID MOVE 7 \"Trash\"");
        send(&mut s, &format!("{tag} OK [COPYUID 999 7 42] moved\r\n"));
        logout(&mut s);
    });
    let account = config(port);
    let ImapChange::Moved(destination) =
        network::imap_trash(&account, &location(&account, "INBOX", 456, 7)).unwrap()
    else {
        panic!("Expected newly created Trash destination");
    };
    assert_eq!(destination.mailbox, "Trash");
    worker.join().unwrap();
}
#[test]
fn restore_clears_deleted_before_moving_out_of_pending_trash() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1 UIDPLUS MOVE");
        select(&mut s, "SELECT \"Trash\"", 999);
        let tag = expect(&mut s, "UID STORE 42 -FLAGS.SILENT (\\Deleted)");
        ok(&mut s, &tag);
        let tag = expect(&mut s, "UID MOVE 42 \"INBOX\"");
        send(&mut s, &format!("{tag} OK [COPYUID 456 42 7] restored\r\n"));
        logout(&mut s);
    });
    let account = config(port);
    let ImapChange::Moved(destination) =
        network::imap_restore(&account, &location(&account, "Trash", 999, 42), "INBOX").unwrap()
    else {
        panic!("Restored message must survive server Deleted filtering");
    };
    assert_eq!(destination, location(&account, "INBOX", 456, 7));
    worker.join().unwrap();
}
#[test]
fn lack_of_uidnext_aborts_without_a_mutation_or_guessed_destination() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1");
        select(&mut s, "SELECT \"INBOX\"", 456);
        let tag = expect(&mut s, "STATUS \"Archive\" (UIDVALIDITY UIDNEXT)");
        send(&mut s, "* STATUS \"Archive\" (UIDVALIDITY 999)\r\n");
        ok(&mut s, &tag);
        assert_eq!(
            read(&mut s),
            "R9999 LOGOUT",
            "No trustworthy preflight means no mutation"
        );
    });
    let account = config(port);
    assert!(
        network::imap_move(&account, &location(&account, "INBOX", 456, 7), "Archive")
            .unwrap_err()
            .contains("UIDNEXT")
    );
    worker.join().unwrap();
}

#[test]
fn destination_scan_is_bounded_before_downloading_concurrent_arrivals() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1");
        preflight(&mut s);
        let tag = expect(&mut s, "UID COPY 7 \"Archive\"");
        ok(&mut s, &tag);
        let values = (42..107)
            .map(|uid| uid.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        candidates(&mut s, &values);
        logout(&mut s);
    });
    let account = config(port);
    let ImapChange::Uncertain(reason) =
        network::imap_move(&account, &location(&account, "INBOX", 456, 7), "Archive").unwrap()
    else {
        panic!("A bounded proof cannot guess the destination amid arbitrary arrivals");
    };
    assert!(reason.contains("Too many concurrent arrivals"));
    worker.join().unwrap();
}
#[test]
fn lost_deleted_flag_acceptance_is_uncertain_and_never_expunges_unrelated_uids() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1");
        select(&mut s, "SELECT \"Trash\"", 999);
        let _ = expect(&mut s, "UID STORE 42 +FLAGS.SILENT (\\Deleted)");
    });
    let account = config(port);
    assert!(matches!(
        network::imap_permanently_delete(&account, &location(&account, "Trash", 999, 42)),
        Ok(ImapChange::Uncertain(_))
    ));
    worker.join().unwrap();
}
#[test]
fn readonly_source_refuses_moves_before_any_uid_mutation() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = connected(listener, "IMAP4rev1 UIDPLUS MOVE");
        let tag = expect(&mut s, "SELECT \"INBOX\"");
        send(
            &mut s,
            &format!("* OK [UIDVALIDITY 456] epoch\r\n{tag} OK [READ-ONLY] selected\r\n"),
        );
        assert_eq!(read(&mut s), "");
    });
    let account = config(port);
    assert!(
        network::imap_move(&account, &location(&account, "INBOX", 456, 7), "Archive")
            .unwrap_err()
            .contains("read-only")
    );
    worker.join().unwrap();
}
