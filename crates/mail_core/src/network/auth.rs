//! Password and XOAUTH2 exchanges, isolated from mailbox and message commands.
use super::{Imap, Session, pop_ok, quoted};
use crate::{
    Result,
    account::{Account, AuthCredential},
    oauth,
};
use base64::{Engine, engine::general_purpose::STANDARD};

pub(super) fn smtp(
    session: &mut Session,
    account: &Account,
    capabilities: &[String],
) -> Result<()> {
    match account.network_auth()? {
        AuthCredential::Password(password) => {
            smtp_password(session, account, capabilities, &password)
        }
        AuthCredential::OAuth2(access_token) => {
            if !smtp_mechanisms(capabilities).iter().any(|m| m == "XOAUTH2") {
                return Err("SMTP server does not advertise AUTH XOAUTH2; password fallback is disabled for OAuth accounts".into());
            }
            let response = oauth::xoauth2_response(&account.username, &access_token)?;
            session.send(&format!("AUTH XOAUTH2 {response}"))?;
            let (status, _) = session.numeric()?;
            if status == 235 {
                return Ok(());
            }
            if status == 334 {
                // XOAUTH2 reports rejection as a SASL error challenge. Complete
                // it with an empty response and drain the final SMTP reply.
                session.send("")?;
                let _ = session.numeric()?;
            }
            Err("SMTP OAuth authentication failed; sign in again or check account access".into())
        }
    }
}
fn smtp_mechanisms(capabilities: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for line in capabilities {
        let Some(extension) = line.get(4..) else {
            continue;
        };
        let upper = extension.to_ascii_uppercase();
        let auth = upper
            .strip_prefix("AUTH ")
            .or_else(|| upper.strip_prefix("AUTH="));
        if let Some(auth) = auth {
            found.extend(auth.split_whitespace().map(str::to_owned));
        }
    }
    found
}
fn smtp_password(
    session: &mut Session,
    account: &Account,
    capabilities: &[String],
    password: &str,
) -> Result<()> {
    let mechanisms = smtp_mechanisms(capabilities);
    if mechanisms.iter().any(|m| m == "PLAIN") {
        let response = STANDARD.encode(format!("\0{}\0{password}", account.username));
        session.send(&format!("AUTH PLAIN {response}"))?;
        let (status, _) = session.numeric()?;
        if status == 334 {
            session.send(&response)?;
            session.expect(&[235])?;
        } else if status != 235 {
            return Err("SMTP authentication failed".into());
        }
    } else if mechanisms.iter().any(|m| m == "LOGIN") {
        session.send("AUTH LOGIN")?;
        session.expect(&[334])?;
        session.send(&STANDARD.encode(account.username.as_bytes()))?;
        session.expect(&[334])?;
        session.send(&STANDARD.encode(password.as_bytes()))?;
        session.expect(&[235])?;
    } else if !account.username.is_empty() {
        return Err("SMTP server does not advertise supported AUTH PLAIN/LOGIN".into());
    }
    Ok(())
}

