//! Bounded protocol implementations with certificate-validated TLS.
//! Remote changes are explicit and use UID-scoped operations only.
use crate::{
    MAX_MESSAGE, Result,
    account::{Account, Incoming, Security},
    message::Draft,
};
use base64::Engine;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

trait Io: Read + Write + Send {}
impl<T: Read + Write + Send> Io for T {}
pub struct Session {
    io: BufReader<Box<dyn Io>>,
}
impl Session {
    fn connect(host: &str, port: u16, mode: Security) -> Result<Self> {
        if host.is_empty() || host.contains(['\r', '\n', '\0']) {
            return Err("Invalid server name".into());
        }
        let mut last = String::new();
        let mut connected = None;
        for address in (host, port).to_socket_addrs().map_err(|e| e.to_string())? {
            if mode == Security::Plain && !address.ip().is_loopback() {
                return Err("Unencrypted mail transport is allowed only on loopback; choose TLS or STARTTLS".into());
            }
            match TcpStream::connect_timeout(&address, Duration::from_secs(15)) {
                Ok(s) => {
                    connected = Some(s);
                    break;
                }
                Err(e) => last = e.to_string(),
            }
        }
        let stream = connected.ok_or_else(|| format!("Connect failed: {last}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| e.to_string())?;
        let mut s = Self {
            io: BufReader::new(Box::new(stream)),
        };
        if mode == Security::Tls {
            s.upgrade(host)?;
        }
        Ok(s)
    }
    fn upgrade(&mut self, host: &str) -> Result<()> {
        if !self.io.buffer().is_empty() {
            return Err("Unexpected buffered data before TLS handshake".into());
        }
        struct Empty;
        impl Read for Empty {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Ok(0)
            }
        }
        impl Write for Empty {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Ok(0)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let raw = std::mem::replace(&mut self.io, BufReader::new(Box::new(Empty))).into_inner();
        let connector = native_tls::TlsConnector::builder()
            .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
            .build()
            .map_err(|e| e.to_string())?;
        let tls = connector.connect(host, raw).map_err(|e| match e {
            native_tls::HandshakeError::Failure(error) => format!("TLS handshake failed: {error}"),
            native_tls::HandshakeError::WouldBlock(_) => "TLS handshake would block".into(),
        })?;
        self.io = BufReader::new(Box::new(tls));
        Ok(())
    }
    fn send(&mut self, text: &str) -> Result<()> {
        if text.contains(['\r', '\n', '\0']) {
            return Err("Protocol command contains control characters".into());
        }
        self.write(text.as_bytes())?;
        self.write(b"\r\n")
    }
    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.io
            .get_mut()
            .write_all(data)
            .and_then(|_| self.io.get_mut().flush())
            .map_err(|e| e.to_string())
    }
    fn bytes_line(&mut self) -> Result<Vec<u8>> {
        self.bytes_line_limit(65536)
    }
    fn bytes_line_limit(&mut self, limit: usize) -> Result<Vec<u8>> {
        let mut line = Vec::new();
        loop {
            let available = self.io.fill_buf().map_err(|e| e.to_string())?;
            if available.is_empty() {
                return Err("Server disconnected".into());
            }
            let n = available
                .iter()
                .position(|&b| b == b'\n')
                .map_or(available.len(), |i| i + 1);
            if line.len() + n > limit {
                return Err(format!("Server line exceeds {limit} bytes"));
            }
            let complete = available[n - 1] == b'\n';
            line.extend_from_slice(&available[..n]);
            self.io.consume(n);
            if complete {
                break;
            }
        }
        if line.ends_with(b"\n") {
            line.pop();
        }
        if line.ends_with(b"\r") {
            line.pop();
        }
        Ok(line)
    }
    fn line(&mut self) -> Result<String> {
        self.bytes_line()
            .map(|line| String::from_utf8_lossy(&line).into_owned())
    }
    fn numeric(&mut self) -> Result<(u16, Vec<String>)> {
        let mut lines = Vec::new();
        let mut status = None;
        for _ in 0..100 {
            let line = self.line()?;
            let code = line
                .get(..3)
                .and_then(|s| s.parse::<u16>().ok())
                .ok_or("Malformed server reply")?;
            if status.is_some_and(|c| c != code) {
                return Err("Inconsistent multiline reply".into());
            }
            status = Some(code);
            let more = line.as_bytes().get(3) == Some(&b'-');
            lines.push(line);
            if !more {
                return Ok((code, lines));
            }
        }
        Err("Too many reply lines".into())
    }
    fn expect(&mut self, allowed: &[u16]) -> Result<Vec<String>> {
        let (code, lines) = self.numeric()?;
        if allowed.contains(&code) {
            Ok(lines)
        } else {
            Err(format!("Server rejected operation with code {code}"))
        }
    }
    fn dot_block(&mut self) -> Result<Vec<u8>> {
        let mut body = Vec::new();
        loop {
            let line = self.bytes_line()?;
            if line == b"." {
                return Ok(body);
            }
            let bytes = if line.starts_with(b"..") {
                &line[1..]
            } else {
                &line[..]
            };
            if body.len() + bytes.len() + 2 > MAX_MESSAGE {
                return Err("Server message exceeds 50 MiB".into());
            }
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
    }
}

pub fn smtp_send(account: &Account, draft: &Draft, raw: &[u8]) -> Result<()> {
    account.validate()?;
    if raw.len() > MAX_MESSAGE {
        return Err("Message exceeds 50 MiB".into());
    }
    let sender = crate::message::addresses(&account.email)?;
    let recipients = draft.recipients()?;
    let mut s = Session::connect(&account.smtp_host, account.smtp_port, account.smtp_security)?;
    s.expect(&[220])?;
    s.send("EHLO winmail-rust.local")?;
    let mut capabilities = s.expect(&[250])?;
    if account.smtp_security == Security::StartTls {
        if !capabilities
            .iter()
            .any(|l| l.to_uppercase().contains("STARTTLS"))
        {
            return Err("Server does not advertise STARTTLS".into());
        }
        s.send("STARTTLS")?;
        s.expect(&[220])?;
        s.upgrade(&account.smtp_host)?;
        s.send("EHLO winmail-rust.local")?;
        capabilities = s.expect(&[250])?;
    }
    auth::smtp(&mut s, account, &capabilities)?;
    s.send(&format!("MAIL FROM:<{}>", sender[0]))?;
    s.expect(&[250])?;
    for recipient in recipients {
        s.send(&format!("RCPT TO:<{recipient}>"))?;
        s.expect(&[250, 251])?;
    }
    s.send("DATA")?;
    s.expect(&[354])?;
    s.write(&dot_stuff(raw))?;
    s.expect(&[250])?;
    // Successful DATA is the acceptance point; a failed QUIT must not cause resend.
    let _ = s.send("QUIT");
    let _ = s.expect(&[221]);
    Ok(())
}
pub fn dot_stuff(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 16);
    for line in raw.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.first() == Some(&b'.') {
            out.push(b'.');
        }
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    if raw.ends_with(b"\n") {
        out.truncate(out.len() - 2);
    }
    out.extend_from_slice(b".\r\n");
    out
}

pub struct Download {
    pub key: String,
    pub raw: Vec<u8>,
}

/// IMAP reserves case-insensitive spelling only for INBOX; other names stay exact.
pub fn mailbox_eq(left: &str, right: &str) -> bool {
    left == right || (left.eq_ignore_ascii_case("INBOX") && right.eq_ignore_ascii_case("INBOX"))
}

/// An IMAP UID is meaningful only in this connection and mailbox epoch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImapLocation {
    pub connection: String,
    pub mailbox: String,
    pub uid_validity: u32,
    pub uid: u32,
}

