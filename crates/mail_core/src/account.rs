use crate::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

/// Catalog identity, independent of server settings and credentials.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(String);

impl AccountId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(crate::message::hash(
            format!("{time}:{}:{next}", std::process::id()).as_bytes(),
        ))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AccountRecord {
    pub id: AccountId,
    pub name: String,
    pub account: Account,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    Tls,
    StartTls,
    Plain,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Incoming {
    Imap,
    Pop3,
}

/// Authentication remains password-based when loading older account catalogs.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(tag = "method", content = "oauth", rename_all = "snake_case")]
pub enum Authentication {
    #[default]
    Password,
    #[serde(rename = "oauth2")]
    OAuth2(crate::oauth::OAuthAccount),
}

/// Ephemeral credentials for a TLS-protected protocol connection.
pub enum AuthCredential {
    Password(String),
    OAuth2(String),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Account {
    #[serde(default)]
    pub auth: Authentication,
    pub email: String,
    pub username: String,
    /// Keep the original catalog key; secret protection belongs to the OS backend.
    pub password_dpapi: String,
    pub incoming: Incoming,
    pub incoming_host: String,
    pub incoming_port: u16,
    pub incoming_security: Security,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: Security,
}
impl Account {
    pub fn validate(&self) -> Result<()> {
        if crate::message::addresses(&self.email)?.len() != 1 {
            return Err("Account must have exactly one sender address".into());
        }
        for s in [&self.username, &self.incoming_host, &self.smtp_host] {
            if s.contains(['\r', '\n', '\0']) || s.is_empty() {
                return Err("Account fields cannot be empty or contain control characters".into());
            }
        }
        if self.incoming_port == 0 || self.smtp_port == 0 {
            return Err("Invalid server port".into());
        }
        if let Authentication::OAuth2(oauth) = &self.auth {
            oauth.config.validate()?;
            if self.incoming_security == Security::Plain || self.smtp_security == Security::Plain {
                return Err("OAuth authentication requires TLS or STARTTLS".into());
            }
        }
        Ok(())
    }
    pub fn network_auth(&self) -> Result<AuthCredential> {
        match &self.auth {
            Authentication::Password => self.password().map(AuthCredential::Password),
            Authentication::OAuth2(oauth) => oauth.access_token().map(AuthCredential::OAuth2),
        }
    }
    pub fn set_password(&mut self, password: &str) -> Result<()> {
        if password.contains(['\r', '\n', '\0']) {
            return Err("Password contains unsupported control characters".into());
        }
        self.password_dpapi = protect(password.as_bytes())?;
        Ok(())
    }
    pub fn password(&self) -> Result<String> {
        String::from_utf8(unprotect(&self.password_dpapi)?).map_err(|e| e.to_string())
    }
    /// Connection fingerprint for remote UID keys; not a catalog account identity.
    pub fn id(&self) -> String {
        crate::message::hash(
            format!(
                "{}|{}|{}|{}",
                self.email, self.incoming_host, self.incoming_port, self.username
            )
            .as_bytes(),
        )
    }
}

pub fn protect(bytes: &[u8]) -> Result<String> {
    crate::platform::protect_secret(bytes).map(|bytes| STANDARD.encode(bytes))
}

pub fn unprotect(encoded: &str) -> Result<Vec<u8>> {
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|error| error.to_string())?;
    crate::platform::unprotect_secret(&bytes)
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(windows)]
    fn secrets_are_not_stored_as_plaintext() {
        let raw = b"test-password-\xe4\xbd\xa0";
        let protected = super::protect(raw).unwrap();
        assert!(!protected.contains("test-password"));
        assert_eq!(super::unprotect(&protected).unwrap(), raw);
    }
}
