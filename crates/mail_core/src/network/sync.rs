//! Folder-aware IMAP synchronization. A complete UID/flag snapshot is separate
//! from bounded body downloads, so interrupted or large mailboxes remain safe.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

mod parsing;
use parsing::{flags_from_reply, folders, response_number, search_uids};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImapFolderRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    All,
    Important,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapFolder {
    /// Exact server name, kept encoded for IMAP4rev1 commands.
    pub mailbox: String,
    pub display_name: String,
    pub delimiter: Option<char>,
    pub selectable: bool,
    pub role: Option<ImapFolderRole>,
}
impl ImapFolder {
    pub fn local_folder_key(&self) -> String {
        match self.role {
            Some(ImapFolderRole::Inbox) => "Inbox".into(),
            Some(ImapFolderRole::Sent) => "Sent".into(),
            Some(ImapFolderRole::Drafts) => "Drafts".into(),
            Some(ImapFolderRole::Trash) => "Trash".into(),
            Some(ImapFolderRole::Junk) => "Junk".into(),
            Some(ImapFolderRole::Archive) => "Archive".into(),
            _ => format!("imap:{}", self.mailbox),
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapFlags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub deleted: bool,
    pub draft: bool,
    #[serde(default)]
    pub keywords: Vec<String>,
}
impl ImapFlags {
    pub(crate) fn wire_flags(&self) -> Result<BTreeSet<String>> {
        let mut flags = BTreeSet::new();
        for (enabled, flag) in [
            (self.seen, "\\Seen"),
            (self.flagged, "\\Flagged"),
            (self.answered, "\\Answered"),
            (self.deleted, "\\Deleted"),
            (self.draft, "\\Draft"),
        ] {
            if enabled {
                flags.insert(flag.into());
            }
        }
        for keyword in &self.keywords {
            validate_keyword(keyword)?;
            flags.insert(keyword.to_ascii_lowercase());
        }
        Ok(flags)
    }
    pub(crate) fn apply_delta(&mut self, change: &PendingFlagChange) {
        for (values, enabled) in [(&change.add, true), (&change.remove, false)] {
            for flag in values {
                match flag.to_ascii_lowercase().as_str() {
                    "\\seen" => self.seen = enabled,
                    "\\flagged" => self.flagged = enabled,
                    "\\answered" => self.answered = enabled,
                    "\\deleted" => self.deleted = enabled,
                    "\\draft" => self.draft = enabled,
                    _ if enabled => {
                        if !self
                            .keywords
                            .iter()
                            .any(|keyword| keyword.eq_ignore_ascii_case(flag))
                        {
                            self.keywords.push(flag.clone());
                        }
                    }
                    _ => self
                        .keywords
                        .retain(|keyword| !keyword.eq_ignore_ascii_case(flag)),
                }
            }
        }
        self.keywords.sort();
    }
}
fn validate_keyword(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 255
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte <= 32 || byte >= 127 || b"(){}%*\"\\]".contains(&byte))
    {
        return Err("Invalid IMAP keyword".into());
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingFlagChange {
    pub location: ImapLocation,
    pub add: Vec<String>,
    pub remove: Vec<String>,
}
#[derive(Debug, Clone, Default)]
pub struct ImapSyncRequest {
    pub known: Vec<ImapLocation>,
    /// Includes tombstones for deliberately purged messages.
    pub known_keys: Vec<String>,
    pub pending_flags: Vec<PendingFlagChange>,
    /// Copied/deleted source UIDs retained until the server expunges them.
    pub deferred_sources: Vec<ImapLocation>,
}
#[derive(Debug)]
pub struct ImapDownload {
    pub location: ImapLocation,
    pub raw: Vec<u8>,
    pub flags: ImapFlags,
}
#[derive(Debug, Clone)]
pub struct ImapMailboxSnapshot {
    pub mailbox: String,
    pub uid_validity: u32,
    pub uid_next: Option<u32>,
    /// Only published after every metadata FETCH succeeds.
    pub messages: BTreeMap<u32, ImapFlags>,
}
#[derive(Debug, Default)]
pub struct ImapSyncReport {
    pub folders: Vec<ImapFolder>,
    pub mailboxes: Vec<ImapMailboxSnapshot>,
    pub downloads: Vec<ImapDownload>,
    pub completed_flags: Vec<PendingFlagChange>,
    pub obsolete_flags: Vec<PendingFlagChange>,
    pub warnings: Vec<String>,
    pub has_more: bool,
}

pub fn sync_imap(account: &Account, request: &ImapSyncRequest) -> Result<ImapSyncReport> {
    sync_imap_with_progress(account, request, &mut |_| {})
}

pub fn sync_imap_with_progress(
    account: &Account,
    request: &ImapSyncRequest,
    progress: &mut dyn FnMut(String),
) -> Result<ImapSyncReport> {
    sync_imap_controlled(account, request, progress, &SyncControl::default())
}
pub fn sync_imap_controlled(
    account: &Account,
    request: &ImapSyncRequest,
    progress: &mut dyn FnMut(String),
    control: &SyncControl,
) -> Result<ImapSyncReport> {
    control.run(|| sync_imap_inner(account, request, progress, control))
}
fn sync_imap_inner(
    account: &Account,
    request: &ImapSyncRequest,
    progress: &mut dyn FnMut(String),
    control: &SyncControl,
) -> Result<ImapSyncReport> {
    control.check()?;
    progress("Connecting and signing in…".into());
    let mut session = imap_connect(account)?;
    progress("Discovering server folders…".into());
    let capabilities = session
        .command("CAPABILITY")?
        .lines
        .join(" ")
        .to_ascii_uppercase();
    let list = if capabilities
        .split_whitespace()
        .any(|value| value == "SPECIAL-USE")
    {
        control.check()?;
        match session.command("LIST \"\" \"*\" RETURN (SPECIAL-USE)") {
            Ok(reply) => reply,
            Err(ImapError::Rejected) => session.command("LIST \"\" \"*\"")?,
            Err(error) => return Err(error.into()),
        }
    } else {
        session.command("LIST \"\" \"*\"")?
    };
    let mut report = ImapSyncReport {
        folders: folders(
            &list,
            capabilities
                .split_whitespace()
                .any(|value| value == "IMAP4REV2"),
        )?,
        ..Default::default()
    };
    if !report
        .folders
        .iter()
        .any(|folder| folder.mailbox.eq_ignore_ascii_case("INBOX"))
    {
        return Err("IMAP LIST omitted INBOX; folder reconciliation refused".into());
    }
    let connection = account.id();
    let known: BTreeSet<_> = request
        .known
        .iter()
        .map(ImapLocation::remote_key)
        .chain(request.known_keys.iter().cloned())
        .collect();
    let mut bytes = 0;
    let total = report
        .folders
        .iter()
        .filter(|folder| folder.selectable)
        .count();
    let mut completed = 0;
    for folder in report
        .folders
        .clone()
        .into_iter()
        .filter(|folder| folder.selectable)
    {
        progress(format!(
            "Folder {}/{}: {} · checking messages…",
            completed + 1,
            total,
            folder.display_name
        ));
        let result = sync_mailbox(
            &mut session,
            &connection,
            &folder,
            request,
            &known,
            &mut bytes,
            &mut report,
            progress,
            control,
        );
        completed += 1;
        progress(format!(
            "Checked {completed}/{total} folders · downloaded {} messages",
            report.downloads.len()
        ));
        if let Err(error) = result {
            let transport_failed = !matches!(&error, ImapError::Rejected);
            report
                .warnings
                .push(format!("{}: {}", folder.display_name, String::from(error)));
            if transport_failed {
                report.has_more = true;
                break;
            }
        }
    }
    let _ = session.session.send("R9999 LOGOUT");
    control.check()?;
    Ok(report)
}

fn sync_mailbox(
    session: &mut Imap,
    connection: &str,
    folder: &ImapFolder,
    request: &ImapSyncRequest,
    known: &BTreeSet<String>,
    bytes: &mut usize,
    report: &mut ImapSyncReport,
    progress: &mut dyn FnMut(String),
    control: &SyncControl,
) -> std::result::Result<(), ImapError> {
    let selected = session.command(&format!("SELECT {}", quoted(&folder.mailbox)?))?;
    let validity = uid_validity(&selected)?;
    let search = session.command("UID SEARCH ALL")?;
    let uids = search_uids(&search)?;
    for change in request.pending_flags.iter().filter(|change| {
        change.location.connection == connection
            && same_mailbox(&change.location.mailbox, &folder.mailbox)
    }) {
        if change.location.uid_validity != validity {
            report.obsolete_flags.push(change.clone());
            report.warnings.push(format!(
                "{}: pending flags discarded after UIDVALIDITY changed",
                folder.display_name
            ));
            continue;
        }
        if !uids.contains(&change.location.uid) {
            continue;
        }
        let mut complete = true;
        for (values, operation) in [
            (&change.add, "+FLAGS.SILENT"),
            (&change.remove, "-FLAGS.SILENT"),
        ] {
            if values.is_empty() {
                continue;
            }
            validate_changes(values)?;
            match session.command(&format!(
                "UID STORE {} {} ({})",
                change.location.uid,
                operation,
                values.join(" ")
            )) {
                Ok(_) => (),
                Err(ImapError::Rejected) => {
                    complete = false;
                    report.warnings.push(format!(
                        "{}: server rejected pending flags for UID {}",
                        folder.display_name, change.location.uid
                    ));
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        if complete {
            report.completed_flags.push(change.clone());
        }
    }
    let mut messages = BTreeMap::new();
    for batch in uids.chunks(256) {
        control.check()?;
        let set = batch
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let fetched = session.command(&format!("UID FETCH {set} (UID FLAGS)"))?;
        for (uid, flags) in flags_from_reply(&fetched)? {
            if batch.contains(&uid) {
                messages.insert(uid, flags);
            }
        }
    }
    // STORE for an expunged UID may return OK without applying anything. Only
    // acknowledge changes proven by the subsequent complete flag snapshot.
    report.completed_flags.retain(|change| {
        if !same_mailbox(&change.location.mailbox, &folder.mailbox) {
            return true;
        }
        messages.get(&change.location.uid).is_some_and(|flags| {
            let mut desired = flags.clone();
            desired.apply_delta(change);
            desired == *flags
        })
    });
    report.mailboxes.push(ImapMailboxSnapshot {
        mailbox: folder.mailbox.clone(),
        uid_validity: validity,
        uid_next: response_number(&selected, "UIDNEXT"),
        messages: messages.clone(),
    });
    progress(format!(
        "{}: {} messages · downloading new mail…",
        folder.display_name,
        messages.len()
    ));
    for (&uid, flags) in messages.iter().rev() {
        control.check()?;
        let location = ImapLocation {
            connection: connection.into(),
            mailbox: folder.mailbox.clone(),
            uid_validity: validity,
            uid,
        };
        let restored_source = !flags.deleted
            && request.deferred_sources.contains(&location)
            && !request.known.contains(&location);
        if known.contains(&location.remote_key()) && !restored_source {
            continue;
        }
        if report.downloads.len() >= 200 || *bytes >= MAX_BATCH {
            report.has_more = true;
            continue;
        }
        let key = location.remote_key();
        let raw = if let Some(raw) = control.load(&key)? {
            raw
        } else {
            let fetched = match session.command(&format!("UID FETCH {uid} (UID BODY.PEEK[])")) {
                Ok(reply) => reply,
                Err(ImapError::Rejected) => {
                    report.warnings.push(format!(
                        "{}: body download rejected for UID {uid}",
                        folder.display_name
                    ));
                    continue;
                }
                Err(error) => return Err(error),
            };
            let Some(raw) = super::changes::fetch_uid_body(&fetched, uid)? else {
                // It may have been expunged after SEARCH. The next complete snapshot reconciles it.
                report.warnings.push(format!(
                    "{}: body unavailable for UID {uid}",
                    folder.display_name
                ));
                continue;
            };
            control.save(&key, &raw)?;
            raw
        };
        if *bytes + raw.len() > MAX_BATCH {
            report.has_more = true;
            continue;
        }
        *bytes += raw.len();
        report.downloads.push(ImapDownload {
            location,
            raw,
            flags: flags.clone(),
        });
        progress(format!(
            "{}: downloaded {} new messages · {} KB",
            folder.display_name,
            report.downloads.len(),
            *bytes / 1024
        ));
    }
    Ok(())
}
pub(crate) fn same_mailbox(left: &str, right: &str) -> bool {
    left == right || (left.eq_ignore_ascii_case("INBOX") && right.eq_ignore_ascii_case("INBOX"))
}
fn validate_changes(flags: &[String]) -> Result<()> {
    for flag in flags {
        if !matches!(
            flag.as_str(),
            "\\Seen" | "\\Flagged" | "\\Answered" | "\\Draft"
        ) {
            // \Deleted is reserved for the explicit UID-scoped delete workflow.
            validate_keyword(flag)?;
        }
    }
    Ok(())
}
