use super::*;
use crate::{
    account::{Authentication, Incoming, Security, protect},
    oauth::{OAuthAccount, OAuthConfig, OAuthTokens},
};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const TOKEN: &str = "synthetic-oauth-token";
fn account() -> Account {
    let protected = protect(
        serde_json::json!({"access_token":TOKEN,"refresh_token":"synthetic-refresh"})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    Account {
        auth: Authentication::OAuth2(OAuthAccount {
            config: OAuthConfig::google("owned-fixture-client"),
            tokens: Some(OAuthTokens {
                protected,
                expires_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 3600,
            }),
        }),
        email: "reader@example.test".into(),
        username: "reader@example.test".into(),
        password_dpapi: String::new(),
        incoming: Incoming::Imap,
        incoming_host: "imap.example.test".into(),
        incoming_port: 993,
        incoming_security: Security::Tls,
        smtp_host: "smtp.example.test".into(),
        smtp_port: 465,
        smtp_security: Security::Tls,
    }
}
fn fixture(
    script: impl FnOnce(BufReader<TcpStream>) + Send + 'static,
) -> (Session, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        script(BufReader::new(stream));
    });
    // Exercise the private SASL state machines over loopback. Public mail APIs
    // separately validate the account and reject plaintext OAuth transports.
    let session = Session::connect("127.0.0.1", port, Security::Plain).unwrap();
    (session, thread)
}
fn line(io: &mut BufReader<TcpStream>) -> String {
    let mut text = String::new();
    io.read_line(&mut text).unwrap();
    text.trim_end_matches(['\r', '\n']).to_owned()
}
fn write(io: &mut BufReader<TcpStream>, text: &str) {
    io.get_mut().write_all(text.as_bytes()).unwrap();
    io.get_mut().flush().unwrap();
}
fn closed(io: &mut BufReader<TcpStream>) {
    let mut text = String::new();
    assert_eq!(
        io.read_line(&mut text).unwrap(),
        0,
        "Unexpected command after failed authentication: {text}"
    );
}
fn expected() -> String {
    oauth::xoauth2_response("reader@example.test", TOKEN).unwrap()
}

