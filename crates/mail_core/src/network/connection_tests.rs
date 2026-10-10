use super::*;
use crate::account::Authentication;
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

fn server(
    script: impl FnOnce(BufReader<TcpStream>) + Send + 'static,
) -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        script(BufReader::new(stream));
    });
    (port, handle)
}
fn read(io: &mut BufReader<TcpStream>) -> String {
    let mut line = String::new();
    io.read_line(&mut line).unwrap();
    line.trim_end().into()
}
fn reply(io: &mut BufReader<TcpStream>, value: &str) {
    io.get_mut().write_all(value.as_bytes()).unwrap();
    io.get_mut().flush().unwrap();
}

#[test]
fn connection_test_authenticates_both_servers_without_mail_commands() {
    for protocol in [Incoming::Imap, Incoming::Pop3] {
        for reject in [false, true] {
            let (incoming_port, incoming) = server(move |mut io| {
                if protocol == Incoming::Imap {
                    reply(&mut io, "* OK fixture\r\n");
                    let login = read(&mut io);
                    let tag = login.split_whitespace().next().unwrap();
                    assert!(login.contains(" LOGIN "));
                    reply(
                        &mut io,
                        &format!("{tag} {} fixture\r\n", if reject { "NO" } else { "OK" }),
                    );
                    if !reject {
                        let logout = read(&mut io);
                        assert!(logout.ends_with(" LOGOUT"));
                        reply(
                            &mut io,
                            &format!(
                                "{} OK logout\r\n",
                                logout.split_whitespace().next().unwrap()
                            ),
                        );
                    }
                } else {
                    reply(&mut io, "+OK fixture\r\n");
                    assert_eq!(read(&mut io), "USER reader@example.test");
                    reply(&mut io, "+OK user\r\n");
                    assert_eq!(read(&mut io), "PASS fixture-password");
                    reply(
                        &mut io,
                        if reject {
                            "-ERR rejected\r\n"
                        } else {
                            "+OK authenticated\r\n"
                        },
                    );
                    if !reject {
                        assert_eq!(read(&mut io), "QUIT");
                    }
                }
                assert_eq!(read(&mut io), "");
            });
            let (smtp_port, smtp) = server(|mut io| {
                reply(&mut io, "220 fixture\r\n");
                assert!(read(&mut io).starts_with("EHLO "));
                reply(&mut io, "250-fixture\r\n250 AUTH PLAIN\r\n");
                assert!(read(&mut io).starts_with("AUTH PLAIN "));
                reply(&mut io, "235 authenticated\r\n");
                assert_eq!(read(&mut io), "QUIT");
                assert_eq!(read(&mut io), "");
            });
            let mut account = Account {
                auth: Authentication::Password,
                email: "reader@example.test".into(),
                username: "reader@example.test".into(),
                password_dpapi: String::new(),
                incoming: protocol,
                incoming_host: "127.0.0.1".into(),
                incoming_port,
                incoming_security: Security::Plain,
                smtp_host: "127.0.0.1".into(),
                smtp_port,
                smtp_security: Security::Plain,
            };
            account.set_password("fixture-password").unwrap();
            let report = test_connection(&account);
            assert_eq!(report.incoming.is_err(), reject);
            assert!(report.outgoing.is_ok(), "{:?}", report.outgoing);
            incoming.join().unwrap();
            smtp.join().unwrap();
        }
    }
}
