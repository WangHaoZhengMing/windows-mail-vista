use mail_core::{
    account::{Account, Incoming, Security},
    message::Draft,
    network,
};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

fn account(port: u16, incoming: Incoming) -> Account {
    let mut account = Account {
        email: "sender@example.com".into(),
        username: "user".into(),
        auth: Default::default(),
        password_dpapi: String::new(),
        incoming,
        incoming_host: "127.0.0.1".into(),
        incoming_port: port,
        incoming_security: Security::Plain,
        smtp_host: "127.0.0.1".into(),
        smtp_port: port,
        smtp_security: Security::Plain,
    };
    account.set_password("test-pass").unwrap();
    account
}
fn server() -> (TcpListener, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}
fn client(listener: TcpListener) -> BufReader<TcpStream> {
    let (stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    BufReader::new(stream)
}
fn line(s: &mut BufReader<TcpStream>) -> String {
    let mut out = String::new();
    s.read_line(&mut out).unwrap();
    out.trim_end_matches(['\r', '\n']).into()
}
fn reply(s: &mut BufReader<TcpStream>, text: &str) {
    s.get_mut().write_all(text.as_bytes()).unwrap();
}

#[test]
fn pop3_completed_download_survives_cancel_and_restart_without_retr_again() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        for session in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut s = BufReader::new(stream);
            reply(&mut s, "+OK fixture\r\n");
            assert_eq!(line(&mut s), "USER user");
            reply(&mut s, "+OK\r\n");
            assert_eq!(line(&mut s), "PASS test-pass");
            reply(&mut s, "+OK\r\n");
            assert_eq!(line(&mut s), "UIDL");
            reply(&mut s, "+OK\r\n1 first\r\n2 second\r\n.\r\n");
            assert_eq!(line(&mut s), if session == 0 { "RETR 1" } else { "RETR 2" });
            reply(
                &mut s,
                "+OK\r\nFrom: a@example.test\r\nSubject: cached\r\n\r\nbody\r\n.\r\n",
            );
            if session == 0 {
                assert_eq!(line(&mut s), "");
            } else {
                assert_eq!(line(&mut s), "QUIT");
                reply(&mut s, "+OK\r\n");
            }
        }
    });
    let root = std::env::temp_dir().join(format!(
        "winmail-pop-checkpoint-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let account = account(port, Incoming::Pop3);
    let control = network::SyncControl::new(root.clone());
    let cancel = control.clone();
    assert!(
        network::receive_controlled(
            &account,
            &[],
            &mut |message| {
                if message.starts_with("Downloaded 1 ") {
                    cancel.cancel();
                }
            },
            &control
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    let resumed = network::SyncControl::new(root.clone());
    let downloads = network::receive_controlled(&account, &[], &mut |_| {}, &resumed).unwrap();
    assert_eq!(downloads.len(), 2);
    worker.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn smtp_auth_multiline_bcc_and_dot_transparency() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        reply(&mut s, "220 fixture\r\n");
        assert!(line(&mut s).starts_with("EHLO "));
        reply(&mut s, "250-fixture\r\n250 AUTH PLAIN LOGIN\r\n");
        assert!(line(&mut s).starts_with("AUTH PLAIN "));
        reply(&mut s, "235 authenticated\r\n");
        assert_eq!(line(&mut s), "MAIL FROM:<sender@example.com>");
        reply(&mut s, "250 OK\r\n");
        let mut recipients = Vec::new();
        loop {
            let command = line(&mut s);
            if command == "DATA" {
                break;
            }
            assert!(command.starts_with("RCPT TO:"));
            recipients.push(command);
            reply(&mut s, "250 OK\r\n");
        }
        assert!(recipients.contains(&"RCPT TO:<hidden@example.com>".into()));
        reply(&mut s, "354 proceed\r\n");
        let mut data = Vec::new();
        loop {
            let value = line(&mut s);
            if value == "." {
                break;
            }
            data.push(value);
        }
        assert!(data.contains(&"..dot".into()));
        assert!(!data.iter().any(|l| l.contains("hidden@example.com")));
        reply(&mut s, "250 accepted\r\n");
        assert_eq!(line(&mut s), "QUIT");
        // Close without 221: message has already been accepted, so no resend/error.
    });
    let account = account(port, Incoming::Imap);
    let draft = Draft {
        to: "to@example.com".into(),
        bcc: "hidden@example.com".into(),
        ..Draft::default()
    };
    network::smtp_send(&account, &draft, b"Subject: test\r\n\r\n.dot\r\n").unwrap();
    worker.join().unwrap();
}

#[test]
fn smtp_rejected_recipient_never_submits_data() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        reply(&mut s, "220 fixture\r\n");
        line(&mut s);
        reply(&mut s, "250 AUTH PLAIN\r\n");
        line(&mut s);
        reply(&mut s, "235 OK\r\n");
        assert!(line(&mut s).starts_with("MAIL FROM:"));
        reply(&mut s, "250 OK\r\n");
        assert!(line(&mut s).starts_with("RCPT TO:"));
        reply(&mut s, "550 unknown\r\n");
        let mut remaining = String::new();
        assert_eq!(s.read_line(&mut remaining).unwrap(), 0);
    });
    let draft = Draft {
        to: "to@example.com".into(),
        ..Draft::default()
    };
    assert!(
        network::smtp_send(
            &account(port, Incoming::Imap),
            &draft,
            b"Subject: x\r\n\r\nbody"
        )
        .is_err()
    );
    worker.join().unwrap();
}

