use super::*;
use base64::engine::general_purpose::STANDARD;

pub(super) fn folders(reply: &ImapReply, utf8_names: bool) -> Result<Vec<ImapFolder>> {
    let mut result: Vec<ImapFolder> = Vec::new();
    let mut literals = reply.literals.iter();
    for line in &reply.lines {
        let Some(rest) = line.strip_prefix("* LIST ") else {
            continue;
        };
        let rest = rest.trim_start();
        let attributes_end = rest.find(')').ok_or("Malformed IMAP LIST attributes")?;
        let attributes = rest
            .strip_prefix('(')
            .ok_or("Malformed IMAP LIST attributes")?[..attributes_end - 1]
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        let (delimiter, rest) = imap_token(&rest[attributes_end + 1..])?;
        let mailbox = if rest.trim_start().starts_with('{') {
            String::from_utf8(
                literals
                    .next()
                    .ok_or("IMAP LIST mailbox literal missing")?
                    .clone(),
            )
            .map_err(|_| "IMAP LIST mailbox is not UTF-8")?
        } else {
            imap_token(rest)?.0
        };
        if mailbox.contains(['\r', '\n', '\0']) || mailbox.len() > 65536 {
            return Err("Invalid IMAP mailbox name".into());
        }
        let delimiter = if delimiter.eq_ignore_ascii_case("NIL") {
            None
        } else {
            let mut chars = delimiter.chars();
            let delimiter = chars.next().ok_or("Empty hierarchy delimiter")?;
            if chars.next().is_some() {
                return Err("Malformed hierarchy delimiter".into());
            }
            Some(delimiter)
        };
        let role = if mailbox.eq_ignore_ascii_case("INBOX") {
            Some(ImapFolderRole::Inbox)
        } else {
            [
                ("\\sent", ImapFolderRole::Sent),
                ("\\drafts", ImapFolderRole::Drafts),
                ("\\trash", ImapFolderRole::Trash),
                ("\\junk", ImapFolderRole::Junk),
                ("\\archive", ImapFolderRole::Archive),
                ("\\all", ImapFolderRole::All),
                ("\\important", ImapFolderRole::Important),
            ]
            .into_iter()
            .find_map(|(attribute, role)| {
                attributes
                    .iter()
                    .any(|value| value == attribute)
                    .then_some(role)
            })
            .or_else(|| conventional_role(&mailbox))
        };
        let folder = ImapFolder {
            display_name: if utf8_names {
                mailbox.clone()
            } else {
                decode_mailbox(&mailbox)?
            },
            mailbox,
            delimiter,
            selectable: !attributes
                .iter()
                .any(|value| matches!(value.as_str(), "\\noselect" | "\\nonexistent")),
            role,
        };
        if let Some(existing) = result
            .iter_mut()
            .find(|existing| same_mailbox(&existing.mailbox, &folder.mailbox))
        {
            *existing = folder;
        } else {
            result.push(folder);
        }
        if result.len() > 4096 {
            return Err("IMAP folder listing exceeds 4096 folders".into());
        }
    }
    result.sort_by(|a, b| {
        (!a.mailbox.eq_ignore_ascii_case("INBOX"))
            .cmp(&!b.mailbox.eq_ignore_ascii_case("INBOX"))
            .then(a.mailbox.cmp(&b.mailbox))
    });
    Ok(result)
}
fn conventional_role(mailbox: &str) -> Option<ImapFolderRole> {
    match mailbox.to_ascii_lowercase().as_str() {
        "sent" | "sent items" | "sent messages" => Some(ImapFolderRole::Sent),
        "drafts" | "draft" => Some(ImapFolderRole::Drafts),
        "trash" | "deleted items" | "deleted messages" => Some(ImapFolderRole::Trash),
        "junk" | "junk email" | "spam" => Some(ImapFolderRole::Junk),
        "archive" | "archives" => Some(ImapFolderRole::Archive),
        _ => None,
    }
}
fn decode_mailbox(mailbox: &str) -> Result<String> {
    // IMAP4rev1 uses modified UTF-7; actual UTF-8 names from UTF8-enabled servers
    // are already valid UTF-8. Preserve exact wire names separately for commands.
    if !mailbox.is_ascii() {
        return Ok(mailbox.into());
    }
    let mut decoded = String::new();
    let mut rest = mailbox;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let end = rest.find('-').ok_or("Malformed modified UTF-7 mailbox")?;
        if end == 0 {
            decoded.push('&');
        } else {
            let mut encoded = rest[..end].replace(',', "/");
            while !encoded.len().is_multiple_of(4) {
                encoded.push('=');
            }
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|_| "Malformed modified UTF-7 mailbox")?;
            if bytes.len() % 2 != 0 {
                return Err("Malformed modified UTF-7 UTF-16 length".into());
            }
            let units = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
                .collect::<Vec<_>>();
            decoded.push_str(
                &String::from_utf16(&units).map_err(|_| "Malformed modified UTF-7 UTF-16")?,
            );
        }
        rest = &rest[end + 1..];
    }
    decoded.push_str(rest);
    Ok(decoded)
}
pub(super) fn response_number(reply: &ImapReply, name: &str) -> Option<u32> {
    let prefix = format!("[{} ", name.to_ascii_uppercase());
    reply
        .lines
        .iter()
        .chain(std::iter::once(&reply.completion))
        .find_map(|line| {
            line.to_ascii_uppercase()
                .find(&prefix)
                .and_then(|position| {
                    line[position + prefix.len()..]
                        .split(']')
                        .next()?
                        .trim()
                        .parse()
                        .ok()
                })
        })
        .filter(|value| *value != 0)
}
pub(super) fn search_uids(reply: &ImapReply) -> Result<Vec<u32>> {
    let mut result = BTreeSet::new();
    let mut found = false;
    for line in &reply.lines {
        if let Some(values) = line.strip_prefix("* SEARCH") {
            found = true;
            for value in values.split_whitespace() {
                insert_uid_set(&mut result, value)?;
            }
        } else if let Some(values) = line.strip_prefix("* ESEARCH") {
            let mut rest = values.trim_start();
            if rest.starts_with('(') {
                // Correlator's quoted TAG is not a UID search result.
                let end = rest.find(')').ok_or("Malformed ESEARCH correlator")?;
                rest = rest[end + 1..].trim_start();
            }
            let tokens: Vec<_> = rest.split_whitespace().collect();
            if !tokens
                .first()
                .is_some_and(|token| token.eq_ignore_ascii_case("UID"))
            {
                return Err("ESEARCH returned sequence numbers instead of UIDs".into());
            }
            let mut all = false;
            let mut count = None;
            for pair in tokens[1..].chunks(2) {
                if pair.len() != 2 {
                    return Err("Malformed ESEARCH data".into());
                }
                if pair[0].eq_ignore_ascii_case("ALL") {
                    insert_uid_set(&mut result, pair[1])?;
                    all = true;
                }
                if pair[0].eq_ignore_ascii_case("COUNT") {
                    count = Some(
                        pair[1]
                            .parse::<usize>()
                            .map_err(|_| "Malformed ESEARCH count")?,
                    );
                }
            }
            if !all && count != Some(0) {
                return Err("ESEARCH omitted complete UID snapshot".into());
            }
            found = true;
        }
    }
    if !found {
        return Err("IMAP SEARCH omitted UID snapshot".into());
    }
    Ok(result.into_iter().collect())
}
fn insert_uid_set(result: &mut BTreeSet<u32>, set: &str) -> Result<()> {
    for range in set.split(',') {
        let mut parts = range.split(':');
        let start = parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value != 0)
            .ok_or("Malformed IMAP search UID")?;
        let end = match parts.next() {
            Some(value) => value
                .parse::<u32>()
                .ok()
                .filter(|value| *value != 0)
                .ok_or("Malformed IMAP search UID")?,
            None => start,
        };
        if parts.next().is_some() || start.abs_diff(end) > 1_000_000 {
            return Err("IMAP UID snapshot exceeds one million messages".into());
        }
        for uid in start.min(end)..=start.max(end) {
            result.insert(uid);
            if result.len() > 1_000_000 {
                return Err("IMAP UID snapshot exceeds one million messages".into());
            }
        }
    }
    Ok(())
}
pub(super) fn flags_from_reply(reply: &ImapReply) -> Result<BTreeMap<u32, ImapFlags>> {
    let mut result = BTreeMap::new();
    for line in &reply.lines {
        let upper = line.to_ascii_uppercase();
        if !upper.starts_with("* ") || !upper.contains(" FETCH (") {
            continue;
        }
        let (uid, flag_values) = metadata_attributes(line)?;
        let mut flags = ImapFlags::default();
        for value in flag_values.split_whitespace() {
            match value.to_ascii_lowercase().as_str() {
                "\\seen" => flags.seen = true,
                "\\flagged" => flags.flagged = true,
                "\\answered" => flags.answered = true,
                "\\deleted" => flags.deleted = true,
                "\\draft" => flags.draft = true,
                "\\recent" => (),
                _ if value.starts_with('\\') => (), // Unknown system flags are server-owned.
                _ => {
                    validate_keyword(value)?;
                    flags.keywords.push(value.to_ascii_lowercase());
                }
            }
        }
        flags.keywords.sort();
        flags.keywords.dedup();
        result.insert(uid, flags);
    }
    Ok(result)
}

