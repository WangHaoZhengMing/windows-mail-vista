//! Safe UID-scoped IMAP mutations.
use super::*;

mod destination;
mod fetch;
use destination::{destination_location, destination_proof};
pub(super) use fetch::fetch_uid_body;

pub(super) fn capabilities(s: &mut Imap) -> Result<Vec<String>> {
    let reply = s.command("CAPABILITY")?;
    Ok(reply
        .lines
        .iter()
        .filter_map(|line| line.strip_prefix("* CAPABILITY "))
        .flat_map(|line| line.split_whitespace().map(str::to_ascii_uppercase))
        .collect())
}
fn validate_location(account: &Account, location: &ImapLocation) -> Result<()> {
    if location.connection != account.id() {
        return Err("Message belongs to earlier server settings; remote operation refused".into());
    }
    if location.uid == 0 || location.uid_validity == 0 {
        return Err("Invalid remote message identity".into());
    }
    Ok(())
}
fn select_location(s: &mut Imap, account: &Account, location: &ImapLocation) -> Result<()> {
    validate_location(account, location)?;
    let selected = s.command(&format!("SELECT {}", quoted(&location.mailbox)?))?;
    if uid_validity(&selected)? != location.uid_validity {
        return Err("Server mailbox UIDVALIDITY changed; remote operation refused".into());
    }
    if selected
        .completion
        .to_ascii_uppercase()
        .contains("[READ-ONLY]")
    {
        return Err("Server mailbox is read-only".into());
    }
    Ok(())
}

/// Prefer the server's special-use Trash while keeping its wire encoding intact.
fn trash_mailbox(s: &mut Imap) -> Result<String> {
    let reply = s.command("LIST \"\" \"*\"")?;
    let mut literal = reply.literals.iter();
    let mut conventional = None;
    for line in &reply.lines {
        let Some(rest) = line.strip_prefix("* LIST (") else {
            continue;
        };
        let Some((attributes, rest)) = rest.split_once(')') else {
            return Err("Malformed IMAP LIST".into());
        };
        let (_, rest) = imap_token(rest)?; // hierarchy delimiter
        let (token, _) = imap_token(rest)?;
        let mailbox = if token.starts_with('{') {
            String::from_utf8(
                literal
                    .next()
                    .ok_or("Missing LIST mailbox literal")?
                    .clone(),
            )
            .map_err(|_| "Mailbox name is not valid UTF-8")?
        } else {
            token
        };
        if attributes
            .split_whitespace()
            .any(|flag| flag.eq_ignore_ascii_case("\\Noselect"))
        {
            continue;
        }
        if attributes
            .split_whitespace()
            .any(|flag| flag.eq_ignore_ascii_case("\\Trash"))
        {
            return Ok(mailbox);
        }
        if [
            "Trash",
            "Deleted Items",
            "Deleted Messages",
            "[Gmail]/Trash",
        ]
        .iter()
        .any(|name| mailbox.eq_ignore_ascii_case(name))
        {
            conventional.get_or_insert(mailbox);
        }
    }
    if let Some(mailbox) = conventional {
        return Ok(mailbox);
    }
    // Creating a dedicated destination is part of the requested move to Trash.
    // A rejected CREATE can be a concurrent creation; LIST again to prove it exists.
    if s.command("CREATE \"Trash\"").is_err() {
        let listed = s.command("LIST \"\" \"Trash\"")?;
        if !listed.lines.iter().any(|line| {
            line.starts_with("* LIST (")
                && !line.to_ascii_uppercase().contains("\\NOSELECT")
                && line
                    .rsplit_once(' ')
                    .is_some_and(|(_, name)| name.trim_matches('"').eq_ignore_ascii_case("Trash"))
        }) {
            return Err("Server has no Trash mailbox and creating it was refused".into());
        }
    }
    Ok("Trash".into())
}
fn supports(capabilities: &[String], name: &str) -> bool {
    capabilities
        .iter()
        .any(|value| value.eq_ignore_ascii_case(name))
}
fn targeted_expunge(capabilities: &[String]) -> bool {
    supports(capabilities, "UIDPLUS") || supports(capabilities, "IMAP4REV2")
}

