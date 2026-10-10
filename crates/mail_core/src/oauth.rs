//! Native OAuth 2.0 authorization-code flow with PKCE and a loopback callback.
//!
//! Provider application registrations belong to the application owner. No client
//! IDs, application passwords or user tokens are bundled into this library.

mod callback;
mod http;
#[cfg(test)]
mod tests;

use crate::{
    Result,
    account::{Account, Authentication, protect, unprotect},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use url::Url;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OAuthProvider {
    Microsoft,
    Google,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoopbackHost {
    #[default]
    IpAddress,
    Localhost,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub provider: OAuthProvider,
    pub client_id: String,
    /// Optional Google desktop credential, protected by the OS like user tokens.
    #[serde(default)]
    pub client_secret_dpapi: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub scopes: Vec<String>,
    #[serde(default)]
    pub loopback_host: LoopbackHost,
}

impl OAuthConfig {
    pub fn microsoft(client_id: impl Into<String>) -> Self {
        Self {
            provider: OAuthProvider::Microsoft,
            client_id: client_id.into(),
            client_secret_dpapi: String::new(),
            authorization_endpoint:
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
            token_endpoint: "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
            scopes: vec![
                "offline_access".into(),
                "https://outlook.office.com/IMAP.AccessAsUser.All".into(),
                "https://outlook.office.com/POP.AccessAsUser.All".into(),
                "https://outlook.office.com/SMTP.Send".into(),
            ],
            loopback_host: LoopbackHost::Localhost,
        }
    }
    pub fn google(client_id: impl Into<String>) -> Self {
        Self {
            provider: OAuthProvider::Google,
            client_id: client_id.into(),
            client_secret_dpapi: String::new(),
            authorization_endpoint: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_endpoint: "https://oauth2.googleapis.com/token".into(),
            scopes: vec!["https://mail.google.com/".into()],
            loopback_host: LoopbackHost::IpAddress,
        }
    }
    pub fn validate(&self) -> Result<()> {
        checked_value(&self.client_id, "OAuth client ID", 1024)?;
        if self.client_id.bytes().any(|b| b.is_ascii_whitespace()) {
            return Err("OAuth client ID cannot contain whitespace".into());
        }
        if self.scopes.is_empty()
            || self
                .scopes
                .iter()
                .any(|scope| scope.is_empty() || scope.bytes().any(|b| b <= 32 || b == 127))
        {
            return Err("OAuth scopes must be nonempty values without whitespace".into());
        }
        if self.scopes.len() > 64 || self.scopes.iter().map(String::len).sum::<usize>() > 8192 {
            return Err("OAuth scope configuration is too large".into());
        }
        endpoint(&self.authorization_endpoint)?;
        endpoint(&self.token_endpoint)?;
        if self.provider == OAuthProvider::Microsoft && !self.client_secret_dpapi.is_empty() {
            return Err(
                "Microsoft desktop OAuth uses a public client ID without a client secret".into(),
            );
        }
        Ok(())
    }
    pub fn set_client_secret(&mut self, value: &str) -> Result<()> {
        if value.is_empty() {
            self.client_secret_dpapi.clear();
            return Ok(());
        }
        checked_value(value, "OAuth client secret", 8192)?;
        self.client_secret_dpapi = protect(value.as_bytes())?;
        Ok(())
    }
    pub fn has_client_secret(&self) -> bool {
        !self.client_secret_dpapi.is_empty()
    }
    fn client_secret(&self) -> Result<Option<String>> {
        if self.client_secret_dpapi.is_empty() {
            return Ok(None);
        }
        String::from_utf8(unprotect(&self.client_secret_dpapi)?)
            .map(Some)
            .map_err(|_| "Stored OAuth client credential is invalid".into())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    /// An encrypted JSON envelope containing both access and refresh tokens.
    pub protected: String,
    pub expires_at: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthAccount {
    pub config: OAuthConfig,
    #[serde(default)]
    pub tokens: Option<OAuthTokens>,
}
impl OAuthAccount {
    pub fn access_token(&self) -> Result<String> {
        let tokens = self
            .tokens
            .as_ref()
            .ok_or("Sign in to this OAuth account first")?;
        if tokens.expires_at <= now().saturating_add(60) {
            return Err("OAuth access token expired; refresh the account before connecting".into());
        }
        Ok(tokens.unseal()?.access_token)
    }
    pub fn signed_in(&self) -> bool {
        self.tokens.is_some()
    }
    pub fn sign_out(&mut self) {
        self.tokens = None;
    }
}

#[derive(Serialize, Deserialize)]
struct TokenPayload {
    access_token: String,
    refresh_token: Option<String>,
}
impl OAuthTokens {
    fn unseal(&self) -> Result<TokenPayload> {
        let value: TokenPayload = serde_json::from_slice(&unprotect(&self.protected)?)
            .map_err(|_| "Stored OAuth tokens are invalid".to_string())?;
        checked_token(&value.access_token)?;
        Ok(value)
    }
}

/// Complete on a worker thread; the callback must open the system browser.
/// Cancellation is checked during the loopback wait and around bounded HTTP I/O.
pub fn authorize(
    config: &OAuthConfig,
    login_hint: &str,
    cancel: &AtomicBool,
    open_browser: impl FnOnce(&str) -> Result<()>,
) -> Result<OAuthAccount> {
    authorize_with_client(
        config,
        login_hint,
        cancel,
        open_browser,
        &http::TokenClient::new(),
    )
}
fn authorize_with_client(
    config: &OAuthConfig,
    login_hint: &str,
    cancel: &AtomicBool,
    open_browser: impl FnOnce(&str) -> Result<()>,
    client: &http::TokenClient,
) -> Result<OAuthAccount> {
    config.validate()?;
    check_cancel(cancel)?;
    let session = callback::Session::start(config, login_hint)?;
    open_browser(session.authorization_url())?;
    let grant = session.wait(cancel, Duration::from_secs(300))?;
    let mut values = vec![
        ("grant_type".into(), "authorization_code".into()),
        ("client_id".into(), config.client_id.clone()),
        ("code".into(), grant.code),
        ("redirect_uri".into(), grant.redirect_uri),
        ("code_verifier".into(), grant.verifier),
    ];
    add_client_secret(config, &mut values)?;
    check_cancel(cancel)?;
    let response = client.exchange(&config.token_endpoint, &values)?;
    check_cancel(cancel)?;
    Ok(OAuthAccount {
        config: config.clone(),
        tokens: Some(seal_response(response, None)?),
    })
}

/// Refreshes an expired token in place, including any rotated refresh token.
/// Persist `account` before starting mail I/O, even if that mail operation fails.
pub fn access_token(account: &mut Account) -> Result<String> {
    refresh_with_client(account, false, &http::TokenClient::new())
}
/// Force a refresh after a server has rejected a previously unexpired token.
pub fn refresh_access_token(account: &mut Account) -> Result<String> {
    refresh_with_client(account, true, &http::TokenClient::new())
}
fn refresh_with_client(
    account: &mut Account,
    force: bool,
    client: &http::TokenClient,
) -> Result<String> {
    let Authentication::OAuth2(oauth) = &mut account.auth else {
        return Err("Account does not use OAuth".into());
    };
    oauth.config.validate()?;
    if !force && let Ok(token) = oauth.access_token() {
        return Ok(token);
    }
    let old = oauth
        .tokens
        .as_ref()
        .ok_or("Sign in to this OAuth account first")?
        .unseal()?;
    let refresh_token = old
        .refresh_token
        .ok_or("This OAuth account has no refresh token; sign in again")?;
    checked_value(&refresh_token, "OAuth refresh token", 32768)?;
    let mut values = vec![
        ("grant_type".into(), "refresh_token".into()),
        ("client_id".into(), oauth.config.client_id.clone()),
        ("refresh_token".into(), refresh_token.clone()),
    ];
    if oauth.config.provider != OAuthProvider::Google {
        values.push(("scope".into(), oauth.config.scopes.join(" ")));
    }
    add_client_secret(&oauth.config, &mut values)?;
    let response = client.exchange(&oauth.config.token_endpoint, &values)?;
    let tokens = seal_response(response, Some(refresh_token))?;
    let token = tokens.unseal()?.access_token;
    oauth.tokens = Some(tokens);
    Ok(token)
}

fn add_client_secret(config: &OAuthConfig, values: &mut Vec<(String, String)>) -> Result<()> {
    if let Some(secret) = config.client_secret()? {
        values.push(("client_secret".into(), secret));
    }
    Ok(())
}
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
}
fn seal_response(response: TokenResponse, previous_refresh: Option<String>) -> Result<OAuthTokens> {
    if !response.token_type.eq_ignore_ascii_case("Bearer") {
        return Err("OAuth token endpoint returned an unsupported token type".into());
    }
    checked_token(&response.access_token)?;
    if response.expires_in == 0 {
        return Err("OAuth token endpoint returned an already expired token".into());
    }
    let refresh_token = response.refresh_token.or(previous_refresh);
    if let Some(refresh) = &refresh_token {
        checked_value(refresh, "OAuth refresh token", 32768)?;
    }
    let payload = TokenPayload {
        access_token: response.access_token,
        refresh_token,
    };
    Ok(OAuthTokens {
        protected: protect(&serde_json::to_vec(&payload).map_err(|e| e.to_string())?)?,
        expires_at: now().saturating_add(response.expires_in),
    })
}
/// SASL XOAUTH2 initial response, suitable for IMAP, POP3 and SMTP.
pub fn xoauth2_response(username: &str, token: &str) -> Result<String> {
    checked_value(username, "OAuth username", 1024)?;
    checked_token(token)?;
    Ok(STANDARD.encode(format!("user={username}\x01auth=Bearer {token}\x01\x01")))
}
fn checked_token(token: &str) -> Result<()> {
    checked_value(token, "OAuth access token", 32768)?;
    if token.bytes().any(|b| b.is_ascii_whitespace()) {
        return Err("OAuth access token contains whitespace".into());
    }
    Ok(())
}
fn checked_value(value: &str, label: &str, max: usize) -> Result<()> {
    if value.is_empty() || value.len() > max || value.bytes().any(|b| b < 32 || b == 127) {
        return Err(format!(
            "{label} is empty, too large or contains control characters"
        ));
    }
    Ok(())
}
fn endpoint(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| "OAuth endpoint URL is invalid")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(
            "OAuth endpoints must use HTTPS without credentials, queries or fragments".into(),
        );
    }
    Ok(url)
}
fn random_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("Secure random generation failed: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err("OAuth sign-in cancelled".into())
    } else {
        Ok(())
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
