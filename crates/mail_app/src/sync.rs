//! Credential refresh handshake keeps rotated tokens durable before mail I/O.
use super::mail::WorkerResult;
use mail_core::{
    Result,
    account::{Account, AccountId, Authentication},
};
use std::{sync::mpsc, time::Duration};

pub(super) fn refresh_credentials(
    id: &AccountId,
    account: &mut Account,
    sender: &mpsc::Sender<WorkerResult>,
) -> Result<()> {
    if !matches!(account.auth, Authentication::OAuth2(_)) {
        return Ok(());
    }
    mail_core::oauth::access_token(account)?;
    let (reply, receiver) = mpsc::channel();
    sender
        .send(WorkerResult::Credentials {
            account: id.clone(),
            connection: account.id(),
            authentication: account.auth.clone(),
            reply,
        })
        .map_err(|_| "Mail window closed during authentication".to_string())?;
    receiver
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| {
            "Timed out saving refreshed credentials; mail operation was not started".to_string()
        })?
}