#[test]
fn pop3_uid_dedup_retains_binary_octets_and_does_not_delete() {
    let (listener, port) = server();
    let account = account(port, Incoming::Pop3);
    let key = format!("{}:pop3:old", account.id());
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        reply(&mut s, "+OK fixture\r\n");
        assert_eq!(line(&mut s), "USER user");
        reply(&mut s, "+OK\r\n");
        assert_eq!(line(&mut s), "PASS test-pass");
        reply(&mut s, "+OK\r\n");
        assert_eq!(line(&mut s), "UIDL");
        reply(&mut s, "+OK\r\n1 old\r\n2 new\r\n.\r\n");
        assert_eq!(line(&mut s), "RETR 2");
        reply(&mut s, "+OK\r\nSubject: binary\r\n\r\n..dot\r\n");
        s.get_mut().write_all(b"\xff\x80\r\n.\r\n").unwrap();
        assert_eq!(line(&mut s), "QUIT");
        reply(&mut s, "+OK\r\n");
    });
    let download = network::receive(&account, &[key]).unwrap();
    assert_eq!(download.len(), 1);
    assert_eq!(
        download[0].raw,
        b"Subject: binary\r\n\r\n.dot\r\n\xff\x80\r\n"
    );
    worker.join().unwrap();
}

#[test]
fn imap_uidvalidity_peek_literal_and_dedup() {
    let (listener, port) = server();
    let account = account(port, Incoming::Imap);
    let key = format!("{}:imap:456:8", account.id());
    let raw = b"Subject: imap\r\n\r\nbody\xff\r\n";
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        reply(&mut s, "* OK fixture\r\n");
        assert_eq!(line(&mut s), "R0001 LOGIN \"user\" \"test-pass\"");
        reply(&mut s, "R0001 OK logged in\r\n");
        assert_eq!(line(&mut s), "R0002 SELECT INBOX");
        reply(
            &mut s,
            "* 2 EXISTS\r\n* OK [UIDVALIDITY 456] epoch\r\nR0002 OK selected\r\n",
        );
        assert_eq!(line(&mut s), "R0003 UID SEARCH ALL");
        reply(&mut s, "* SEARCH 7 8\r\nR0003 OK search\r\n");
        assert_eq!(line(&mut s), "R0004 UID FETCH 7 (BODY.PEEK[])");
        reply(
            &mut s,
            &format!("* 1 FETCH (UID 7 BODY[] {{{}}}\r\n", raw.len()),
        );
        s.get_mut().write_all(raw).unwrap();
        reply(&mut s, ")\r\nR0004 OK fetched\r\n");
        assert_eq!(line(&mut s), "R9999 LOGOUT");
    });
    let download = network::receive(&account, &[key]).unwrap();
    assert_eq!(download.len(), 1);
    assert_eq!(download[0].raw, raw);
    assert!(download[0].key.ends_with(":456:7"));
    worker.join().unwrap();
}

