//! Associate FETCH attributes and body literals with a verified UID.
use super::{ImapReply, Result};

/// Associate body literals with their complete FETCH response, including a UID
/// that legally follows the literal. Unsolicited FETCH responses never replace it.
pub(in crate::network) fn fetch_uid_body(
    reply: &ImapReply,
    expected: u32,
) -> Result<Option<Vec<u8>>> {
    fn fetch_uid(text: &str) -> Result<Option<u32>> {
        let mut depth = 0u32;
        let mut uid = None;
        let mut expect_uid = false;
        let mut characters = text.chars().peekable();
        while let Some(character) = characters.next() {
            match character {
                '(' => depth += 1,
                ')' => depth = depth.checked_sub(1).ok_or("Malformed FETCH parentheses")?,
                '"' => {
                    let mut escaped = false;
                    for next in characters.by_ref() {
                        if escaped {
                            escaped = false;
                        } else if next == '\\' {
                            escaped = true;
                        } else if next == '"' {
                            break;
                        }
                    }
                }
                value if value.is_whitespace() => {}
                first => {
                    let mut atom = String::from(first);
                    while characters.peek().is_some_and(|value| {
                        !value.is_whitespace() && !['(', ')', '"'].contains(value)
                    }) {
                        atom.push(characters.next().unwrap());
                    }
                    if depth == 1 {
                        if expect_uid {
                            let value = atom
                                .parse::<u32>()
                                .ok()
                                .filter(|value| *value != 0)
                                .ok_or("Malformed FETCH UID")?;
                            if uid.replace(value).is_some() {
                                return Err("Duplicate FETCH UID attribute".into());
                            }
                            expect_uid = false;
                        } else if atom.eq_ignore_ascii_case("UID") {
                            expect_uid = true;
                        }
                    }
                }
            }
        }
        if expect_uid {
            return Err("Missing FETCH UID value".into());
        }
        Ok(uid)
    }
    fn take_body(
        text: &str,
        body: &mut Option<Vec<u8>>,
        expected: u32,
        result: &mut Option<Vec<u8>>,
    ) -> Result<()> {
        let Some(body) = body.take() else {
            return Ok(());
        };
        let uid = fetch_uid(text)?.ok_or("FETCH body omitted its UID identity")?;
        if uid == expected && result.replace(body).is_some() {
            return Err("Server returned duplicate body literals for one UID".into());
        }
        Ok(())
    }
    let mut result = None;
    let mut metadata = String::new();
    let mut body = None;
    let mut literals = reply.literals.iter();
    for line in &reply.lines {
        if line.starts_with("* ") {
            take_body(&metadata, &mut body, expected, &mut result)?;
            metadata.clear();
            let atoms: Vec<_> = line.split_whitespace().take(3).collect();
            if !atoms
                .get(2)
                .is_some_and(|atom| atom.eq_ignore_ascii_case("FETCH"))
            {
                if line.ends_with('}') {
                    let _ = literals.next().ok_or("Missing response literal")?;
                }
                continue;
            }
        }
        if metadata.is_empty() && !line.starts_with("* ") {
            continue;
        }
        metadata.push(' ');
        metadata.push_str(line);
        if line.ends_with('}') {
            let literal = literals.next().ok_or("Missing FETCH literal")?;
            let literal_at = line.rfind('{').ok_or("Malformed FETCH literal marker")?;
            if line[..literal_at]
                .split_whitespace()
                .last()
                .is_some_and(|atom| atom.trim_start_matches('(').eq_ignore_ascii_case("BODY[]"))
                && body.replace(literal.clone()).is_some()
            {
                return Err("Duplicate complete body in FETCH".into());
            }
        }
    }
    take_body(&metadata, &mut body, expected, &mut result)?;
    if literals.next().is_some() {
        return Err("Unassociated FETCH literal".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(lines: &[&str], literals: &[&[u8]]) -> ImapReply {
        ImapReply {
            lines: lines.iter().map(|line| (*line).into()).collect(),
            literals: literals.iter().map(|bytes| bytes.to_vec()).collect(),
            completion: "R0001 OK fetched".into(),
        }
    }
    #[test]
    fn fetch_identity_after_literal_ignores_other_uids_and_flag_tokens() {
        let data = reply(
            &[
                "* 7 FETCH (UID 99 FLAGS (\\Seen))",
                "* 2 FETCH (FLAGS (UID 100) BODY[] {4}",
                " UID 42)",
                "* 9 FETCH (BODY[] {5}",
                " UID 70)",
            ],
            &[b"ours", b"other"],
        );
        assert_eq!(fetch_uid_body(&data, 42).unwrap(), Some(b"ours".to_vec()));
        assert_eq!(fetch_uid_body(&data, 70).unwrap(), Some(b"other".to_vec()));
    }
    #[test]
    fn fetch_without_identity_cannot_be_used_as_proof() {
        let data = reply(&["* 2 FETCH (BODY[] {4}", ")"], &[b"ours"]);
        assert!(
            fetch_uid_body(&data, 42)
                .unwrap_err()
                .contains("UID identity")
        );
    }
    #[test]
    fn duplicate_body_for_the_requested_uid_is_rejected() {
        let data = reply(
            &[
                "* 2 FETCH (UID 42 BODY[] {4}",
                ")",
                "* 2 FETCH (UID 42 BODY[] {4}",
                ")",
            ],
            &[b"ours", b"copy"],
        );
        assert!(fetch_uid_body(&data, 42).unwrap_err().contains("duplicate"));
    }
}
