use super::*;
use crate::account::{AuthCredential, Incoming, Security};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

type Forms = Arc<Mutex<Vec<HashMap<String, String>>>>;
struct Fixture {
    url: String,
    forms: Forms,
    thread: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn start(responses: Vec<(u16, String, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "https://127.0.0.1:{}/token",
            listener.local_addr().unwrap().port()
        );
        let forms = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&forms);
        let thread = thread::spawn(move || {
            for (status, body, extra_headers) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut raw = Vec::new();
                let mut byte = [0u8; 1];
                while !raw.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    raw.push(byte[0]);
                    assert!(raw.len() < 16384);
                }
                let header = String::from_utf8(raw).unwrap();
                assert!(header.starts_with("POST /token HTTP/1.1"));
                let count: usize = header
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                            .map(|(_, v)| v.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut body_raw = vec![0u8; count];
                stream.read_exact(&mut body_raw).unwrap();
                seen.lock().unwrap().push(
                    url::form_urlencoded::parse(&body_raw)
                        .map(|(a, b)| (a.into_owned(), b.into_owned()))
                        .collect(),
                );
                write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}", body.len()).unwrap();
            }
        });
        Self {
            url,
            forms,
            thread: Some(thread),
        }
    }
    fn finish(mut self) -> Vec<HashMap<String, String>> {
        self.thread.take().unwrap().join().unwrap();
        self.forms.lock().unwrap().clone()
    }
}
fn response(access: &str, refresh: Option<&str>) -> (u16, String, String) {
    let mut body =
        serde_json::json!({"access_token":access,"token_type":"Bearer","expires_in":3600});
    if let Some(refresh) = refresh {
        body["refresh_token"] = refresh.into();
    }
    (200, body.to_string(), String::new())
}
fn account(oauth: OAuthAccount) -> Account {
    Account {
        auth: Authentication::OAuth2(oauth),
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
fn custom(endpoint: &str) -> OAuthConfig {
    OAuthConfig {
        provider: OAuthProvider::Custom,
        client_id: "owned-desktop-client".into(),
        client_secret_dpapi: String::new(),
        authorization_endpoint: "https://provider.example.test/authorize".into(),
        token_endpoint: endpoint.into(),
        scopes: vec!["mail".into(), "offline_access".into()],
        loopback_host: LoopbackHost::IpAddress,
    }
}
fn browser_callback(
    url: &str,
    modifications: impl FnOnce(&Url, &HashMap<String, String>),
) -> Result<()> {
    let authorize = Url::parse(url).unwrap();
    let params: HashMap<String, String> = authorize
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let redirect = Url::parse(&params["redirect_uri"]).unwrap();
    modifications(&redirect, &params);
    Ok(())
}
fn send_callback(redirect: &Url, target: &str, host_override: Option<&str>) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", redirect.port().unwrap())).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let host = format!(
        "{}:{}",
        redirect.host_str().unwrap(),
        redirect.port().unwrap()
    );
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        host_override.unwrap_or(&host)
    )
    .unwrap();
    let mut body = String::new();
    stream.read_to_string(&mut body).unwrap();
    body
}
fn callback_target(state: &str, code: &str) -> String {
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("state", state)
        .append_pair("code", code)
        .finish();
    format!("/?{encoded}")
}