fn move_uid(
    s: &mut Imap,
    account: &Account,
    source: &ImapLocation,
    target: &str,
    capabilities: &[String],
) -> Result<ImapChange> {
    if source.mailbox == target
        || (source.mailbox.eq_ignore_ascii_case("INBOX") && target.eq_ignore_ascii_case("INBOX"))
    {
        return Err("Source and destination mailbox are the same".into());
    }
    let proof = if targeted_expunge(capabilities) {
        None
    } else {
        Some(destination_proof(s, source, target)?)
    };
    if supports(capabilities, "MOVE") || supports(capabilities, "IMAP4REV2") {
        let moved = match s.command(&format!("UID MOVE {} {}", source.uid, quoted(target)?)) {
            Ok(reply) => reply,
            Err(error) => {
                let error: String = error.into();
                // RFC 6851 permits a partial MOVE even when the tagged result is NO.
                return Ok(ImapChange::Uncertain(format!(
                    "Server may already have moved the message; destination UID is unavailable: {error}. Further synchronized changes are blocked for this copy."
                )));
            }
        };
        return Ok(
            match destination_location(s, &moved, source, target, proof.as_ref()) {
                Ok((location, _)) => ImapChange::Moved(location),
                Err(error) => ImapChange::Uncertain(format!(
                    "Server reported a completed move, but the destination UID is unavailable: {error}. Further synchronized changes are blocked for this copy."
                )),
            },
        );
    }
    let copied = match s.command(&format!("UID COPY {} {}", source.uid, quoted(target)?)) {
        Ok(reply) => reply,
        Err(ImapError::Rejected) => return Err(ImapError::Rejected.into()),
        Err(ImapError::Failure(error)) => {
            return Ok(ImapChange::Uncertain(format!(
                "Server may already have copied the message, but the move did not complete: {error}. Further synchronized changes are blocked to avoid duplicate copies."
            )));
        }
    };
    let (destination, changed_selection) = match destination_location(
        s,
        &copied,
        source,
        target,
        proof.as_ref(),
    ) {
        Ok(location) => location,
        Err(error) => {
            return Ok(ImapChange::Uncertain(format!(
                "Server reported a completed copy, but the move did not complete because the destination UID is unavailable: {error}. The original was not marked deleted. Further synchronized changes are blocked to avoid duplicate copies."
            )));
        }
    };
    if changed_selection && let Err(error) = select_location(s, account, source) {
        return Ok(ImapChange::Uncertain(format!(
            "Destination copy exists, but the move did not complete because the original mailbox identity changed: {error}. Further synchronized changes are blocked to avoid duplicate copies."
        )));
    }
    if let Err(error) = s.command(&format!(
        "UID STORE {} +FLAGS.SILENT (\\Deleted)",
        source.uid
    )) {
        let error: String = error.into();
        return Ok(ImapChange::Uncertain(format!(
            "Destination copy exists, but the move did not complete because marking the original failed: {error}. Further synchronized changes are blocked to avoid duplicate copies."
        )));
    }
    if !targeted_expunge(capabilities) {
        return Ok(ImapChange::MovedDeferred {
            destination,
            original: source.clone(),
        });
    }
    match s.command(&format!("UID EXPUNGE {}", source.uid)) {
        Ok(_) => Ok(ImapChange::Moved(destination)),
        Err(ImapError::Rejected) => Ok(ImapChange::MovedDeferred {
            destination,
            original: source.clone(),
        }),
        Err(ImapError::Failure(error)) => Ok(ImapChange::Uncertain(format!(
            "Destination copy exists and the server may already have expunged the original: {error}. Further synchronized changes are blocked for this copy."
        ))),
    }
}
pub fn imap_trash(account: &Account, source: &ImapLocation) -> Result<ImapChange> {
    validate_location(account, source)?;
    let mut s = imap_connect(account)?;
    let capabilities = capabilities(&mut s)?;
    let target = trash_mailbox(&mut s)?;
    select_location(&mut s, account, source)?;
    let result = move_uid(&mut s, account, source, &target, &capabilities);
    let _ = s.session.send("R9999 LOGOUT");
    result
}
/// Move to the server's exact mailbox name, including nonstandard folders.
fn move_message(
    account: &Account,
    source: &ImapLocation,
    target: &str,
    restoring: bool,
) -> Result<ImapChange> {
    validate_location(account, source)?;
    quoted(target)?;
    let mut s = imap_connect(account)?;
    let capabilities = capabilities(&mut s)?;
    select_location(&mut s, account, source)?;
    if restoring {
        match s.command(&format!(
            "UID STORE {} -FLAGS.SILENT (\\Deleted)",
            source.uid
        )) {
            Ok(_) => {}
            Err(ImapError::Rejected) => {
                return Err("Server rejected restoring the message's Deleted flag".into());
            }
            Err(ImapError::Failure(error)) => {
                return Ok(ImapChange::Uncertain(format!(
                    "Server may have cleared the message's Deleted flag, but restoring did not complete: {error}. Resynchronize before trying again."
                )));
            }
        }
    }
    let result = move_uid(&mut s, account, source, target, &capabilities);
    let _ = s.session.send("R9999 LOGOUT");
    result
}
pub fn imap_move(account: &Account, source: &ImapLocation, target: &str) -> Result<ImapChange> {
    move_message(account, source, target, false)
}
pub fn imap_restore(account: &Account, source: &ImapLocation, target: &str) -> Result<ImapChange> {
    move_message(account, source, target, true)
}
pub fn imap_permanently_delete(account: &Account, source: &ImapLocation) -> Result<ImapChange> {
    validate_location(account, source)?;
    let mut s = imap_connect(account)?;
    let capabilities = capabilities(&mut s)?;
    select_location(&mut s, account, source)?;
    let result = match s.command(&format!(
        "UID STORE {} +FLAGS.SILENT (\\Deleted)",
        source.uid
    )) {
        Err(ImapError::Rejected) => Err("Server rejected marking the message deleted".into()),
        Err(ImapError::Failure(error)) => Ok(ImapChange::Uncertain(format!(
            "Server may already have marked the message deleted: {error}. Further synchronized changes require resynchronization."
        ))),
        Ok(_) if !targeted_expunge(&capabilities) => Ok(ImapChange::MarkedDeleted(source.clone())),
        Ok(_) => match s.command(&format!("UID EXPUNGE {}", source.uid)) {
            Ok(_) => Ok(ImapChange::Deleted),
            Err(ImapError::Rejected) => Ok(ImapChange::MarkedDeleted(source.clone())),
            Err(ImapError::Failure(error)) => Ok(ImapChange::Uncertain(format!(
                "Server may already have expunged the message: {error}. Further synchronized changes are blocked for this copy."
            ))),
        },
    };
    let _ = s.session.send("R9999 LOGOUT");
    result
}
