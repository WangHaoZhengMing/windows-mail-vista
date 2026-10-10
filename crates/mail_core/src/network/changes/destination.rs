//! Resolve the destination without guessing based on Message-ID alone.
use super::*;

fn copy_location(reply: &ImapReply, source: &ImapLocation, mailbox: &str) -> Result<ImapLocation> {
    let code = reply
        .lines
        .iter()
        .chain(std::iter::once(&reply.completion))
        .find_map(|line| {
            line.to_ascii_uppercase()
                .find("[COPYUID ")
                .and_then(|position| line[position + 9..].split(']').next())
        })
        .ok_or("Server omitted COPYUID; remote destination identity is unavailable")?;
    let values: Vec<_> = code.split_whitespace().collect();
    if values.len() != 3 {
        return Err("Malformed COPYUID response".into());
    }
    let parse_uid = |value: &str| {
        let value = if let Some((first, last)) = value.split_once(':') {
            if first != last {
                return Err("COPYUID covers more than one message");
            }
            first
        } else {
            value
        };
        value
            .parse::<u32>()
            .ok()
            .filter(|uid| *uid != 0)
            .ok_or("Malformed COPYUID identity")
    };
    let uid_validity = parse_uid(values[0])?;
    let original = parse_uid(values[1])?;
    let uid = parse_uid(values[2])?;
    if original != source.uid {
        return Err("COPYUID does not match the selected message".into());
    }
    Ok(ImapLocation {
        connection: source.connection.clone(),
        mailbox: mailbox.into(),
        uid_validity,
        uid,
    })
}
/// Proof kept before the mutation when the server cannot provide COPYUID.
/// Message-ID is not a unique identity: compare complete raw content instead.
pub(super) struct DestinationProof {
    uid_validity: u32,
    uid_next: u32,
    raw: Vec<u8>,
}

fn fetch_raw(s: &mut Imap, uid: u32) -> Result<Vec<u8>> {
    let reply = s.command(&format!("UID FETCH {uid} (UID BODY.PEEK[])"))?;
    fetch_uid_body(&reply, uid)?
        .ok_or_else(|| "Source UID disappeared or FETCH identity is missing".into())
}

fn target_status(reply: &ImapReply, target: &str) -> Result<(u32, u32)> {
    if reply
        .lines
        .iter()
        .chain(std::iter::once(&reply.completion))
        .any(|line| line.to_ascii_uppercase().contains("[UIDNOTSTICKY]"))
    {
        return Err("Target mailbox UIDs are not persistent".into());
    }
    let mut validity = None;
    let mut next = None;
    let mut literals = reply.literals.iter();
    let mut lines = reply.lines.iter();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("* STATUS ") else {
            continue;
        };
        let (token, rest) = imap_token(rest)?;
        let (mailbox, values) = if token.starts_with('{') {
            let name = String::from_utf8(
                literals
                    .next()
                    .ok_or("Missing STATUS mailbox literal")?
                    .clone(),
            )
            .map_err(|_| "Invalid STATUS mailbox encoding")?;
            let values = if rest.trim().is_empty() {
                lines
                    .next()
                    .ok_or("Missing STATUS values after literal")?
                    .as_str()
            } else {
                rest
            };
            (name, values)
        } else {
            (token, rest)
        };
        if mailbox != target
            && !(mailbox.eq_ignore_ascii_case("INBOX") && target.eq_ignore_ascii_case("INBOX"))
        {
            continue;
        }
        let values: Vec<_> = values
            .trim()
            .trim_matches(['(', ')'])
            .split_whitespace()
            .collect();
        if !values.len().is_multiple_of(2) {
            return Err("Malformed STATUS values".into());
        }
        for pair in values.as_chunks::<2>().0 {
            let slot = if pair[0].eq_ignore_ascii_case("UIDVALIDITY") {
                &mut validity
            } else if pair[0].eq_ignore_ascii_case("UIDNEXT") {
                &mut next
            } else {
                continue;
            };
            let value = pair[1]
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or("Malformed STATUS UID identity")?;
            if slot
                .replace(value)
                .is_some_and(|previous| previous != value)
            {
                return Err("Conflicting STATUS UID identities".into());
            }
        }
    }
    Ok((
        validity.ok_or("Target STATUS omitted reliable UIDVALIDITY")?,
        next.ok_or("Target STATUS omitted reliable UIDNEXT")?,
    ))
}