#[test]
fn smtp_requires_starttls_before_authentication() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        reply(&mut s, "220 fixture\r\n");
        assert!(line(&mut s).starts_with("EHLO "));
        reply(&mut s, "250 AUTH PLAIN\r\n");
        let mut command = String::new();
        assert_eq!(s.read_line(&mut command).unwrap(), 0);
    });
    let mut account = account(port, Incoming::Imap);
    account.smtp_security = Security::StartTls;
    let draft = Draft {
        to: "to@example.com".into(),
        ..Draft::default()
    };
    assert!(
        network::smtp_send(&account, &draft, b"Subject: x\r\n\r\nbody")
            .unwrap_err()
            .contains("STARTTLS")
    );
    worker.join().unwrap();
}

#[test]
fn tls_failure_never_falls_back_to_plaintext() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        use std::io::Read;
        let mut first = [0u8; 1];
        s.read_exact(&mut first).unwrap();
        assert_eq!(
            first[0], 0x16,
            "Expected a TLS handshake, not a mail command"
        );
        // Close during handshake. The client must fail without a plaintext reconnect.
    });
    let mut account = account(port, Incoming::Imap);
    account.smtp_security = Security::Tls;
    let draft = Draft {
        to: "to@example.com".into(),
        ..Draft::default()
    };
    assert!(
        network::smtp_send(&account, &draft, b"Subject: x\r\n\r\nbody")
            .unwrap_err()
            .contains("TLS handshake")
    );
    worker.join().unwrap();
}

fn imap_login_capabilities(s: &mut BufReader<TcpStream>, capabilities: &str) {
    reply(s, "* OK fixture\r\n");
    assert_eq!(line(s), "R0001 LOGIN \"user\" \"test-pass\"");
    reply(s, "R0001 OK logged in\r\n");
    assert_eq!(line(s), "R0002 CAPABILITY");
    reply(
        s,
        &format!("* CAPABILITY IMAP4rev1 {capabilities}\r\nR0002 OK capabilities\r\n"),
    );
}
fn clear_deleted(s: &mut BufReader<TcpStream>, tag: u32, uid: u32) {
    assert_eq!(
        line(s),
        format!("R{tag:04} UID STORE {uid} -FLAGS.SILENT (\\Deleted)")
    );
    reply(s, &format!("R{tag:04} OK restored flag\r\n"));
}
fn location(
    account: &Account,
    mailbox: &str,
    uid_validity: u32,
    uid: u32,
) -> network::ImapLocation {
    network::ImapLocation {
        connection: account.id(),
        mailbox: mailbox.into(),
        uid_validity,
        uid,
    }
}

#[test]
fn imap_trash_uses_special_use_move_and_retains_destination_uid() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        imap_login_capabilities(&mut s, "UIDPLUS MOVE");
        assert_eq!(line(&mut s), "R0003 LIST \"\" \"*\"");
        reply(
            &mut s,
            "* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n* LIST (\\Trash) \"/\" \"Deleted Items\"\r\nR0003 OK listed\r\n",
        );
        assert_eq!(line(&mut s), "R0004 SELECT \"INBOX\"");
        reply(
            &mut s,
            "* OK [UIDVALIDITY 456] epoch\r\nR0004 OK [READ-WRITE] selected\r\n",
        );
        assert_eq!(line(&mut s), "R0005 UID MOVE 7 \"Deleted Items\"");
        reply(
            &mut s,
            "* OK [COPYUID 999 7 42] moved\r\nR0005 OK moved\r\n",
        );
        assert_eq!(line(&mut s), "R9999 LOGOUT");
    });
    let account = account(port, Incoming::Imap);
    let network::ImapChange::Moved(moved) =
        network::imap_trash(&account, &location(&account, "INBOX", 456, 7)).unwrap()
    else {
        panic!("Expected a known destination UID");
    };
    assert_eq!(moved.mailbox, "Deleted Items");
    assert_eq!((moved.uid_validity, moved.uid), (999, 42));
    worker.join().unwrap();
}