fn metadata_attributes(line: &str) -> Result<(u32, &str)> {
    let upper = line.to_ascii_uppercase();
    let start = upper.find(" FETCH (").ok_or("Malformed FETCH response")? + 8;
    let mut rest = &line[start..];
    let mut uid = None;
    let mut flags = None;
    while !rest.trim_start().starts_with(')') {
        rest = rest.trim_start();
        let name_end = rest
            .find(char::is_whitespace)
            .ok_or("Malformed FETCH attribute")?;
        let name = &rest[..name_end];
        rest = rest[name_end..].trim_start();
        let value = if rest.starts_with('(') {
            let mut depth = 0;
            let mut end = None;
            for (position, character) in rest.char_indices() {
                match character {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(position + 1);
                            break;
                        }
                    }
                    _ => (),
                }
            }
            let end = end.ok_or("Malformed FETCH list")?;
            let value = &rest[..end];
            rest = &rest[end..];
            value
        } else {
            let end = rest
                .find(|character: char| character.is_whitespace() || character == ')')
                .unwrap_or(rest.len());
            if end == 0 {
                return Err("Missing FETCH attribute value".into());
            }
            let value = &rest[..end];
            rest = &rest[end..];
            value
        };
        if name.eq_ignore_ascii_case("UID") {
            if uid.is_some() {
                return Err("Duplicate FETCH UID".into());
            }
            uid = Some(
                value
                    .parse::<u32>()
                    .ok()
                    .filter(|uid| *uid != 0)
                    .ok_or("Malformed FETCH UID")?,
            );
        } else if name.eq_ignore_ascii_case("FLAGS") {
            if flags.is_some() {
                return Err("Duplicate FETCH FLAGS".into());
            }
            flags = Some(
                value
                    .strip_prefix('(')
                    .and_then(|value| value.strip_suffix(')'))
                    .ok_or("Malformed FETCH flags")?,
            );
        }
        if rest.is_empty() {
            return Err("Incomplete FETCH attributes".into());
        }
    }
    Ok((
        uid.ok_or("IMAP FLAGS FETCH omitted UID")?,
        flags.ok_or("IMAP FLAGS FETCH omitted FLAGS")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modified_utf7_and_roles_are_decoded_without_changing_identity() {
        assert_eq!(
            decode_mailbox("Projects/&U,BTFw-/A&-B").unwrap(),
            "Projects/台北/A&B"
        );
        let reply = ImapReply {
            lines: vec![
                "* LIST (\\Noselect) \"/\" \"Projects\"".into(),
                "* LIST (\\Trash) \"/\" \"Projects/&U,BTFw-\"".into(),
            ],
            literals: vec![],
            completion: String::new(),
        };
        let folders = folders(&reply, false).unwrap();
        assert!(!folders[0].selectable);
        assert_eq!(folders[1].display_name, "Projects/台北");
        assert_eq!(folders[1].role, Some(ImapFolderRole::Trash));
    }
    #[test]
    fn flags_keywords_cannot_override_the_top_level_uid() {
        let reply = ImapReply {
            lines: vec!["* 1 FETCH (FLAGS (UID 999 \\Seen) UID 42)".into()],
            literals: vec![],
            completion: String::new(),
        };
        let flags = flags_from_reply(&reply).unwrap();
        assert!(flags[&42].seen);
        assert!(flags[&42].keywords.contains(&"uid".into()));
        assert!(!flags.contains_key(&999));
    }
    #[test]
    fn rev2_esearch_uses_complete_uid_sets_and_rejects_sequence_numbers() {
        let mut reply = ImapReply {
            lines: vec!["* ESEARCH (TAG \"R0004\") UID ALL 3:1,8 COUNT 4".into()],
            literals: vec![],
            completion: String::new(),
        };
        assert_eq!(search_uids(&reply).unwrap(), vec![1, 2, 3, 8]);
        reply.lines = vec!["* ESEARCH UID COUNT 0".into()];
        assert!(search_uids(&reply).unwrap().is_empty());
        reply.lines = vec!["* ESEARCH ALL 1:3".into()];
        assert!(search_uids(&reply).is_err());
        reply.lines = vec!["* ESEARCH UID ALL 1:4000000000".into()];
        assert!(search_uids(&reply).is_err());
    }
    #[test]
    fn empty_search_is_distinct_from_missing_snapshot() {
        let mut reply = ImapReply {
            lines: vec!["* SEARCH".into()],
            literals: vec![],
            completion: String::new(),
        };
        assert!(search_uids(&reply).unwrap().is_empty());
        reply.lines.clear();
        assert!(search_uids(&reply).is_err());
    }
}
