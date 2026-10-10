use mail_core::{Result, message::Draft};
use std::path::{Path, PathBuf};

/// User intentions, with native menu and control IDs decoded by the UI adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    NewMessage,
    OpenMessage,
    SaveAttachments,
    ShowPictures,
    MarkRead,
    MarkUnread,
    ToggleFlag,
    Reply,
    ReplyAll,
    Forward,
    Import,
    Export,
    Attach,
    ConfigureAccount,
    Receive,
    CancelSync,
    SaveDraft,
    Send,
    Delete,
    Restore,
    DeletePermanently,
    EmptyTrash,
    Find,
    About,
    Exit,
    ToggleFolders,
    TogglePreview,
    Refresh,
    CloseComposer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    Sender,
    All,
    Forward,
}

/// A decoded launch argument. Native command-line tokenization stays in the
/// platform adapter, so Windows quoting rules do not leak into application code.
#[derive(Debug)]
pub enum LaunchAction {
    Ignore,
    Compose(Draft),
    Import(PathBuf),
    Unsupported(String),
}

impl LaunchAction {
    pub fn parse(argument: &str) -> Result<Self> {
        let lower = argument.to_ascii_lowercase();
        if matches!(lower.as_str(), "/mail" | "/mailonly" | "/outmail")
            || lower.starts_with("-embedding")
        {
            return Ok(Self::Ignore);
        }
        let argument = if lower.starts_with("/mailurl:") {
            &argument[9..]
        } else if lower.starts_with("/eml:") {
            &argument[5..]
        } else {
            argument
        };
        if argument.to_ascii_lowercase().starts_with("mailto:") {
            return Draft::mailto(argument).map(Self::Compose);
        }
        let path = Path::new(argument.trim_matches('"'));
        if path.is_file() || path.is_dir() {
            Ok(Self::Import(path.to_owned()))
        } else if argument.is_empty() {
            Ok(Self::Ignore)
        } else {
            Ok(Self::Unsupported(argument.to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_flags_and_mailto_are_decoded_without_window_state() {
        for flag in ["", "/MAIL", "/mailonly", "/outmail", "-Embedding"] {
            assert!(matches!(
                LaunchAction::parse(flag).unwrap(),
                LaunchAction::Ignore
            ));
        }
        let LaunchAction::Compose(draft) = LaunchAction::parse(
            "/MAILURL:mailto:to@example.com?subject=%E4%BD%A0%E5%A5%BD%20world&bcc=hidden%40example.com",
        )
        .unwrap()
        else {
            panic!("Expected a mailto draft");
        };
        assert_eq!(draft.to, "to@example.com");
        assert_eq!(draft.subject, "你好 world");
        assert_eq!(draft.bcc, "hidden@example.com");
        assert!(matches!(
            LaunchAction::parse("/not-a-supported-argument").unwrap(),
            LaunchAction::Unsupported(value) if value == "/not-a-supported-argument"
        ));
    }

    #[test]
    fn launch_import_accepts_existing_unicode_paths() {
        let root = std::env::temp_dir().join(format!(
            "winmail-launch-{}-{}-中文 folder",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("邮件.eml");
        std::fs::write(&file, b"Subject: launch\r\n\r\nbody").unwrap();
        for path in [&root, &file] {
            let argument = format!("/eml:\"{}\"", path.display());
            assert!(matches!(
                LaunchAction::parse(&argument).unwrap(),
                LaunchAction::Import(found) if found == *path
            ));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
