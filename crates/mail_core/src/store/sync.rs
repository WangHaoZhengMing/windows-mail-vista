//! Persistent folder snapshots and idempotent flag outbox.
use super::*;
use crate::network::{ImapFolder, ImapSyncReport, ImapSyncRequest, PendingFlagChange};
use std::collections::BTreeSet;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct StoredFolder {
    pub account_id: AccountId,
    pub folder: ImapFolder,
    pub uid_validity: Option<u32>,
    pub uid_next: Option<u32>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct StoredFlagChange {
    pub account_id: AccountId,
    pub change: PendingFlagChange,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingCompaction {
    pub account_id: AccountId,
    pub source: ImapLocation,
    pub local_id: String,
    /// A move preserves the destination; only an explicit permanent deletion
    /// retires the cached local copy after the source UID actually disappears.
    pub purge_when_absent: bool,
}
#[derive(Debug, Default)]
pub struct SyncSummary {
    pub downloaded: usize,
    pub updated: usize,
    pub recovered: usize,
    pub purged: usize,
    pub pending: usize,
    pub has_more: bool,
    pub warnings: Vec<String>,
}
impl Store {
    pub fn server_folders(&self, account: &AccountId) -> Vec<&ImapFolder> {
        self.catalog
            .server_folders
            .iter()
            .filter(|folder| &folder.account_id == account)
            .map(|folder| &folder.folder)
            .collect()
    }
    pub fn pending_compactions(&self) -> &[PendingCompaction] {
        &self.catalog.pending_compactions
    }
    pub fn move_remote(&mut self, id: &str, destination: ImapLocation) -> Result<()> {
        self.apply_remote_move(id, destination, None)
    }
    pub fn move_remote_deferred(
        &mut self,
        id: &str,
        destination: ImapLocation,
        original: ImapLocation,
    ) -> Result<()> {
        self.apply_remote_move(id, destination, Some(original))
    }
    fn apply_remote_move(
        &mut self,
        id: &str,
        destination: ImapLocation,
        deferred: Option<ImapLocation>,
    ) -> Result<()> {
        self.commit(|catalog| {
            let entry = catalog
                .messages
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or("Message not found")?;
            let account = entry
                .account_id
                .clone()
                .ok_or("Remote message has no account")?;
            let original = entry.remote.clone();
            entry.folder = catalog
                .server_folders
                .iter()
                .find(|folder| {
                    folder.account_id == account
                        && mailbox_eq(&folder.folder.mailbox, &destination.mailbox)
                })
                .map(|folder| folder.folder.local_folder_key())
                .unwrap_or_else(|| {
                    if destination.mailbox.eq_ignore_ascii_case("INBOX") {
                        "Inbox".into()
                    } else {
                        format!("imap:{}", destination.mailbox)
                    }
                });
            entry.remote_key = Some(destination.remote_key());
            entry.remote = Some(destination.clone());
            entry.pending_delete = false;
            entry.flags.deleted = false;
            entry.remote_problem = None;
            entry.trash_origin = None;
            entry.restore_mailbox = None;
            for pending in &mut catalog.pending_flags {
                if pending.account_id == account
                    && Some(&pending.change.location) == original.as_ref()
                {
                    pending.change.location = destination.clone();
                }
            }
            catalog
                .pending_compactions
                .retain(|pending| pending.local_id != id || !pending.purge_when_absent);
            if let Some(source) = deferred {
                if !catalog
                    .pending_compactions
                    .iter()
                    .any(|pending| pending.source == source)
                {
                    catalog.pending_compactions.push(PendingCompaction {
                        account_id: account.clone(),
                        source: source.clone(),
                        local_id: id.into(),
                        purge_when_absent: false,
                    });
                }
                remember(catalog, Some(&account), Some(&source.remote_key()));
            }
            if let Some(source) = original {
                remember(catalog, Some(&account), Some(&source.remote_key()));
            }
            remember(catalog, Some(&account), Some(&destination.remote_key()));
            Ok(())
        })
    }
    pub fn is_local_draft(&self, id: &str) -> bool {
        self.catalog.drafts.contains_key(id)
    }
    pub fn imap_sync_request(&self, account: &AccountId) -> Result<ImapSyncRequest> {
        self.account_record(account).ok_or("Account not found")?;
        Ok(ImapSyncRequest {
            known: self
                .entries()
                .iter()
                .filter(|entry| entry.account_id.as_ref() == Some(account))
                .filter_map(|entry| entry.remote.clone())
                .collect(),
            known_keys: self
                .known_remote_for(account)
                .into_iter()
                .chain(
                    self.catalog
                        .pending_compactions
                        .iter()
                        .filter(|pending| &pending.account_id == account)
                        .map(|pending| pending.source.remote_key()),
                )
                .collect(),
            deferred_sources: self
                .catalog
                .pending_compactions
                .iter()
                .filter(|pending| &pending.account_id == account)
                .map(|pending| pending.source.clone())
                .collect(),
            pending_flags: self
                .catalog
                .pending_flags
                .iter()
                .filter(|change| &change.account_id == account)
                .map(|change| change.change.clone())
                .collect(),
        })
    }
    pub fn set_message_flags(&mut self, id: &str, mut flags: ImapFlags) -> Result<()> {
        flags
            .keywords
            .iter_mut()
            .for_each(|keyword| *keyword = keyword.to_ascii_lowercase());
        flags.keywords.sort();
        flags.keywords.dedup();
        let desired = flags.wire_flags()?;
        let entry = self.entry(id).ok_or("Message not found")?;
        if flags.deleted != entry.flags.deleted {
            return Err(
                "Use the explicit delete or restore operation to change the Deleted flag".into(),
            );
        }
        let existing = entry.flags.wire_flags()?;
        let delta = PendingFlagChange {
            location: entry.remote.clone().unwrap_or(ImapLocation {
                connection: String::new(),
                mailbox: String::new(),
                uid_validity: 0,
                uid: 0,
            }),
            add: desired.difference(&existing).cloned().collect(),
            remove: existing.difference(&desired).cloned().collect(),
        };
        if delta.add.is_empty() && delta.remove.is_empty() && entry.read == flags.seen {
            return Ok(());
        }
        let account = entry.account_id.clone();
        let remote = entry.remote.clone();
        self.commit(|catalog| {
            let entry = catalog
                .messages
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or("Message not found")?;
            entry.read = flags.seen;
            entry.flags = flags;
            if let (Some(account), Some(_)) = (account, remote) {
                if let Some(existing) = catalog.pending_flags.iter_mut().find(|pending| {
                    pending.account_id == account && pending.change.location == delta.location
                }) {
                    for value in &delta.add {
                        existing
                            .change
                            .remove
                            .retain(|flag| !flag.eq_ignore_ascii_case(value));
                        if !existing
                            .change
                            .add
                            .iter()
                            .any(|flag| flag.eq_ignore_ascii_case(value))
                        {
                            existing.change.add.push(value.clone());
                        }
                    }
                    for value in &delta.remove {
                        existing
                            .change
                            .add
                            .retain(|flag| !flag.eq_ignore_ascii_case(value));
                        if !existing
                            .change
                            .remove
                            .iter()
                            .any(|flag| flag.eq_ignore_ascii_case(value))
                        {
                            existing.change.remove.push(value.clone());
                        }
                    }
                    existing.change.add.sort();
                    existing.change.remove.sort();
                } else {
                    catalog.pending_flags.push(StoredFlagChange {
                        account_id: account,
                        change: delta,
                    });
                }
            }
            Ok(())
        })
    }
    pub fn trash_deferred(
        &mut self,
        id: &str,
        destination: ImapLocation,
        original: ImapLocation,
    ) -> Result<()> {
        self.apply_deferred_move(id, destination, original, true)
    }
    pub fn restore_deferred(
        &mut self,
        id: &str,
        destination: ImapLocation,
        original: ImapLocation,
    ) -> Result<()> {
        self.apply_deferred_move(id, destination, original, false)
    }
    fn apply_deferred_move(
        &mut self,
        id: &str,
        destination: ImapLocation,
        original: ImapLocation,
        trash: bool,
    ) -> Result<()> {
        self.commit(|catalog| {
            let entry = catalog
                .messages
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or("Message not found")?;
            let account = entry
                .account_id
                .clone()
                .ok_or("Remote message has no account")?;
            if trash {
                if entry.folder == "Trash" {
                    return Err("Message is already in Deleted Items".into());
                }
                entry.trash_origin = Some(entry.folder.clone());
                entry.restore_mailbox = Some(original.mailbox.clone());
                entry.folder = "Trash".into();
            } else {
                if entry.folder != "Trash" {
                    return Err("Only Deleted Items can be restored".into());
                }
                entry.folder = entry.trash_origin.take().unwrap_or_else(|| "Inbox".into());
                entry.restore_mailbox = None;
            }
            entry.flags.deleted = false;
            entry.pending_delete = false;
            entry.remote_key = Some(destination.remote_key());
            entry.remote = Some(destination.clone());
            for pending in &mut catalog.pending_flags {
                if pending.account_id == account && pending.change.location == original {
                    pending.change.location = destination.clone();
                }
            }
            catalog
                .pending_compactions
                .retain(|pending| pending.local_id != id || !pending.purge_when_absent);
            if !catalog
                .pending_compactions
                .iter()
                .any(|pending| pending.source == original)
            {
                catalog.pending_compactions.push(PendingCompaction {
                    account_id: account.clone(),
                    source: original.clone(),
                    local_id: id.into(),
                    purge_when_absent: false,
                });
            }
            remember(catalog, Some(&account), Some(&original.remote_key()));
            remember(catalog, Some(&account), Some(&destination.remote_key()));
            Ok(())
        })
    }
    pub fn defer_permanent_delete(&mut self, id: &str, source: ImapLocation) -> Result<()> {
        self.commit(|catalog| {
            let entry = catalog
                .messages
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or("Message not found")?;
            if entry.folder != "Trash" {
                return Err("Only Deleted Items can be permanently deleted".into());
            }
            let account = entry
                .account_id
                .clone()
                .ok_or("Remote message has no account")?;
            entry.flags.deleted = true;
            entry.pending_delete = true;
            catalog
                .pending_flags
                .retain(|change| change.change.location != source);
            if let Some(pending) = catalog
                .pending_compactions
                .iter_mut()
                .find(|pending| pending.source == source)
            {
                pending.purge_when_absent = true;
                pending.local_id = id.into();
            } else {
                catalog.pending_compactions.push(PendingCompaction {
                    account_id: account.clone(),
                    source: source.clone(),
                    local_id: id.into(),
                    purge_when_absent: true,
                });
            }
            remember(catalog, Some(&account), Some(&source.remote_key()));
            Ok(())
        })
    }
    pub fn apply_imap_sync(
        &mut self,
        account: &AccountId,
        report: ImapSyncReport,
    ) -> Result<SyncSummary> {
        let connection = self
            .account_record(account)
            .ok_or("Account not found")?
            .account
            .id();
        if !report
            .folders
            .iter()
            .any(|folder| folder.mailbox.eq_ignore_ascii_case("INBOX"))
        {
            return Err(
                "Cannot reconcile without a successful complete IMAP folder listing".into(),
            );
        }
        for snapshot in &report.mailboxes {
            if snapshot.uid_validity == 0
                || !report
                    .folders
                    .iter()
                    .any(|folder| mailbox_eq(&folder.mailbox, &snapshot.mailbox))
            {
                return Err("Invalid IMAP mailbox snapshot".into());
            }
        }
        let mut summary = SyncSummary {
            has_more: report.has_more,
            warnings: report.warnings.clone(),
            ..Default::default()
        };
        // Parse and persist every downloaded body before the atomic catalog transaction.
        // Any interrupted body write is recoverable and cannot remove existing mail.
        let mut prepared = Vec::new();
        let mut reused_ids = BTreeSet::new();
        for download in &report.downloads {
            if download.location.connection != connection {
                return Err("Sync report belongs to a different server connection".into());
            }
            if self.entries().iter().any(|entry| {
                entry.account_id.as_ref() == Some(account)
                    && entry.remote.as_ref() == Some(&download.location)
            }) {
                continue;
            }
            let parsed = match message::parse(&download.raw) {
                Ok(parsed) => parsed,
                Err(error) => {
                    summary.warnings.push(format!(
                        "{} UID {}: invalid MIME: {error}",
                        download.location.mailbox, download.location.uid
                    ));
                    continue;
                }
            };
            let moved = self
                .entries()
                .iter()
                .filter(|entry| {
                    entry.account_id.as_ref() == Some(account)
                        && !entry.pending_delete
                        && !reused_ids.contains(&entry.id)
                        && entry.summary.subject == parsed.summary.subject
                        && entry.summary.message_id == parsed.summary.message_id
                })
                .find(|entry| {
                    entry.remote.as_ref().is_some_and(|remote| {
                        remote.connection == connection
                            && !mailbox_eq(&remote.mailbox, &download.location.mailbox)
                            && report.mailboxes.iter().any(|snapshot| {
                                mailbox_eq(&snapshot.mailbox, &remote.mailbox)
                                    && snapshot.uid_validity == remote.uid_validity
                                    && !snapshot.messages.contains_key(&remote.uid)
                            })
                    }) && self.raw(&entry.id).is_ok_and(|raw| raw == download.raw)
                })
                .map(|entry| entry.id.clone());
            if let Some(id) = moved {
                reused_ids.insert(id.clone());
                prepared.push((id, parsed.summary, download));
                continue;
            }
            use sha2::{Digest, Sha256};
            let mut hash = Sha256::new();
            hash.update(account.as_str().as_bytes());
            hash.update([0]);
            hash.update(download.location.remote_key().as_bytes());
            hash.update([0]);
            hash.update(&download.raw);
            let mut id = format!("{:x}", hash.finalize());
            // A stable local ID can survive a remote move. The original source
            // may later be restored by another client, making the old hash busy.
            let mut collision = 0u32;
            while self.entry(&id).is_some()
                || prepared.iter().any(|(existing, _, _)| existing == &id)
            {
                collision += 1;
                let mut hash = Sha256::new();
                hash.update(id.as_bytes());
                hash.update(collision.to_be_bytes());
                id = format!("{:x}", hash.finalize());
            }
            atomic_write(&self.path(&id)?, &download.raw)?;
            prepared.push((id, parsed.summary, download));
        }
        self.commit(|catalog| {
            // LIST succeeded before this report exists. Failed SELECT/FETCH folders have
            // no snapshot and keep their existing messages and flags untouched.
            let previous_folders: Vec<_> = catalog
                .server_folders
                .iter()
                .filter(|folder| &folder.account_id == account)
                .cloned()
                .collect();
            catalog
                .server_folders
                .retain(|folder| &folder.account_id != account);
            for folder in &report.folders {
                let snapshot = report
                    .mailboxes
                    .iter()
                    .find(|snapshot| mailbox_eq(&snapshot.mailbox, &folder.mailbox));
                let previous = previous_folders
                    .iter()
                    .find(|old| mailbox_eq(&old.folder.mailbox, &folder.mailbox));
                catalog.server_folders.push(StoredFolder {
                    account_id: account.clone(),
                    folder: folder.clone(),
                    uid_validity: snapshot
                        .map(|snapshot| snapshot.uid_validity)
                        .or_else(|| previous.and_then(|old| old.uid_validity)),
                    uid_next: snapshot
                        .and_then(|snapshot| snapshot.uid_next)
                        .or_else(|| previous.and_then(|old| old.uid_next)),
                });
            }
            catalog.pending_flags.retain(|pending| {
                &pending.account_id != account
                    || (!report.completed_flags.contains(&pending.change)
                        && !report.obsolete_flags.contains(&pending.change))
            });
            for (id, _, download) in prepared.iter().filter(|(id, _, _)| reused_ids.contains(id)) {
                let entry = catalog
                    .messages
                    .iter_mut()
                    .find(|entry| entry.id == *id)
                    .ok_or("Moved cached message disappeared")?;
                let original = entry.remote.clone();
                entry.remote = Some(download.location.clone());
                entry.remote_key = Some(download.location.remote_key());
                entry.remote_problem = None;
                entry.pending_delete = false;
                for pending in &mut catalog.pending_flags {
                    if Some(&pending.change.location) == original.as_ref() {
                        pending.change.location = download.location.clone();
                    }
                }
                remember(
                    catalog,
                    Some(account),
                    Some(&download.location.remote_key()),
                );
                summary.downloaded += 1;
            }
            let mut purged_ids = BTreeSet::new();
            let mut restored_keys = BTreeSet::new();
            let mut cancelled_deletions = BTreeSet::new();
            catalog.pending_compactions.retain(|pending| {
                if &pending.account_id != account || pending.source.connection != connection {
                    return true;
                }
                if !report.folders.iter().any(|folder| mailbox_eq(&folder.mailbox, &pending.source.mailbox)) {
                    if pending.purge_when_absent { purged_ids.insert(pending.local_id.clone()); }
                    return false;
                }
                let Some(snapshot) = report
                    .mailboxes
                    .iter()
                    .find(|snapshot| mailbox_eq(&snapshot.mailbox, &pending.source.mailbox))
                else {
                    return true;
                };
                if snapshot.uid_validity != pending.source.uid_validity {
                    return false;
                }
                if let Some(flags) = snapshot.messages.get(&pending.source.uid) {
                    if flags.deleted { return true; }
                    restored_keys.insert(pending.source.remote_key());
                    if pending.purge_when_absent { cancelled_deletions.insert(pending.local_id.clone()); }
                    summary.warnings.push(format!("{} UID {}: Deleted flag cleared on the server; deferred deletion cancelled", pending.source.mailbox, pending.source.uid));
                    return false;
                }
                if pending.purge_when_absent {
                    purged_ids.insert(pending.local_id.clone());
                }
                false
            });
            catalog.seen.retain(|seen| seen.account_id.as_ref() != Some(account) || !restored_keys.contains(&seen.key));
            for entry in &mut catalog.messages {
                if cancelled_deletions.contains(&entry.id) { entry.pending_delete = false; }
                if entry.account_id.as_ref() != Some(account) {
                    continue;
                }
                let Some(remote) = entry.remote.clone() else {
                    continue;
                };
                if remote.connection != connection {
                    continue;
                }
                if purged_ids.contains(&entry.id) {
                    continue;
                }
                let folder_exists = report
                    .folders
                    .iter()
                    .any(|folder| mailbox_eq(&folder.mailbox, &remote.mailbox));
                let snapshot = report
                    .mailboxes
                    .iter()
                    .find(|snapshot| mailbox_eq(&snapshot.mailbox, &remote.mailbox));
                let missing = !folder_exists
                    || snapshot.is_some_and(|snapshot| {
                        snapshot.uid_validity != remote.uid_validity
                            || !snapshot.messages.contains_key(&remote.uid)
                    });
                if missing {
                    entry.folder = "Recovered".into();
                    entry.account_id = None;
                    entry.remote = None;
                    entry.pending_delete = false;
                    entry.restore_mailbox = None;
                    entry.trash_origin = None;
                    entry.remote_problem = Some(
                        "Server message removed or mailbox identity changed; local copy preserved"
                            .into(),
                    );
                    summary.recovered += 1;
                    continue;
                }
                if let Some(flags) =
                    snapshot.and_then(|snapshot| snapshot.messages.get(&remote.uid))
                {
                    let mut flags = flags.clone();
                    for pending in catalog.pending_flags.iter().filter(|pending| {
                        &pending.account_id == account && pending.change.location == remote
                    }) {
                        flags.apply_delta(&pending.change);
                    }
                    if entry.flags != flags || entry.read != flags.seen {
                        summary.updated += 1;
                    }
                    entry.read = flags.seen;
                    entry.flags = flags;
                    if let Some(folder) = report
                        .folders
                        .iter()
                        .find(|folder| mailbox_eq(&folder.mailbox, &remote.mailbox))
                    {
                        entry.folder = folder.local_folder_key();
                    }
                }
            }
            catalog.pending_flags.retain(|pending| {
                catalog.messages.iter().any(|entry| {
                    entry.account_id.as_ref() == Some(&pending.account_id)
                        && entry.remote.as_ref() == Some(&pending.change.location)
                })
            });
            for id in purged_ids {
                catalog.messages.retain(|entry| entry.id != id);
                catalog.drafts.remove(&id);
                if !catalog.purged.contains(&id) {
                    catalog.purged.push(id);
                }
                summary.purged += 1;
            }
            for (id, parsed, download) in &prepared {
                let key = download.location.remote_key();
                remember(catalog, Some(account), Some(&key));
                if catalog.messages.iter().any(|entry| {
                    entry.account_id.as_ref() == Some(account)
                        && entry.remote.as_ref() == Some(&download.location)
                }) {
                    continue;
                }
                let folder = report
                    .folders
                    .iter()
                    .find(|folder| mailbox_eq(&folder.mailbox, &download.location.mailbox))
                    .ok_or("Downloaded mailbox absent from folder listing")?;
                catalog.purged.retain(|purged| purged != id);
                catalog.messages.push(Entry {
                    id: id.clone(),
                    folder: folder.local_folder_key(),
                    summary: parsed.clone(),
                    read: download.flags.seen,
                    flags: download.flags.clone(),
                    pending_delete: false,
                    remote_key: Some(key),
                    account_id: Some(account.clone()),
                    remote: Some(download.location.clone()),
                    remote_problem: None,
                    trash_origin: None,
                    restore_mailbox: None,
                });
                summary.downloaded += 1;
            }
            summary.pending = catalog
                .pending_flags
                .iter()
                .filter(|pending| &pending.account_id == account)
                .count()
                + catalog
                    .pending_compactions
                    .iter()
                    .filter(|pending| &pending.account_id == account)
                    .count();
            Ok(())
        })?;
        Ok(summary)
    }
}
fn mailbox_eq(left: &str, right: &str) -> bool {
    left == right || (left.eq_ignore_ascii_case("INBOX") && right.eq_ignore_ascii_case("INBOX"))
}
