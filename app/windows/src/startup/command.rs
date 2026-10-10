#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Launch,
    Register,
    Unregister,
    Identify,
    Install,
}

fn prefix(text: &[u16], prefix: &str) -> bool {
    let expected: Vec<u16> = prefix.encode_utf16().collect();
    text.len() >= expected.len()
        && text.iter().zip(expected).all(|(&a, b)| {
            let lower = |c: u16| if (65..=90).contains(&c) { c + 32 } else { c };
            lower(a) == lower(b)
        })
}

/// RVA 0x4408: match the complete raw WinMain tail, not tokenized argv.
pub fn route(tail: &[u16]) -> Route {
    if prefix(tail, "OCInstall") {
        Route::Install
    } else if tail.eq_ignore_ascii_case(&"/reg".encode_utf16().collect::<Vec<_>>()) {
        Route::Register
    } else if tail.eq_ignore_ascii_case(&"/unreg".encode_utf16().collect::<Vec<_>>()) {
        Route::Unregister
    } else if ["/identcatalog", "/identfileslist:", "/identfile:"]
        .iter()
        .any(|s| prefix(tail, s))
    {
        Route::Identify
    } else {
        Route::Launch
    }
}

trait WideAsciiEq {
    fn eq_ignore_ascii_case(&self, other: &[u16]) -> bool;
}
impl WideAsciiEq for [u16] {
    fn eq_ignore_ascii_case(&self, other: &[u16]) -> bool {
        self.len() == other.len()
            && self.iter().zip(other).all(|(&a, &b)| {
                let lower = |c: u16| if (65..=90).contains(&c) { c + 32 } else { c };
                lower(a) == lower(b)
            })
    }
}

/// CRT RVA 0x6205..0x629a: skip argv[0], toggling quotes and trimming <= U+0020.
/// Keep the remaining UTF-16 unchanged, including quoting and lone surrogates.
pub fn command_tail(full: &[u16]) -> &[u16] {
    let mut quoted = false;
    let mut i = 0;
    while i < full.len() && full[i] != 0 {
        if full[i] <= 32 && !quoted {
            break;
        }
        if full[i] == 34 {
            quoted = !quoted;
        }
        i += 1;
    }
    while i < full.len() && full[i] != 0 && full[i] <= 32 {
        i += 1;
    }
    let end = full[i..]
        .iter()
        .position(|&c| c == 0)
        .map_or(full.len(), |n| i + n);
    &full[i..end]
}

pub fn policy_blocks(tail: &[u16], allowed: bool) -> bool {
    !allowed && !prefix(tail, "-Embedding")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn exact_registration_and_prefix_identification_match_disassembly() {
        assert_eq!(route(&wide("/REG")), Route::Register);
        assert_eq!(route(&wide("/reg extra")), Route::Launch);
        assert_eq!(route(&wide(" /reg")), Route::Launch);
        assert_eq!(route(&wide("/unREG")), Route::Unregister);
        assert_eq!(route(&wide("/IDENTFILE:C:\\信件.eml")), Route::Identify);
        assert_eq!(route(&wide("/identcatalogsuffix")), Route::Identify);
        assert_eq!(route(&wide("OCInstallUserConfigOE")), Route::Install);
        assert_eq!(route(&wide("mailto:a@example.test")), Route::Launch);
    }

    #[test]
    fn raw_tail_preserves_unicode_and_quotes() {
        let full =
            wide("\"C:\\Program Files\\Windows Mail\\WinMail.exe\" \t\"C:\\信件.eml\" /news\0");
        assert_eq!(command_tail(&full), wide("\"C:\\信件.eml\" /news"));
        let full = [b'x' as u16, 32, 0xd800, 0];
        assert_eq!(command_tail(&full), [0xd800]);
        assert!(command_tail(&wide("WinMail.exe")).is_empty());
    }

    #[test]
    fn embedding_bypasses_manual_launch_policy_by_prefix() {
        assert!(!policy_blocks(&wide("-Embedding"), false));
        assert!(!policy_blocks(&wide("-embeddingsuffix"), false));
        assert!(policy_blocks(&wide("/reg"), false));
        assert!(!policy_blocks(&[], true));
    }
}