#[test]
fn imap_restore_copy_fallback_expunges_only_the_copied_uid() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        imap_login_capabilities(&mut s, "UIDPLUS");
        assert_eq!(line(&mut s), "R0003 SELECT \"Trash\"");
        reply(
            &mut s,
            "* OK [UIDVALIDITY 999] epoch\r\nR0003 OK selected\r\n",
        );
        clear_deleted(&mut s, 4, 42);
        assert_eq!(line(&mut s), "R0005 UID COPY 42 \"INBOX\"");
        reply(&mut s, "R0005 OK [COPYUID 456 42 17] copied\r\n");
        assert_eq!(line(&mut s), "R0006 UID STORE 42 +FLAGS.SILENT (\\Deleted)");
        reply(&mut s, "R0006 OK marked\r\n");
        assert_eq!(line(&mut s), "R0007 UID EXPUNGE 42");
        reply(&mut s, "* 3 EXPUNGE\r\nR0007 OK expunged\r\n");
        assert_eq!(line(&mut s), "R9999 LOGOUT");
    });
    let account = account(port, Incoming::Imap);
    let network::ImapChange::Moved(moved) =
        network::imap_restore(&account, &location(&account, "Trash", 999, 42), "INBOX").unwrap()
    else {
        panic!("Expected a known restored UID");
    };
    assert_eq!(moved, location(&account, "INBOX", 456, 17));
    worker.join().unwrap();
}

#[test]
fn imap_permanent_delete_requires_uid_expunge_and_never_uses_global_expunge() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        imap_login_capabilities(&mut s, "");
        assert_eq!(line(&mut s), "R0003 SELECT \"Trash\"");
        reply(
            &mut s,
            "* OK [UIDVALIDITY 999] epoch\r\nR0003 OK selected\r\n",
        );
        assert_eq!(line(&mut s), "R0004 UID STORE 42 +FLAGS.SILENT (\\Deleted)");
        reply(&mut s, "R0004 OK marked\r\n");
        assert_eq!(
            line(&mut s),
            "R9999 LOGOUT",
            "No global EXPUNGE or CLOSE is allowed"
        );
    });
    let config = account(port, Incoming::Imap);
    let original = location(&config, "Trash", 999, 42);
    let network::ImapChange::MarkedDeleted(pending) =
        network::imap_permanently_delete(&config, &original).unwrap()
    else {
        panic!("A flag is pending compaction, not permanently erased");
    };
    assert_eq!(pending, original);
    worker.join().unwrap();

    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        imap_login_capabilities(&mut s, "UIDPLUS");
        assert_eq!(line(&mut s), "R0003 SELECT \"Trash\"");
        reply(
            &mut s,
            "* OK [UIDVALIDITY 999] epoch\r\nR0003 OK selected\r\n",
        );
        assert_eq!(line(&mut s), "R0004 UID STORE 42 +FLAGS.SILENT (\\Deleted)");
        reply(&mut s, "R0004 OK marked\r\n");
        assert_eq!(line(&mut s), "R0005 UID EXPUNGE 42");
        reply(&mut s, "R0005 OK expunged\r\n");
        assert_eq!(line(&mut s), "R9999 LOGOUT");
    });
    let account = account(port, Incoming::Imap);
    network::imap_permanently_delete(&account, &location(&account, "Trash", 999, 42)).unwrap();
    worker.join().unwrap();
}

#[test]
fn imap_changed_uidvalidity_refuses_to_touch_a_reused_uid() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        imap_login_capabilities(&mut s, "UIDPLUS MOVE");
        assert_eq!(line(&mut s), "R0003 SELECT \"Trash\"");
        reply(
            &mut s,
            "* OK [UIDVALIDITY 1000] new epoch\r\nR0003 OK selected\r\n",
        );
        assert_eq!(
            line(&mut s),
            "",
            "UIDVALIDITY mismatch must send no mutation"
        );
    });
    let account = account(port, Incoming::Imap);
    assert!(
        network::imap_restore(&account, &location(&account, "Trash", 999, 42), "INBOX")
            .unwrap_err()
            .contains("UIDVALIDITY changed")
    );
    worker.join().unwrap();
}

