use super::{
    LoopbackHost, OAuthConfig, OAuthProvider, check_cancel, checked_value, endpoint,
    pkce_challenge, random_token,
};
use crate::Result;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use url::Url;

const MAX_CALLBACK: usize = 16384;
pub(super) struct Session {
    listener: TcpListener,
    authorization_url: String,
    redirect_uri: String,
    verifier: String,
    state: String,
}
pub(super) struct Grant {
    pub code: String,
    pub redirect_uri: String,
    pub verifier: String,
}

impl Session {
    pub fn start(config: &OAuthConfig, login_hint: &str) -> Result<Self> {
        if !login_hint.is_empty() {
            checked_value(login_hint, "OAuth login hint", 1024)?;
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("Cannot listen for OAuth sign-in: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let host = match config.loopback_host {
            LoopbackHost::IpAddress => "127.0.0.1",
            LoopbackHost::Localhost => "localhost",
        };
        let redirect_uri = format!(
            "http://{host}:{}/",
            listener.local_addr().map_err(|e| e.to_string())?.port()
        );
        let verifier = random_token()?;
        let state = random_token()?;
        let mut authorization = endpoint(&config.authorization_endpoint)?;
        {
            let mut query = authorization.query_pairs_mut();
            query
                .append_pair("client_id", &config.client_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("scope", &config.scopes.join(" "))
                .append_pair("state", &state)
                .append_pair("code_challenge", &pkce_challenge(&verifier))
                .append_pair("code_challenge_method", "S256");
            if !login_hint.is_empty() {
                query.append_pair("login_hint", login_hint);
            }
            match config.provider {
                OAuthProvider::Google => {
                    query
                        .append_pair("access_type", "offline")
                        .append_pair("prompt", "consent");
                }
                OAuthProvider::Microsoft => {
                    query
                        .append_pair("response_mode", "query")
                        .append_pair("prompt", "select_account");
                }
                OAuthProvider::Custom => {}
            }
        }
        Ok(Self {
            listener,
            authorization_url: authorization.into(),
            redirect_uri,
            verifier,
            state,
        })
    }
    pub fn authorization_url(&self) -> &str {
        &self.authorization_url
    }
    pub fn wait(self, cancel: &AtomicBool, timeout: Duration) -> Result<Grant> {
        let deadline = Instant::now() + timeout;
        loop {
            check_cancel(cancel)?;
            if Instant::now() >= deadline {
                return Err("OAuth sign-in timed out; try signing in again".into());
            }
            match self.listener.accept() {
                Ok((mut stream, peer)) => {
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    let request = read_request(&mut stream, cancel, deadline);
                    match request.and_then(|text| self.parse_callback(&text)) {
                        Ok(Callback::Ignore) => {
                            respond(
                                &mut stream,
                                "404 Not Found",
                                "This address is only for mail sign-in.",
                            );
                        }
                        Ok(Callback::Denied) => {
                            respond(
                                &mut stream,
                                "200 OK",
                                "Sign-in was declined. Return to Windows Mail.",
                            );
                            return Err("OAuth sign-in was declined by the provider".into());
                        }
                        Ok(Callback::Code(code)) => {
                            respond(
                                &mut stream,
                                "200 OK",
                                "Sign-in is complete. You may close this page and return to Windows Mail.",
                            );
                            return Ok(Grant {
                                code,
                                redirect_uri: self.redirect_uri,
                                verifier: self.verifier,
                            });
                        }
                        Err(_) => {
                            respond(
                                &mut stream,
                                "400 Bad Request",
                                "This sign-in response was not accepted. Use the original sign-in browser tab.",
                            );
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(e) => return Err(format!("OAuth callback listener failed: {e}")),
            }
        }
    }
    fn parse_callback(&self, request: &str) -> Result<Callback> {
        let mut lines = request.split("\r\n");
        let mut first = lines
            .next()
            .ok_or("Missing callback request")?
            .split_whitespace();
        let method = first.next().ok_or("Missing callback method")?;
        let target = first.next().ok_or("Missing callback target")?;
        let version = first.next().ok_or("Missing callback HTTP version")?;
        if method != "GET"
            || !matches!(version, "HTTP/1.1" | "HTTP/1.0")
            || first.next().is_some()
            || !target.starts_with('/')
            || target.starts_with("//")
        {
            return Err("Invalid OAuth callback request".into());
        }
        let redirect = Url::parse(&self.redirect_uri).map_err(|e| e.to_string())?;
        let expected_host = format!(
            "{}:{}",
            redirect.host_str().unwrap_or_default(),
            redirect.port().unwrap_or(80)
        );
        let mut host = None;
        for line in lines.take_while(|line| !line.is_empty()) {
            let (name, value) = line.split_once(':').ok_or("Invalid callback HTTP header")?;
            if name.eq_ignore_ascii_case("host") && host.replace(value.trim()).is_some() {
                return Err("Duplicate callback Host header".into());
            }
        }
        if host != Some(expected_host.as_str()) {
            return Err("OAuth callback Host mismatch".into());
        }
        let callback = Url::parse(&format!("http://{expected_host}{target}"))
            .map_err(|_| "Invalid callback URL")?;
        if callback.path() != "/" {
            return Ok(Callback::Ignore);
        }
        let mut state = None;
        let mut code = None;
        let mut error = None;
        for (key, value) in callback.query_pairs() {
            let slot = match key.as_ref() {
                "state" => &mut state,
                "code" => &mut code,
                "error" => &mut error,
                _ => continue,
            };
            if slot.replace(value.into_owned()).is_some() {
                return Err("Duplicate OAuth callback parameter".into());
            }
        }
        if !state
            .as_deref()
            .is_some_and(|value| equal_state(value, &self.state))
        {
            return Err("OAuth callback state mismatch".into());
        }
        match (code, error) {
            (Some(code), None) => {
                checked_value(&code, "OAuth authorization code", 8192)?;
                Ok(Callback::Code(code))
            }
            (None, Some(_)) => Ok(Callback::Denied),
            _ => Err("OAuth callback has no unambiguous authorization result".into()),
        }
    }
}
enum Callback {
    Ignore,
    Denied,
    Code(String),
}
fn equal_state(left: &str, right: &str) -> bool {
    let mut mismatch = left.len() ^ right.len();
    for (a, b) in left.bytes().zip(right.bytes()) {
        mismatch |= usize::from(a ^ b);
    }
    mismatch == 0
}
fn read_request(stream: &mut TcpStream, cancel: &AtomicBool, deadline: Instant) -> Result<String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|e| e.to_string())?;
    let connection_deadline = (Instant::now() + Duration::from_secs(2)).min(deadline);
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        check_cancel(cancel)?;
        if Instant::now() >= connection_deadline {
            return Err("OAuth callback request timed out".into());
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err("OAuth callback connection closed".into()),
            Ok(count) => {
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.len() > MAX_CALLBACK {
                    return Err("OAuth callback request is too large".into());
                }
                if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    return String::from_utf8(bytes)
                        .map_err(|_| "OAuth callback request is not UTF-8".into());
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
}
fn respond(stream: &mut TcpStream, status: &str, text: &str) {
    let body = format!(
        "<!doctype html><html><head><meta charset=utf-8><title>Windows Mail</title></head><body><p>{text}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
    let _ = stream.write_all(response.as_bytes());
}