#[test]
fn pkce_uses_the_rfc_7636_vector_and_cryptographic_unique_values() {
    assert_eq!(
        pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    let first = random_token().unwrap();
    let second = random_token().unwrap();
    assert_eq!(first.len(), 43);
    assert_ne!(first, second);
}
#[test]
fn presets_validate_and_unsafe_or_incomplete_configurations_fail() {
    OAuthConfig::microsoft("owned-client-id")
        .validate()
        .unwrap();
    OAuthConfig::google("owned.apps.googleusercontent.com")
        .validate()
        .unwrap();
    let mut config = OAuthConfig::google("");
    assert!(config.validate().is_err());
    config.client_id = " ".into();
    assert!(config.validate().is_err());
    config.client_id = "owned".into();
    for url in [
        "http://example.test/token",
        "https://user:secret@example.test/token",
        "https://example.test/token#fragment",
        "https://example.test/token?state=bad",
    ] {
        config.token_endpoint = url.into();
        assert!(config.validate().is_err(), "{url}");
    }
    config = OAuthConfig::microsoft("owned");
    config.set_client_secret("not-applicable").unwrap();
    assert!(config.validate().is_err());
}
#[test]
fn legacy_catalogs_default_to_password_and_xoauth2_cannot_inject_sasl_fields() {
    let old = r#"{"email":"reader@example.test","username":"reader","password_dpapi":"","incoming":"imap","incoming_host":"imap.example.test","incoming_port":993,"incoming_security":"tls","smtp_host":"smtp.example.test","smtp_port":465,"smtp_security":"tls"}"#;
    let value: Account = serde_json::from_str(old).unwrap();
    assert!(matches!(value.auth, Authentication::Password));
    let encoded = xoauth2_response("reader@example.test", "test-token").unwrap();
    assert_eq!(
        STANDARD.decode(encoded).unwrap(),
        b"user=reader@example.test\x01auth=Bearer test-token\x01\x01"
    );
    assert!(xoauth2_response("reader\x01auth=Bearer evil", "token").is_err());
    assert!(xoauth2_response("reader", "token\r\n").is_err());
}
#[test]
#[cfg(windows)]
fn authorization_code_and_rotating_refresh_tokens_round_trip_without_plaintext_storage() {
    let fixture = Fixture::start(vec![
        response("access-one", Some("refresh-one")),
        response("access-two", Some("refresh-two")),
        response("access-three", None),
    ]);
    let mut config = custom(&fixture.url);
    config.set_client_secret("desktop-secret&special").unwrap();
    let challenge = Arc::new(Mutex::new(String::new()));
    let expected = Arc::clone(&challenge);
    let oauth = authorize_with_client(
        &config,
        "reader@example.test",
        &AtomicBool::new(false),
        |url| {
            browser_callback(url, |redirect, params| {
                *expected.lock().unwrap() = params["code_challenge"].clone();
                assert_eq!(params["code_challenge_method"], "S256");
                let redirect = redirect.clone();
                let state = params["state"].clone();
                thread::spawn(move || {
                    assert!(
                        send_callback(
                            &redirect,
                            &callback_target(&state, "code+with&encoding"),
                            None
                        )
                        .starts_with("HTTP/1.1 200")
                    );
                });
            })
        },
        &http::TokenClient::for_loopback_fixture(),
    )
    .unwrap();
    let saved = serde_json::to_string(&oauth).unwrap();
    for secret in ["access-one", "refresh-one", "desktop-secret&special"] {
        assert!(!saved.contains(secret));
    }
    let mut restored: OAuthAccount = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored.access_token().unwrap(), "access-one");
    restored.tokens.as_mut().unwrap().expires_at = 0;
    let mut account = account(restored);
    assert_eq!(
        refresh_with_client(
            &mut account,
            false,
            &http::TokenClient::for_loopback_fixture()
        )
        .unwrap(),
        "access-two"
    );
    let Authentication::OAuth2(oauth) = &mut account.auth else {
        panic!()
    };
    oauth.tokens.as_mut().unwrap().expires_at = 0;
    assert_eq!(
        refresh_with_client(
            &mut account,
            false,
            &http::TokenClient::for_loopback_fixture()
        )
        .unwrap(),
        "access-three"
    );
    assert!(
        matches!(account.network_auth().unwrap(),AuthCredential::OAuth2(token) if token=="access-three")
    );
    let forms = fixture.finish();
    assert_eq!(forms[0]["grant_type"], "authorization_code");
    assert_eq!(forms[0]["code"], "code+with&encoding");
    assert_eq!(
        pkce_challenge(&forms[0]["code_verifier"]),
        *challenge.lock().unwrap()
    );
    assert_eq!(forms[0]["client_secret"], "desktop-secret&special");
    assert_eq!(forms[1]["refresh_token"], "refresh-one");
    assert_eq!(forms[2]["refresh_token"], "refresh-two");
    let Authentication::OAuth2(oauth) = &account.auth else {
        panic!()
    };
    assert_eq!(
        oauth
            .tokens
            .as_ref()
            .unwrap()
            .unseal()
            .unwrap()
            .refresh_token
            .as_deref(),
        Some("refresh-two")
    );
}
#[test]
fn callback_rejects_wrong_state_duplicate_parameters_and_dns_rebinding_before_accepting_original_flow()
 {
    let config = OAuthConfig::google("owned-desktop");
    let session = callback::Session::start(&config, "").unwrap();
    let params: HashMap<String, String> = Url::parse(session.authorization_url())
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert_eq!(params["access_type"], "offline");
    assert_eq!(params["prompt"], "consent");
    let redirect = Url::parse(&params["redirect_uri"]).unwrap();
    let state = params["state"].clone();
    let browser = thread::spawn(move || {
        assert!(
            send_callback(
                &redirect,
                &callback_target("attacker-state", "wrong-code"),
                None
            )
            .starts_with("HTTP/1.1 400")
        );
        let duplicate = format!("{}&state={state}", callback_target(&state, "wrong-code"));
        assert!(send_callback(&redirect, &duplicate, None).starts_with("HTTP/1.1 400"));
        assert!(
            send_callback(
                &redirect,
                &callback_target(&state, "wrong-code"),
                Some("attacker.example.test")
            )
            .starts_with("HTTP/1.1 400")
        );
        assert!(
            send_callback(&redirect, &callback_target(&state, "correct-code"), None)
                .starts_with("HTTP/1.1 200")
        );
    });
    let grant = session
        .wait(&AtomicBool::new(false), Duration::from_secs(3))
        .unwrap();
    browser.join().unwrap();
    assert_eq!(grant.code, "correct-code");
}
#[test]
fn callback_cancellation_and_timeout_release_the_bound_port() {
    let session = callback::Session::start(&OAuthConfig::google("owned"), "").unwrap();
    let params: HashMap<String, String> = Url::parse(session.authorization_url())
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let port = Url::parse(&params["redirect_uri"]).unwrap().port().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = Arc::clone(&cancel);
    let thread = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        trigger.store(true, Ordering::Release);
    });
    assert!(
        session
            .wait(&cancel, Duration::from_secs(3))
            .err()
            .unwrap()
            .contains("cancelled")
    );
    thread.join().unwrap();
    TcpListener::bind(("127.0.0.1", port)).unwrap();
    let session = callback::Session::start(&OAuthConfig::google("owned"), "").unwrap();
    assert!(
        session
            .wait(&AtomicBool::new(false), Duration::from_millis(10))
            .err()
            .unwrap()
            .contains("timed out")
    );
    assert!(
        authorize(
            &OAuthConfig::google("owned"),
            "",
            &AtomicBool::new(true),
            |_| panic!("Cancelled sign-in must not open a browser")
        )
        .is_err()
    );
}
#[test]
fn provider_denial_is_not_a_successful_login_and_never_echoes_query_secrets() {
    let session = callback::Session::start(&OAuthConfig::microsoft("owned"), "").unwrap();
    let params: HashMap<String, String> = Url::parse(session.authorization_url())
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let redirect = Url::parse(&params["redirect_uri"]).unwrap();
    let state = params["state"].clone();
    let browser = thread::spawn(move || {
        let body = send_callback(
            &redirect,
            &format!("/?state={state}&error=access_denied&error_description=secret-information"),
            None,
        );
        assert!(!body.contains("secret-information"));
    });
    assert!(
        session
            .wait(&AtomicBool::new(false), Duration::from_secs(3))
            .err()
            .unwrap()
            .contains("declined")
    );
    browser.join().unwrap();
}
#[test]
#[cfg(windows)]
fn refresh_rejection_preserves_original_encrypted_tokens_and_redacts_provider_description() {
    let fixture = Fixture::start(vec![(
        400,
        r#"{"error":"invalid_grant","error_description":"super-secret-refresh-token"}"#.into(),
        String::new(),
    )]);
    let tokens = seal_response(
        TokenResponse {
            access_token: "old-access".into(),
            token_type: "Bearer".into(),
            expires_in: 1,
            refresh_token: Some("old-refresh".into()),
        },
        None,
    )
    .unwrap();
    let original = tokens.protected.clone();
    let mut account = account(OAuthAccount {
        config: custom(&fixture.url),
        tokens: Some(tokens),
    });
    let error = refresh_with_client(
        &mut account,
        true,
        &http::TokenClient::for_loopback_fixture(),
    )
    .err()
    .unwrap();
    assert!(error.contains("invalid_grant"));
    assert!(!error.contains("super-secret"));
    let Authentication::OAuth2(oauth) = &account.auth else {
        panic!()
    };
    assert_eq!(oauth.tokens.as_ref().unwrap().protected, original);
    assert_eq!(fixture.finish()[0]["refresh_token"], "old-refresh");
}
#[test]
fn token_redirects_do_not_forward_codes_or_client_credentials() {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let fixture = Fixture::start(vec![(
        302,
        String::new(),
        format!(
            "Location: http://127.0.0.1:{}/stolen\r\n",
            target.local_addr().unwrap().port()
        ),
    )]);
    let error = http::TokenClient::for_loopback_fixture()
        .exchange(
            &fixture.url,
            &[("code".into(), "never-forward-this".into())],
        )
        .err()
        .unwrap();
    assert!(error.contains("redirected"));
    assert_eq!(fixture.finish().len(), 1);
    assert_eq!(
        target.accept().err().unwrap().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn untrusted_https_certificates_are_rejected_before_sending_token_credentials() {
    // This identity is self-signed and belongs only to the loopback test fixture.
    // Never install it in an operating-system trust store.
    let identity = native_tls::Identity::from_pkcs12(
        include_bytes!("../../tests/fixtures/oauth-loopback.pfx"),
        "fixture-only",
    )
    .unwrap();
    let acceptor = native_tls::TlsAcceptor::new(identity).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!(
        "https://127.0.0.1:{}/token",
        listener.local_addr().unwrap().port()
    );
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        match acceptor.accept(stream) {
            Err(_) => false,
            Ok(mut tls) => {
                let mut byte = [0u8; 1];
                tls.read(&mut byte).is_ok_and(|count| count > 0)
            }
        }
    });
    let error = http::TokenClient::new()
        .exchange(
            &endpoint,
            &[(
                "refresh_token".into(),
                "fixture-credential-must-not-be-sent".into(),
            )],
        )
        .err()
        .unwrap();
    assert!(error.contains("HTTPS certificate"));
    assert!(
        !server.join().unwrap(),
        "The client sent token HTTP data before validating the certificate"
    );
}

#[test]
fn oversized_or_invalid_token_responses_are_rejected() {
    let fixture = Fixture::start(vec![
        (200, "x".repeat(65537), String::new()),
        (
            200,
            r#"{"access_token":"token","token_type":"Bearer","expires_in":0}"#.into(),
            String::new(),
        ),
    ]);
    let client = http::TokenClient::for_loopback_fixture();
    let values = [("grant_type".into(), "fixture".into())];
    assert!(
        client
            .exchange(&fixture.url, &values)
            .err()
            .unwrap()
            .contains("too large")
    );
    let response = client.exchange(&fixture.url, &values).unwrap();
    assert!(
        seal_response(response, None)
            .err()
            .unwrap()
            .contains("already expired")
    );
    fixture.finish();
}