pub(super) fn pop3(session: &mut Session, account: &Account) -> Result<()> {
    match account.network_auth()? {
        AuthCredential::Password(password) => {
            session.send(&format!("USER {}", account.username))?;
            pop_ok(session)?;
            session.send(&format!("PASS {password}"))?;
            pop_ok(session)?;
            Ok(())
        }
        AuthCredential::OAuth2(access_token) => {
            session.send("CAPA")?;
            let capability_status = session.line()?;
            if capability_status.starts_with("+OK") {
                let mut xoauth2 = false;
                let mut completed = false;
                for _ in 0..256 {
                    let line = session.line()?;
                    if line == "." {
                        completed = true;
                        break;
                    }
                    let mut parts = line.split_whitespace();
                    if parts.next().is_some_and(|p| p.eq_ignore_ascii_case("SASL")) {
                        xoauth2 |= parts.any(|p| p.eq_ignore_ascii_case("XOAUTH2"));
                    }
                }
                if !completed {
                    return Err("POP3 capability response exceeds limits".into());
                }
                if !xoauth2 {
                    return Err("POP3 server does not advertise SASL XOAUTH2; password fallback is disabled for OAuth accounts".into());
                }
            } else if !capability_status.starts_with("-ERR") {
                return Err("Malformed POP3 capability response".into());
            }
            let response = oauth::xoauth2_response(&account.username, &access_token)?;
            session.send("AUTH XOAUTH2")?;
            let challenge = session.line()?;
            if !continuation(&challenge) {
                return Err("POP3 OAuth authentication failed".into());
            }
            session.send(&response)?;
            let reply = session.line()?;
            if reply.starts_with("+OK") {
                return Ok(());
            }
            if continuation(&reply) {
                session.send("")?;
                let _ = session.line()?;
            }
            Err("POP3 OAuth authentication failed; sign in again or check account access".into())
        }
    }
}
fn continuation(line: &str) -> bool {
    line == "+" || line.starts_with("+ ")
}

pub(super) fn imap(session: &mut Imap, account: &Account) -> Result<()> {
    match account.network_auth()? {
        AuthCredential::Password(password) => {
            session.command(&format!(
                "LOGIN {} {}",
                quoted(&account.username)?,
                quoted(&password)?
            ))?;
            Ok(())
        }
        AuthCredential::OAuth2(access_token) => {
            let capabilities = session.command("CAPABILITY")?;
            let mut xoauth2 = false;
            let mut initial_response = false;
            for line in &capabilities.lines {
                let mut parts = line.split_whitespace();
                if parts.next() != Some("*")
                    || !parts
                        .next()
                        .is_some_and(|p| p.eq_ignore_ascii_case("CAPABILITY"))
                {
                    continue;
                }
                for value in parts {
                    xoauth2 |= value.eq_ignore_ascii_case("AUTH=XOAUTH2");
                    initial_response |= value.eq_ignore_ascii_case("SASL-IR");
                }
            }
            if !xoauth2 {
                return Err("IMAP server does not advertise AUTH=XOAUTH2; password fallback is disabled for OAuth accounts".into());
            }
            let response = oauth::xoauth2_response(&account.username, &access_token)?;
            session.tag = session
                .tag
                .checked_add(1)
                .ok_or("IMAP tag counter exhausted")?;
            let tag = format!("R{:04}", session.tag);
            let command = if initial_response {
                format!("{tag} AUTHENTICATE XOAUTH2 {response}")
            } else {
                format!("{tag} AUTHENTICATE XOAUTH2")
            };
            session.session.send(&command)?;
            let mut response_sent = initial_response;
            let mut cancelled = false;
            let mut total = 0;
            for _ in 0..256 {
                let line = session.session.line()?;
                total += line.len();
                if total > 1024 * 1024 {
                    return Err("IMAP authentication response exceeds limits".into());
                }
                if continuation(&line) {
                    if cancelled {
                        return Err(
                            "IMAP server repeated a cancelled authentication challenge".into()
                        );
                    }
                    if !response_sent {
                        session.session.send(&response)?;
                        response_sent = true;
                    } else {
                        session.session.send("")?;
                        cancelled = true;
                    }
                    continue;
                }
                let mut parts = line.split_whitespace();
                match parts.next() {
                    Some(value) if value == tag => {
                        if parts
                            .next()
                            .is_some_and(|value| value.eq_ignore_ascii_case("OK"))
                            && response_sent
                            && !cancelled
                        {
                            return Ok(());
                        }
                        return Err("IMAP OAuth authentication failed; sign in again or check account access".into());
                    }
                    Some("*")
                        if !line.to_ascii_uppercase().starts_with("* BYE")
                            && !line.ends_with('}') => {}
                    _ => return Err("Invalid IMAP authentication response".into()),
                }
            }
            Err("IMAP authentication response has too many lines".into())
        }
    }
}

#[cfg(test)]
mod tests;
