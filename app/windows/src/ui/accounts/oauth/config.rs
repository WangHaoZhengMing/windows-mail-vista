//! Provider form conversion. Draft tokens survive only unchanged credentials.
use mail_core::{
    Result,
    oauth::{LoopbackHost, OAuthAccount, OAuthConfig, OAuthProvider},
};

pub(super) struct ConfigInput {
    pub provider: OAuthProvider,
    pub client_id: String,
    pub client_secret: String,
    pub clear_secret: bool,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub scopes: String,
    pub loopback_host: LoopbackHost,
}
pub(super) fn preset(provider: OAuthProvider, client_id: String) -> OAuthConfig {
    match provider {
        OAuthProvider::Microsoft => OAuthConfig::microsoft(client_id),
        OAuthProvider::Google => OAuthConfig::google(client_id),
        OAuthProvider::Custom => OAuthConfig {
            provider,
            client_id,
            client_secret_dpapi: String::new(),
            authorization_endpoint: String::new(),
            token_endpoint: String::new(),
            scopes: vec![],
            loopback_host: LoopbackHost::IpAddress,
        },
    }
}
fn same_config(left: &OAuthConfig, right: &OAuthConfig) -> bool {
    left.provider == right.provider
        && left.client_id == right.client_id
        && left.client_secret_dpapi == right.client_secret_dpapi
        && left.authorization_endpoint == right.authorization_endpoint
        && left.token_endpoint == right.token_endpoint
        && left.scopes == right.scopes
        && left.loopback_host == right.loopback_host
}
impl ConfigInput {
    pub fn account(&self, previous: &OAuthAccount) -> Result<OAuthAccount> {
        let mut config = preset(self.provider, self.client_id.trim().into());
        if self.provider == OAuthProvider::Custom {
            config.authorization_endpoint = self.authorization_endpoint.trim().into();
            config.token_endpoint = self.token_endpoint.trim().into();
            config.scopes = self.scopes.split_whitespace().map(str::to_owned).collect();
            config.loopback_host = self.loopback_host;
        }
        if self.provider != OAuthProvider::Microsoft {
            if !self.client_secret.is_empty() {
                config.set_client_secret(&self.client_secret)?;
            } else if !self.clear_secret
                && previous.config.provider == self.provider
                && previous.config.client_id == config.client_id
                && previous.config.authorization_endpoint == config.authorization_endpoint
                && previous.config.token_endpoint == config.token_endpoint
            {
                config.client_secret_dpapi = previous.config.client_secret_dpapi.clone();
            }
        }
        config.validate()?;
        Ok(OAuthAccount {
            tokens: same_config(&config, &previous.config)
                .then(|| previous.tokens.clone())
                .flatten(),
            config,
        })
    }
}
