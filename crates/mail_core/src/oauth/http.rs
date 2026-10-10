use super::{TokenResponse, endpoint};
use crate::Result;
use std::time::Duration;
use ureq::{
    Agent,
    tls::{RootCerts, TlsConfig, TlsProvider},
};

pub(super) struct TokenClient {
    agent: Agent,
    #[cfg(test)]
    plaintext_loopback: bool,
}
impl TokenClient {
    pub fn new() -> Self {
        Self {
            agent: Agent::config_builder()
                .tls_config(
                    TlsConfig::builder()
                        .provider(TlsProvider::NativeTls)
                        .root_certs(RootCerts::PlatformVerifier)
                        .build(),
                )
                .https_only(true)
                .max_redirects(0)
                .max_redirects_will_error(false)
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(30)))
                .timeout_connect(Some(Duration::from_secs(10)))
                .timeout_resolve(Some(Duration::from_secs(10)))
                .max_response_header_size(16384)
                .build()
                .into(),
            #[cfg(test)]
            plaintext_loopback: false,
        }
    }
    #[cfg(test)]
    pub fn for_loopback_fixture() -> Self {
        // This bypass exists only in test binaries. The public configuration
        // still requires HTTPS; a local fixture supplies the HTTP wire protocol.
        Self {
            agent: Agent::config_builder()
                .proxy(None)
                .max_redirects(0)
                .max_redirects_will_error(false)
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(3)))
                .build()
                .into(),
            plaintext_loopback: true,
        }
    }
    pub fn exchange(&self, target: &str, values: &[(String, String)]) -> Result<TokenResponse> {
        let url = endpoint(target)?;
        #[cfg(test)]
        let url = if self.plaintext_loopback {
            assert!(url.host_str().is_some_and(|host| {
                host.parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
            }));
            let mut local = url;
            local.set_scheme("http").unwrap();
            local
        } else {
            url
        };
        let mut response = self
            .agent
            .post(url.as_str())
            .header("Accept", "application/json")
            .send_form(
                values
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
            .map_err(|_| {
                "OAuth token request failed; check the connection and HTTPS certificate".to_string()
            })?;
        let status = response.status().as_u16();
        // Never follow a redirect while sending authorization codes or tokens.
        if (300..400).contains(&status) {
            return Err("OAuth token endpoint redirected; credentials were not forwarded".into());
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(65536)
            .read_to_vec()
            .map_err(|_| "OAuth token response is unavailable or too large".to_string())?;
        if !(200..300).contains(&status) {
            let error = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|json| {
                    json.get("error")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                })
                .filter(|value| {
                    value.len() <= 64
                        && value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                })
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(format!(
                "OAuth provider rejected the token request ({error}); sign in again if access was revoked"
            ));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| "OAuth token endpoint returned an invalid token response".into())
    }
}