/// A lost response after a mutation is different from a rejected command.
#[derive(Debug)]
pub enum ImapChange {
    Moved(ImapLocation),
    /// Source is marked Deleted; only the server may compact it without UIDPLUS.
    MovedDeferred {
        destination: ImapLocation,
        original: ImapLocation,
    },
    /// Acknowledged Deleted flag, without a safe targeted expunge.
    MarkedDeleted(ImapLocation),
    Deleted,
    Uncertain(String),
}

impl ImapLocation {
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        let mut parts = key.split(':');
        let connection = parts.next()?.to_owned();
        let kind = parts.next()?;
        let mailbox = match kind {
            "imap" => "INBOX".into(),
            "imap2" => String::from_utf8(
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(parts.next()?)
                    .ok()?,
            )
            .ok()?,
            _ => return None,
        };
        let uid_validity = parts.next()?.parse().ok()?;
        let uid = parts.next()?.parse().ok()?;
        if parts.next().is_some()
            || uid_validity == 0
            || uid == 0
            || mailbox.contains(['\r', '\n', '\0'])
        {
            return None;
        }
        Some(Self {
            connection,
            mailbox,
            uid_validity,
            uid,
        })
    }
    pub fn remote_key(&self) -> String {
        if self.mailbox.eq_ignore_ascii_case("INBOX") {
            format!(
                "{}:imap:{}:{}",
                self.connection, self.uid_validity, self.uid
            )
        } else {
            format!(
                "{}:imap2:{}:{}:{}",
                self.connection,
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.mailbox.as_bytes()),
                self.uid_validity,
                self.uid
            )
        }
    }
}
const MAX_BATCH: usize = 100 * 1024 * 1024;
pub fn receive(account: &Account, known: &[String]) -> Result<Vec<Download>> {
    account.validate()?;
    match account.incoming {
        Incoming::Pop3 => pop3(account, known),
        Incoming::Imap => imap(account, known),
    }
}
fn pop_ok(s: &mut Session) -> Result<String> {
    let line = s.line()?;
    if line.starts_with("+OK") {
        Ok(line)
    } else {
        Err("POP3 server rejected operation".into())
    }
}
fn pop3(account: &Account, known: &[String]) -> Result<Vec<Download>> {
    let mut s = Session::connect(
        &account.incoming_host,
        account.incoming_port,
        account.incoming_security,
    )?;
    pop_ok(&mut s)?;
    if account.incoming_security == Security::StartTls {
        s.send("STLS")?;
        pop_ok(&mut s)?;
        s.upgrade(&account.incoming_host)?;
    }
    auth::pop3(&mut s, account)?;
    s.send("UIDL")?;
    pop_ok(&mut s)?;
    let listing = s.dot_block()?;
    let mut downloads = Vec::new();
    let mut bytes = 0;
    for line in String::from_utf8_lossy(&listing).lines() {
        let mut parts = line.split_whitespace();
        let number = parts
            .next()
            .ok_or("Malformed POP3 UIDL")?
            .parse::<u32>()
            .map_err(|_| "Malformed POP3 message number")?;
        let uid = parts.next().ok_or("Malformed POP3 UIDL")?;
        let key = format!("{}:pop3:{uid}", account.id());
        if known.contains(&key) {
            continue;
        }
        s.send(&format!("RETR {number}"))?;
        pop_ok(&mut s)?;
        let raw = s.dot_block()?;
        if bytes + raw.len() > MAX_BATCH {
            break;
        }
        bytes += raw.len();
        downloads.push(Download { key, raw });
        if downloads.len() >= 200 || bytes >= MAX_BATCH {
            break;
        }
    }
    let _ = s.send("QUIT");
    let _ = pop_ok(&mut s);
    Ok(downloads)
}
fn quoted(text: &str) -> Result<String> {
    if text.contains(['\r', '\n', '\0']) {
        return Err("IMAP string contains control characters".into());
    }
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}
struct Imap {
    session: Session,
    tag: u32,
}
struct ImapReply {
    lines: Vec<String>,
    literals: Vec<Vec<u8>>,
    completion: String,
}
enum ImapError {
    Rejected,
    Failure(String),
}
impl From<String> for ImapError {
    fn from(value: String) -> Self {
        Self::Failure(value)
    }
}
impl From<&str> for ImapError {
    fn from(value: &str) -> Self {
        Self::Failure(value.into())
    }
}
impl From<ImapError> for String {
    fn from(value: ImapError) -> Self {
        match value {
            ImapError::Rejected => "IMAP server rejected operation".into(),
            ImapError::Failure(error) => error,
        }
    }
}
impl Imap {
    fn command(&mut self, text: &str) -> std::result::Result<ImapReply, ImapError> {
        self.tag = self
            .tag
            .checked_add(1)
            .ok_or("IMAP command tag exhausted")?;
        let tag = format!("R{:04}", self.tag);
        self.session.send(&format!("{tag} {text}"))?;
        let mut lines = Vec::new();
        let mut literals = Vec::new();
        let mut total = 0;
        for _ in 0..100_000 {
            // SEARCH may legitimately return hundreds of thousands of UIDs on
            // one line. Overall response bytes remain capped at MAX_BATCH.
            let line = String::from_utf8_lossy(&self.session.bytes_line_limit(16 * 1024 * 1024)?)
                .into_owned();
            total += line.len();
            if total > MAX_BATCH {
                return Err("IMAP response exceeds 100 MiB".into());
            }
            if line.starts_with(&format!("{tag} ")) {
                if line
                    .split_whitespace()
                    .nth(1)
                    .is_some_and(|s| s.eq_ignore_ascii_case("OK"))
                {
                    return Ok(ImapReply {
                        lines,
                        literals,
                        completion: line,
                    });
                }
                return Err(ImapError::Rejected);
            }
            if line.starts_with("* BYE") {
                return Err("IMAP server closed session".into());
            }
            if line.ends_with('}')
                && let Some(pos) = line.rfind('{')
            {
                let size = line[pos + 1..line.len() - 1]
                    .trim_end_matches('+')
                    .parse::<usize>()
                    .map_err(|_| "Malformed IMAP literal")?;
                if size > MAX_MESSAGE || literals.len() >= 4096 {
                    return Err("IMAP literal exceeds limits".into());
                }
                total += size;
                if total > MAX_BATCH {
                    return Err("IMAP response exceeds 100 MiB".into());
                }
                let mut body = vec![0; size];
                self.session
                    .io
                    .read_exact(&mut body)
                    .map_err(|e| e.to_string())?;
                literals.push(body);
            }
            lines.push(line);
        }
        Err("Too many IMAP response lines".into())
    }
}
fn imap_connect(account: &Account) -> Result<Imap> {
    account.validate()?;
    if account.incoming != Incoming::Imap {
        return Err("Account does not use IMAP".into());
    }
    let mut session = Session::connect(
        &account.incoming_host,
        account.incoming_port,
        account.incoming_security,
    )?;
    let greeting = session.line()?;
    let preauth = greeting.starts_with("* PREAUTH");
    if !greeting.starts_with("* OK") && !preauth {
        return Err("Invalid IMAP greeting".into());
    }
    let mut s = Imap { session, tag: 0 };
    if account.incoming_security == Security::StartTls {
        s.command("STARTTLS")?;
        s.session.upgrade(&account.incoming_host)?;
    }
    if !preauth {
        auth::imap(&mut s, account)?;
    }
    Ok(s)
}
fn uid_validity(reply: &ImapReply) -> Result<u32> {
    if reply
        .lines
        .iter()
        .chain(std::iter::once(&reply.completion))
        .any(|line| line.to_ascii_uppercase().contains("[UIDNOTSTICKY]"))
    {
        return Err("Server mailbox UIDs are not persistent; remote operation refused".into());
    }
    let value = reply
        .lines
        .iter()
        .chain(std::iter::once(&reply.completion))
        .find_map(|line| {
            line.to_ascii_uppercase()
                .find("[UIDVALIDITY ")
                .and_then(|position| line[position + 13..].split(']').next())
        })
        .ok_or("IMAP server omitted UIDVALIDITY; refusing unreliable identities")?;
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| "Malformed UIDVALIDITY".into())
}
fn imap(account: &Account, known: &[String]) -> Result<Vec<Download>> {
    let mut s = imap_connect(account)?;
    let selected = s.command("SELECT INBOX")?;
    let validity = uid_validity(&selected)?;
    let reply = s.command("UID SEARCH ALL")?;
    let mut uids = Vec::new();
    for line in &reply.lines {
        if let Some(values) = line.strip_prefix("* SEARCH") {
            for uid in values.split_whitespace() {
                uids.push(uid.parse::<u32>().map_err(|_| "Malformed IMAP UID")?);
            }
        }
    }
    uids.sort();
    uids.dedup();
    let mut downloads = Vec::new();
    let mut bytes = 0;
    for uid in uids.into_iter().rev() {
        let key = format!("{}:imap:{validity}:{uid}", account.id());
        if known.contains(&key) {
            continue;
        }
        let reply = s.command(&format!("UID FETCH {uid} (BODY.PEEK[])"))?;
        let raw = reply
            .literals
            .into_iter()
            .next()
            .ok_or("IMAP FETCH returned no body literal")?;
        if bytes + raw.len() > MAX_BATCH {
            break;
        }
        bytes += raw.len();
        downloads.push(Download { key, raw });
        if downloads.len() >= 200 || bytes >= MAX_BATCH {
            break;
        }
    }
    // LOGOUT often sends BYE before the tagged OK; sending is enough here.
    let _ = s.session.send("R9999 LOGOUT");
    Ok(downloads)
}

fn imap_token(text: &str) -> Result<(String, &str)> {
    let text = text.trim_start();
    if let Some(text) = text.strip_prefix('"') {
        let mut result = String::new();
        let mut escape = false;
        for (position, character) in text.char_indices() {
            if escape {
                result.push(character);
                escape = false;
            } else if character == '\\' {
                escape = true;
            } else if character == '"' {
                return Ok((result, &text[position + 1..]));
            } else {
                result.push(character);
            }
        }
        return Err("Malformed quoted IMAP mailbox".into());
    }
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    if end == 0 {
        return Err("Missing IMAP mailbox name".into());
    }
    Ok((text[..end].into(), &text[end..]))
}
mod auth;
mod changes;
pub use changes::{imap_move, imap_permanently_delete, imap_restore, imap_trash};

mod sync;
pub use sync::{
    ImapDownload, ImapFlags, ImapFolder, ImapFolderRole, ImapMailboxSnapshot, ImapSyncReport,
    ImapSyncRequest, PendingFlagChange, sync_imap,
};
