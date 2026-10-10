//! Read and validate account properties before any persistence request.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum Field {
    Name,
    Email,
    Username,
    Password,
    IncomingHost,
    IncomingPort,
    SmtpHost,
    SmtpPort,
}

impl Field {
    pub(super) const COUNT: usize = 8;
    #[cfg(test)]
    pub(super) const ALL: [Self; Self::COUNT] = [
        Self::Name,
        Self::Email,
        Self::Username,
        Self::Password,
        Self::IncomingHost,
        Self::IncomingPort,
        Self::SmtpHost,
        Self::SmtpPort,
    ];

    pub(super) fn initial_text(self, previous: Option<&AccountRecord>) -> String {
        let account = previous.map(|record| &record.account);
        match (self, account) {
            (Self::Name, _) => previous
                .map(|record| record.name.clone())
                .unwrap_or_default(),
            (Self::Email, Some(account)) => account.email.clone(),
            (Self::Username, Some(account)) => account.username.clone(),
            (Self::IncomingHost, Some(account)) => account.incoming_host.clone(),
            (Self::IncomingPort, Some(account)) => account.incoming_port.to_string(),
            (Self::SmtpHost, Some(account)) => account.smtp_host.clone(),
            (Self::SmtpPort, Some(account)) => account.smtp_port.to_string(),
            (Self::IncomingPort, None) => "993".into(),
            (Self::SmtpPort, None) => "587".into(),
            _ => String::new(),
        }
    }
}

pub(super) struct FormInput {
    pub(super) fields: [String; Field::COUNT],
    pub(super) authentication: Authentication,
    pub(super) incoming: Incoming,
    pub(super) incoming_security: Security,
    pub(super) smtp_security: Security,
}

#[derive(Debug)]
pub(super) struct ValidationError {
    pub(super) field: Field,
    pub(super) message: String,
}

impl ValidationError {
    pub(super) fn new(field: Field, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

impl FormInput {
    pub(super) fn account(
        &self,
        previous: Option<&AccountRecord>,
    ) -> std::result::Result<(String, Account), ValidationError> {
        let value = |field: Field| self.fields[field as usize].trim().to_owned();
        let email = value(Field::Email);
        if !matches!(crate::message::addresses(&email), Ok(addresses) if addresses.len() == 1) {
            return Err(ValidationError::new(
                Field::Email,
                "Enter one valid email address.",
            ));
        }
        let name = value(Field::Name);
        let name = if name.is_empty() { email.clone() } else { name };
        if name.chars().any(char::is_control) {
            return Err(ValidationError::new(
                Field::Name,
                "Account name cannot contain control characters.",
            ));
        }
        let username = value(Field::Username);
        if username.is_empty() || username.chars().any(char::is_control) {
            return Err(ValidationError::new(
                Field::Username,
                "Enter the username required by your mail provider.",
            ));
        }
        let host = |field| {
            let host = value(field);
            if host.is_empty()
                || host
                    .chars()
                    .any(|character| character.is_whitespace() || character.is_control())
            {
                Err(ValidationError::new(
                    field,
                    "Enter a server hostname without spaces.",
                ))
            } else {
                Ok(host)
            }
        };
        let incoming_host = host(Field::IncomingHost)?;
        let smtp_host = host(Field::SmtpHost)?;
        let port = |field| {
            value(field)
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| {
                    ValidationError::new(field, "Server port must be a number from 1 to 65535.")
                })
        };
        let incoming_port = port(Field::IncomingPort)?;
        let smtp_port = port(Field::SmtpPort)?;
        for (host, security, field) in [
            (&incoming_host, self.incoming_security, Field::IncomingHost),
            (&smtp_host, self.smtp_security, Field::SmtpHost),
        ] {
            if security == Security::Plain && !is_localhost(host) {
                return Err(ValidationError::new(
                    field,
                    "Choose TLS or STARTTLS for this server. Unencrypted connections require localhost.",
                ));
            }
        }
        let password = &self.fields[Field::Password as usize];
        let password_auth = matches!(self.authentication, Authentication::Password);
        let saved_password = previous.filter(|record| {
            matches!(record.account.auth, Authentication::Password)
                && !record.account.password_dpapi.is_empty()
        });
        if password_auth && password.is_empty() && saved_password.is_none() {
            return Err(ValidationError::new(
                Field::Password,
                "Enter your account password or provider app password.",
            ));
        }
        if let Authentication::OAuth2(oauth) = &self.authentication {
            oauth
                .config
                .validate()
                .map_err(|message| ValidationError::new(Field::Password, message))?;
            if self.incoming_security == Security::Plain || self.smtp_security == Security::Plain {
                return Err(ValidationError::new(
                    Field::IncomingHost,
                    "OAuth sign-in requires TLS or STARTTLS for incoming mail and SMTP.",
                ));
            }
        }
        let mut account = Account {
            auth: self.authentication.clone(),
            email,
            username,
            password_dpapi: if password_auth {
                saved_password
                    .map(|record| record.account.password_dpapi.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            },
            incoming: self.incoming,
            incoming_host,
            incoming_port,
            incoming_security: self.incoming_security,
            smtp_host,
            smtp_port,
            smtp_security: self.smtp_security,
        };
        if password_auth && !password.is_empty() {
            account
                .set_password(password)
                .map_err(|message| ValidationError::new(Field::Password, message))?;
        }
        account
            .validate()
            .map_err(|message| ValidationError::new(Field::Email, message))?;
        Ok((name, account))
    }
}

fn is_localhost(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}