#[test]
fn imap_copy_without_destination_identity_never_marks_source_deleted() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        imap_login_capabilities(&mut s, "UIDPLUS");
        assert_eq!(line(&mut s), "R0003 SELECT \"Trash\"");
        reply(
            &mut s,
            "* OK [UIDVALIDITY 999] epoch\r\nR0003 OK selected\r\n",
        );
        clear_deleted(&mut s, 4, 42);
        assert_eq!(line(&mut s), "R0005 UID COPY 42 \"INBOX\"");
        reply(&mut s, "R0005 OK copied without COPYUID\r\n");
        assert_eq!(line(&mut s), "R9999 LOGOUT");
    });
    let account = account(port, Incoming::Imap);
    let network::ImapChange::Uncertain(reason) =
        network::imap_restore(&account, &location(&account, "Trash", 999, 42), "INBOX").unwrap()
    else {
        panic!("Completed COPY without its UID must block a duplicate retry");
    };
    assert!(reason.contains("COPYUID"));
    assert!(reason.contains("original was not marked deleted"));
    worker.join().unwrap();
}

#[test]
fn imap_nonpersistent_uids_are_never_used_for_remote_mutations() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        imap_login_capabilities(&mut s, "UIDPLUS MOVE");
        assert_eq!(line(&mut s), "R0003 SELECT \"Trash\"");
        reply(
            &mut s,
            "* OK [UIDVALIDITY 999] epoch\r\n* NO [UIDNOTSTICKY] ephemeral\r\nR0003 OK selected\r\n",
        );
        assert_eq!(
            line(&mut s),
            "",
            "Ephemeral UIDs must never be moved or deleted"
        );
    });
    let account = account(port, Incoming::Imap);
    assert!(
        network::imap_restore(&account, &location(&account, "Trash", 999, 42), "INBOX")
            .unwrap_err()
            .contains("not persistent")
    );
    worker.join().unwrap();
}

#[test]
fn imap_completed_move_without_copyuid_reports_uncertain_server_state() {
    let (listener, port) = server();
    let worker = thread::spawn(move || {
        let mut s = client(listener);
        imap_login_capabilities(&mut s, "UIDPLUS MOVE");
        assert_eq!(line(&mut s), "R0003 SELECT \"Trash\"");
        reply(
            &mut s,
            "* OK [UIDVALIDITY 999] epoch\r\nR0003 OK selected\r\n",
        );
        clear_deleted(&mut s, 4, 42);
        assert_eq!(line(&mut s), "R0005 UID MOVE 42 \"INBOX\"");
        reply(&mut s, "R0005 OK completed but omitted COPYUID\r\n");
        assert_eq!(line(&mut s), "R9999 LOGOUT");
    });
    let account = account(port, Incoming::Imap);
    let network::ImapChange::Uncertain(reason) =
        network::imap_restore(&account, &location(&account, "Trash", 999, 42), "INBOX").unwrap()
    else {
        panic!("Missing destination identity after an acknowledged MOVE is uncertain");
    };
    assert!(reason.contains("completed move"));
    assert!(reason.contains("destination UID"));
    worker.join().unwrap();
}

#[test]
fn imap_lost_expunge_reply_is_uncertain_but_explicit_rejection_remains_pending() {
    for reject in [false, true] {
        let (listener, port) = server();
        let worker = thread::spawn(move || {
            let mut s = client(listener);
            imap_login_capabilities(&mut s, "UIDPLUS");
            assert_eq!(line(&mut s), "R0003 SELECT \"Trash\"");
            reply(
                &mut s,
                "* OK [UIDVALIDITY 999] epoch\r\nR0003 OK selected\r\n",
            );
            assert_eq!(line(&mut s), "R0004 UID STORE 42 +FLAGS.SILENT (\\Deleted)");
            reply(&mut s, "R0004 OK marked\r\n");
            assert_eq!(line(&mut s), "R0005 UID EXPUNGE 42");
            if reject {
                reply(&mut s, "R0005 NO rejected\r\n");
                assert_eq!(line(&mut s), "R9999 LOGOUT");
            }
            // Otherwise drop the connection at the acceptance point.
        });
        let account = account(port, Incoming::Imap);
        let result =
            network::imap_permanently_delete(&account, &location(&account, "Trash", 999, 42));
        if reject {
            assert!(matches!(result, Ok(network::ImapChange::MarkedDeleted(_))));
        } else {
            assert!(matches!(result, Ok(network::ImapChange::Uncertain(_))));
        }
        worker.join().unwrap();
    }
}