pub(super) fn destination_proof(
    s: &mut Imap,
    source: &ImapLocation,
    target: &str,
) -> Result<DestinationProof> {
    let status = s.command(&format!("STATUS {} (UIDVALIDITY UIDNEXT)", quoted(target)?))?;
    let (validity, next) = target_status(&status, target)?;
    Ok(DestinationProof {
        uid_validity: validity,
        uid_next: next,
        raw: fetch_raw(s, source.uid)?,
    })
}
fn resolve_destination(
    s: &mut Imap,
    source: &ImapLocation,
    target: &str,
    proof: &DestinationProof,
) -> Result<ImapLocation> {
    // EXAMINE never clears or expunges another client's Deleted flags.
    let selected = s.command(&format!("EXAMINE {}", quoted(target)?))?;
    if uid_validity(&selected)? != proof.uid_validity {
        return Err("Target UIDVALIDITY changed during the move".into());
    }
    let found = s.command(&format!("UID SEARCH UID {}:*", proof.uid_next))?;
    let mut candidates = Vec::new();
    for line in found.lines {
        if let Some(values) = line.strip_prefix("* SEARCH") {
            for value in values.split_whitespace() {
                let uid = value
                    .parse::<u32>()
                    .map_err(|_| "Malformed destination UID")?;
                // n:* can include an older largest UID when n exceeds the mailbox's last UID.
                if uid >= proof.uid_next {
                    candidates.push(uid);
                }
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    if candidates.len() > 64 {
        return Err("Too many concurrent arrivals to prove the destination identity".into());
    }
    let mut found = None;
    let mut bytes = 0;
    for uid in candidates {
        let raw = fetch_raw(s, uid)?;
        bytes += raw.len();
        if bytes > MAX_BATCH {
            return Err("Destination identity scan exceeds 100 MiB".into());
        }
        if raw == proof.raw && found.replace(uid).is_some() {
            return Err(
                "Multiple identical new messages make the destination identity ambiguous".into(),
            );
        }
    }
    Ok(ImapLocation {
        connection: source.connection.clone(),
        mailbox: target.into(),
        uid_validity: proof.uid_validity,
        uid: found.ok_or("No unique exact-content destination message was found")?,
    })
}
pub(super) fn destination_location(
    s: &mut Imap,
    reply: &ImapReply,
    source: &ImapLocation,
    target: &str,
    proof: Option<&DestinationProof>,
) -> Result<(ImapLocation, bool)> {
    match copy_location(reply, source, target) {
        Ok(location) => Ok((location, false)),
        Err(error) => match proof {
            Some(proof) => {
                resolve_destination(s, source, target, proof).map(|location| (location, true))
            }
            None => Err(error),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_literal_keeps_mailbox_identity_and_ignores_unsolicited_other_status() {
        let reply = ImapReply {
            lines: vec![
                "* STATUS Other (UIDVALIDITY 888 UIDNEXT 17)".into(),
                "* STATUS {7}".into(),
                " (UIDNEXT 42 UIDVALIDITY 999)".into(),
            ],
            literals: vec![b"Archive".to_vec()],
            completion: "R0004 OK complete".into(),
        };
        assert_eq!(target_status(&reply, "Archive").unwrap(), (999, 42));
    }
    #[test]
    fn contradictory_or_ephemeral_target_ids_cannot_prove_a_copy() {
        let mut reply = ImapReply {
            lines: vec!["* STATUS Archive (UIDVALIDITY 999 UIDNEXT 42 UIDNEXT 43)".into()],
            literals: vec![],
            completion: "R0004 OK complete".into(),
        };
        assert!(target_status(&reply, "Archive").is_err());
        reply.lines = vec![
            "* STATUS Archive (UIDVALIDITY 999 UIDNEXT 42)".into(),
            "* NO [UIDNOTSTICKY] temporary ids".into(),
        ];
        assert!(
            target_status(&reply, "Archive")
                .unwrap_err()
                .contains("not persistent")
        );
    }
}