#[test]
#[cfg(windows)]
fn smtp_xoauth2_success_and_error_challenge_are_fully_drained_without_fallback() {
    for succeeds in [true, false] {
        let (mut session, server) = fixture(move |mut io| {
            assert_eq!(line(&mut io), format!("AUTH XOAUTH2 {}", expected()));
            if succeeds {
                write(&mut io, "235 2.7.0 authenticated\r\n");
            } else {
                write(&mut io, "334 eyJzdGF0dXMiOiI0MDEifQ==\r\n");
                assert_eq!(line(&mut io), "");
                write(&mut io, "535 synthetic-oauth-token rejected\r\n");
            }
            closed(&mut io);
        });
        let result = smtp(
            &mut session,
            &account(),
            &[
                "250-AUTH XOAUTH2 PLAIN LOGIN".into(),
                "250 SIZE 50000000".into(),
            ],
        );
        if succeeds {
            result.unwrap();
        } else {
            let error = result.err().unwrap();
            assert!(error.contains("authentication failed"));
            assert!(!error.contains(TOKEN));
        }
        drop(session);
        server.join().unwrap();
    }
}
#[test]
#[cfg(windows)]
fn missing_smtp_xoauth2_never_sends_a_password_or_token() {
    let (mut session, server) = fixture(|mut io| closed(&mut io));
    let error = smtp(
        &mut session,
        &account(),
        &[
            "250-AUTH PLAIN LOGIN".into(),
            "250 X-XOAUTH2-DIAGNOSTIC".into(),
        ],
    )
    .err()
    .unwrap();
    assert!(error.contains("does not advertise"));
    drop(session);
    server.join().unwrap();
}
#[test]
#[cfg(windows)]
fn pop3_xoauth2_handles_capabilities_continuations_and_error_completion() {
    for (capa_supported, succeeds) in [(true, true), (false, true), (true, false)] {
        let (mut session, server) = fixture(move |mut io| {
            assert_eq!(line(&mut io), "CAPA");
            if capa_supported {
                write(
                    &mut io,
                    "+OK capabilities\r\nSASL XOAUTH2 PLAIN\r\nUIDL\r\n.\r\n",
                );
            } else {
                write(&mut io, "-ERR CAPA unavailable\r\n");
            }
            assert_eq!(line(&mut io), "AUTH XOAUTH2");
            write(&mut io, "+ \r\n");
            assert_eq!(line(&mut io), expected());
            if succeeds {
                write(&mut io, "+OK authenticated\r\n");
            } else {
                write(&mut io, "+ eyJzdGF0dXMiOiI0MDEifQ==\r\n");
                assert_eq!(line(&mut io), "");
                write(&mut io, "-ERR synthetic-oauth-token rejected\r\n");
            }
            closed(&mut io);
        });
        let result = pop3(&mut session, &account());
        if succeeds {
            result.unwrap();
        } else {
            let error = result.err().unwrap();
            assert!(!error.contains(TOKEN));
            assert!(error.contains("authentication failed"));
        }
        drop(session);
        server.join().unwrap();
    }
}
#[test]
#[cfg(windows)]
fn missing_pop3_xoauth2_is_an_error_without_user_pass_fallback() {
    let (mut session, server) = fixture(|mut io| {
        assert_eq!(line(&mut io), "CAPA");
        write(&mut io, "+OK\r\nSASL PLAIN LOGIN\r\n.\r\n");
        closed(&mut io);
    });
    let error = pop3(&mut session, &account()).err().unwrap();
    assert!(error.contains("does not advertise"));
    drop(session);
    server.join().unwrap();
}
#[test]
#[cfg(windows)]
fn imap_xoauth2_supports_sasl_ir_and_continuations_without_consuming_the_next_tag() {
    for ir in [true, false] {
        let (session, server) = fixture(move |mut io| {
            assert_eq!(line(&mut io), "R0001 CAPABILITY");
            write(
                &mut io,
                &format!(
                    "* CAPABILITY IMAP4rev1 AUTH=XOAUTH2{}\r\nR0001 OK capabilities\r\n",
                    if ir { " SASL-IR" } else { "" }
                ),
            );
            if ir {
                assert_eq!(
                    line(&mut io),
                    format!("R0002 AUTHENTICATE XOAUTH2 {}", expected())
                );
            } else {
                assert_eq!(line(&mut io), "R0002 AUTHENTICATE XOAUTH2");
                write(&mut io, "+ \r\n");
                assert_eq!(line(&mut io), expected());
            }
            write(
                &mut io,
                "* OK authentication progress\r\nR0002 OK authenticated\r\n",
            );
            assert_eq!(line(&mut io), "R0003 NOOP");
            write(&mut io, "R0003 OK noop\r\n");
            closed(&mut io);
        });
        let mut session = Imap { session, tag: 0 };
        imap(&mut session, &account()).unwrap();
        assert_eq!(session.tag, 2);
        assert!(session.command("NOOP").is_ok());
        drop(session);
        server.join().unwrap();
    }
}
#[test]
#[cfg(windows)]
fn imap_xoauth2_rejection_sends_empty_error_response_and_does_not_login_with_password() {
    let (session, server) = fixture(|mut io| {
        assert_eq!(line(&mut io), "R0001 CAPABILITY");
        write(
            &mut io,
            "* CAPABILITY IMAP4rev1 AUTH=XOAUTH2 SASL-IR\r\nR0001 OK capabilities\r\n",
        );
        assert_eq!(
            line(&mut io),
            format!("R0002 AUTHENTICATE XOAUTH2 {}", expected())
        );
        write(&mut io, "+ eyJzdGF0dXMiOiI0MDEifQ==\r\n");
        assert_eq!(line(&mut io), "");
        write(&mut io, "R0002 NO synthetic-oauth-token rejected\r\n");
        closed(&mut io);
    });
    let mut session = Imap { session, tag: 0 };
    let error = imap(&mut session, &account()).err().unwrap();
    assert!(error.contains("authentication failed"));
    assert!(!error.contains(TOKEN));
    drop(session);
    server.join().unwrap();
}
#[test]
#[cfg(windows)]
fn imap_requires_the_advertised_mechanism_and_the_matching_completion_tag() {
    for advertised in [true, false] {
        let (session, server) = fixture(move |mut io| {
            assert_eq!(line(&mut io), "R0001 CAPABILITY");
            if advertised {
                write(
                    &mut io,
                    "* CAPABILITY AUTH=XOAUTH2 SASL-IR\r\nR0001 OK capa\r\n",
                );
                assert!(line(&mut io).starts_with("R0002 AUTHENTICATE XOAUTH2 "));
                write(&mut io, "R0003 OK wrong-tag\r\n");
            } else {
                write(
                    &mut io,
                    "* CAPABILITY AUTH=PLAIN AUTH=XOAUTH2FAKE\r\nR0001 OK capa\r\n",
                );
            }
            closed(&mut io);
        });
        let mut session = Imap { session, tag: 0 };
        assert!(imap(&mut session, &account()).is_err());
        drop(session);
        server.join().unwrap();
    }
}
#[test]
#[cfg(windows)]
fn password_accounts_keep_their_existing_authentication_exchange() {
    let mut account = account();
    account.auth = Authentication::Password;
    account.set_password("fixture-password").unwrap();
    let (mut session, server) = fixture(|mut io| {
        assert_eq!(line(&mut io), "AUTH LOGIN");
        write(&mut io, "334 username\r\n");
        assert_eq!(
            STANDARD.decode(line(&mut io)).unwrap(),
            b"reader@example.test"
        );
        write(&mut io, "334 password\r\n");
        assert_eq!(STANDARD.decode(line(&mut io)).unwrap(), b"fixture-password");
        write(&mut io, "235 accepted\r\n");
        closed(&mut io);
    });
    smtp(&mut session, &account, &["250-AUTH LOGIN".into()]).unwrap();
    drop(session);
    server.join().unwrap();
    let (mut session, server) = fixture(|mut io| {
        assert_eq!(line(&mut io), "USER reader@example.test");
        write(&mut io, "+OK user\r\n");
        assert_eq!(line(&mut io), "PASS fixture-password");
        write(&mut io, "+OK pass\r\n");
        closed(&mut io);
    });
    pop3(&mut session, &account).unwrap();
    drop(session);
    server.join().unwrap();
    let (session, server) = fixture(|mut io| {
        assert_eq!(
            line(&mut io),
            "R0001 LOGIN \"reader@example.test\" \"fixture-password\""
        );
        write(&mut io, "R0001 OK login\r\n");
        closed(&mut io);
    });
    let mut session = Imap { session, tag: 0 };
    imap(&mut session, &account).unwrap();
    drop(session);
    server.join().unwrap();
}
#[test]
#[cfg(windows)]
fn expired_tokens_and_plaintext_oauth_configuration_cannot_be_used_for_mail() {
    let mut account = account();
    let Authentication::OAuth2(oauth) = &mut account.auth else {
        panic!()
    };
    oauth.tokens.as_mut().unwrap().expires_at = 0;
    let (mut session, server) = fixture(|mut io| closed(&mut io));
    assert!(smtp(&mut session, &account, &["250 AUTH XOAUTH2".into()]).is_err());
    drop(session);
    server.join().unwrap();
    account.incoming_security = Security::Plain;
    assert!(account.validate().err().unwrap().contains("requires TLS"));
}