#[test]
fn imap_message_from_earlier_connection_settings_cannot_mutate_new_server() {
    let (listener, port) = server();
    listener.set_nonblocking(true).unwrap();
    let mut account = account(port, Incoming::Imap);
    let old = location(&account, "Trash", 999, 42);
    account.email = "changed@example.com".into();
    assert!(
        network::imap_restore(&account, &old, "INBOX")
            .unwrap_err()
            .contains("earlier server settings")
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "Old message ownership must be rejected before connecting to the new server"
    );
}

#[test]
fn imap_copy_lost_response_is_uncertain_but_tagged_rejection_is_retryable() {
    for reject in [false, true] {
        let (listener, port) = server();
        let worker = thread::spawn(move || {
            let mut s = client(listener);
            imap_login_capabilities(&mut s, "UIDPLUS");
            assert_eq!(line(&mut s), "R0003 SELECT \"Trash\"");
            reply(
                &mut s,
                "* OK [UIDVALIDITY 999] epoch\r\nR0003 OK selected\r\n",
            );
            clear_deleted(&mut s, 4, 42);
            assert_eq!(line(&mut s), "R0005 UID COPY 42 \"INBOX\"");
            if reject {
                reply(&mut s, "R0005 NO rejected\r\n");
                assert_eq!(
                    line(&mut s),
                    "R9999 LOGOUT",
                    "Rejected COPY must not mark or expunge the source"
                );
            }
        });
        let account = account(port, Incoming::Imap);
        let result =
            network::imap_restore(&account, &location(&account, "Trash", 999, 42), "INBOX");
        if reject {
            assert!(result.unwrap_err().contains("rejected"));
        } else {
            let Ok(network::ImapChange::Uncertain(reason)) = result else {
                panic!("Lost COPY acceptance must block duplicate retries");
            };
            assert!(reason.contains("may already have copied"));
            assert!(reason.contains("move did not complete"));
        }
        worker.join().unwrap();
    }
}

#[test]
fn imap_copy_fallback_failures_after_copy_block_duplicate_retries() {
    #[derive(Debug, Clone, Copy)]
    enum Failure {
        StoreRejected,
        StoreDisconnected,
        ExpungeRejected,
    }
    for failure in [
        Failure::StoreRejected,
        Failure::StoreDisconnected,
        Failure::ExpungeRejected,
    ] {
        let (listener, port) = server();
        let worker = thread::spawn(move || {
            let mut s = client(listener);
            imap_login_capabilities(&mut s, "UIDPLUS");
            assert_eq!(line(&mut s), "R0003 SELECT \"Trash\"");
            reply(
                &mut s,
                "* OK [UIDVALIDITY 999] epoch\r\nR0003 OK selected\r\n",
            );
            clear_deleted(&mut s, 4, 42);
            assert_eq!(line(&mut s), "R0005 UID COPY 42 \"INBOX\"");
            reply(&mut s, "R0005 OK [COPYUID 456 42 17] copied\r\n");
            assert_eq!(line(&mut s), "R0006 UID STORE 42 +FLAGS.SILENT (\\Deleted)");
            match failure {
                Failure::StoreDisconnected => return,
                Failure::StoreRejected => reply(&mut s, "R0006 NO rejected\r\n"),
                Failure::ExpungeRejected => {
                    reply(&mut s, "R0006 OK marked\r\n");
                    assert_eq!(line(&mut s), "R0007 UID EXPUNGE 42");
                    reply(&mut s, "R0007 NO rejected\r\n");
                }
            }
            assert_eq!(line(&mut s), "R9999 LOGOUT");
        });
        let account = account(port, Incoming::Imap);
        let outcome =
            network::imap_restore(&account, &location(&account, "Trash", 999, 42), "INBOX")
                .unwrap();
        if matches!(failure, Failure::ExpungeRejected) {
            let network::ImapChange::MovedDeferred {
                destination,
                original,
            } = outcome
            else {
                panic!("Known copy with marked source must retain deferred metadata");
            };
            assert_eq!(destination, location(&account, "INBOX", 456, 17));
            assert_eq!(original, location(&account, "Trash", 999, 42));
        } else {
            let network::ImapChange::Uncertain(reason) = outcome else {
                panic!("Existing destination copy must block retries after {failure:?}");
            };
            assert!(reason.contains("Destination copy exists"));
            assert!(reason.contains("move did not complete"));
            assert!(reason.contains("avoid duplicate copies"));
        }
        worker.join().unwrap();
    }
}
