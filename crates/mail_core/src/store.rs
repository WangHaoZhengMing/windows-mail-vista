use crate::{
    MAX_MESSAGE, Result,
    account::{Account, AccountId, AccountRecord},
    message::{self, Summary},
    network::{ImapFlags, ImapLocation},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub folder: String,
    pub summary: Summary,
    pub read: bool,
    #[serde(default)]
    pub flags: ImapFlags,
    #[serde(default)]
    pub pending_delete: bool,
    pub remote_key: Option<String>,
    /// Stable owner; None means a local import or a removed account's preserved mail.
    #[serde(default)]
    pub account_id: Option<AccountId>,
    #[serde(default)]
    pub remote: Option<ImapLocation>,
    /// An acknowledged/lost server mutation whose resulting identity is unknown.
    #[serde(default)]
    pub remote_problem: Option<String>,
    #[serde(default)]
    pub trash_origin: Option<String>,
    /// Present only when an IMAP move to Trash completed successfully.
    #[serde(default)]
    pub restore_mailbox: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct SeenRemote {
    account_id: Option<AccountId>,
    key: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Catalog {
    #[serde(default)]
    messages: Vec<Entry>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<Account>,
    #[serde(default)]
    accounts: Vec<AccountRecord>,
    #[serde(default)]
    default_account: Option<AccountId>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    remote_seen: Vec<String>,
    #[serde(default)]
    seen: Vec<SeenRemote>,
    // A failed file cleanup must never resurrect a permanently deleted message.
    #[serde(default)]
    purged: Vec<String>,
    #[serde(default)]
    drafts: std::collections::BTreeMap<String, crate::message::Draft>,
    #[serde(default)]
    server_folders: Vec<sync::StoredFolder>,
    #[serde(default)]
    pending_flags: Vec<sync::StoredFlagChange>,
    #[serde(default)]
    pending_compactions: Vec<PendingCompaction>,
}
pub struct Store {
    root: PathBuf,
    catalog: Catalog,
    _lock: std::fs::File,
}

pub fn default_root() -> PathBuf {
    crate::platform::default_data_dir()
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), nonce));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root.join("messages")).map_err(|e| e.to_string())?;
        let root = &std::fs::canonicalize(root).map_err(|e| e.to_string())?;
        let lock = root.join("store.lock");
        let lock = crate::platform::open_store_lock(&lock)
            .map_err(|error| format!("Mail store is in use or unavailable: {error}"))?;
        let path = root.join("index.json");
        let catalog = if path.is_file() {
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Store index: {e}"))?
        } else {
            Catalog::default()
        };
        let mut store = Self {
            root: root.to_owned(),
            catalog,
            _lock: lock,
        };
        store.migrate()?;
        store.recover()?;
        Ok(store)
    }
    fn migrate(&mut self) -> Result<()> {
        let mut changed = false;
        if let Some(account) = self.catalog.account.take() {
            let fingerprint = account.id();
            let id = AccountId::new();
            self.catalog.accounts.push(AccountRecord {
                id: id.clone(),
                name: account.email.clone(),
                account,
            });
            self.catalog.default_account.get_or_insert(id.clone());
            for entry in &mut self.catalog.messages {
                if entry.account_id.is_none()
                    && entry
                        .remote_key
                        .as_ref()
                        .is_some_and(|key| key.starts_with(&format!("{fingerprint}:")))
                {
                    entry.account_id = Some(id.clone());
                    entry.remote = entry
                        .remote_key
                        .as_ref()
                        .and_then(|key| ImapLocation::from_key(key));
                }
            }
            changed = true;
        }
        for key in std::mem::take(&mut self.catalog.remote_seen) {
            let account_id = self
                .catalog
                .accounts
                .iter()
                .find(|record| key.starts_with(&format!("{}:", record.account.id())))
                .map(|record| record.id.clone());
            self.catalog.seen.push(SeenRemote { account_id, key });
            changed = true;
        }
        if self
            .catalog
            .default_account
            .as_ref()
            .is_some_and(|id| !self.catalog.accounts.iter().any(|r| &r.id == id))
            || (self.catalog.default_account.is_none() && !self.catalog.accounts.is_empty())
        {
            self.catalog.default_account = self.catalog.accounts.first().map(|r| r.id.clone());
            changed = true;
        }
        for entry in &mut self.catalog.messages {
            if entry.flags.seen != entry.read {
                entry.flags.seen = entry.read;
                changed = true;
            }
        }
        if changed {
            self.flush()?;
        }
        Ok(())
    }
    fn recover(&mut self) -> Result<()> {
        let mut changed = false;
        for file in std::fs::read_dir(self.root.join("messages")).map_err(|e| e.to_string())? {
            let file = file.map_err(|e| e.to_string())?;
            let name = file.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".eml")
                && id.len() == 64
                && id.bytes().all(|b| b.is_ascii_hexdigit())
                && !self.catalog.messages.iter().any(|e| e.id == id)
                && !self.catalog.purged.iter().any(|e| e == id)
            {
                let raw = read_bounded(&file.path())?;
                if let Ok(parsed) = message::parse(&raw) {
                    self.catalog.messages.push(Entry {
                        id: id.into(),
                        folder: "Recovered".into(),
                        summary: parsed.summary,
                        read: false,
                        flags: ImapFlags::default(),
                        pending_delete: false,
                        remote_key: None,
                        account_id: None,
                        remote: None,
                        remote_problem: None,
                        trash_origin: None,
                        restore_mailbox: None,
                    });
                    changed = true;
                }
            }
        }
        if changed {
            self.flush()?;
        }
        Ok(())
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn entries(&self) -> &[Entry] {
        &self.catalog.messages
    }
    pub fn account(&self) -> Option<&Account> {
        self.default_account_id()
            .and_then(|id| self.account_record(id))
            .map(|r| &r.account)
    }
    pub fn set_account(&mut self, account: Account) -> Result<()> {
        if let Some(id) = self.default_account_id().cloned() {
            self.update_account(&id, account)
        } else {
            self.add_account(account).map(|_| ())
        }
    }
    pub fn accounts(&self) -> &[AccountRecord] {
        &self.catalog.accounts
    }
    pub fn default_account_id(&self) -> Option<&AccountId> {
        self.catalog.default_account.as_ref()
    }
    pub fn account_record(&self, id: &AccountId) -> Option<&AccountRecord> {
        self.catalog.accounts.iter().find(|r| &r.id == id)
    }
    fn commit<T>(&mut self, change: impl FnOnce(&mut Catalog) -> Result<T>) -> Result<T> {
        let previous = self.catalog.clone();
        let result = change(&mut self.catalog).and_then(|value| self.flush().map(|_| value));
        if result.is_err() {
            self.catalog = previous;
        }
        result
    }
    pub fn add_account(&mut self, account: Account) -> Result<AccountId> {
        self.add_account_named(account.email.clone(), account)
    }
    pub fn add_account_named(&mut self, name: String, account: Account) -> Result<AccountId> {
        account.validate()?;
        let name = account_name(name, &account)?;
        let id = AccountId::new();
        self.commit(|catalog| {
            catalog.accounts.push(AccountRecord {
                id: id.clone(),
                name,
                account,
            });
            catalog.default_account.get_or_insert(id.clone());
            Ok(id)
        })
    }
    pub fn update_account(&mut self, id: &AccountId, account: Account) -> Result<()> {
        let name = self
            .account_record(id)
            .ok_or("Account not found")?
            .name
            .clone();
        self.update_account_named(id, name, account)
    }
    pub fn update_account_named(
        &mut self,
        id: &AccountId,
        name: String,
        account: Account,
    ) -> Result<()> {
        account.validate()?;
        let name = account_name(name, &account)?;
        self.commit(|catalog| {
            let record = catalog
                .accounts
                .iter_mut()
                .find(|r| &r.id == id)
                .ok_or("Account not found")?;
            let previous_connection = record.account.id();
            let connection_changed = previous_connection != account.id();
            record.name = name;
            record.account = account;
            if connection_changed {
                // A stable account record can be edited to a different server, but
                // the old server's UID space must never appear in the new folders.
                for entry in &mut catalog.messages {
                    if entry.account_id.as_ref() == Some(id)
                        && entry
                            .remote_key
                            .as_ref()
                            .is_some_and(|key| key.starts_with(&format!("{previous_connection}:")))
                    {
                        entry.folder = "Recovered".into();
                        entry.remote = None;
                        entry.remote_problem = Some(
                            "Account connection changed; previous server copy preserved".into(),
                        );
                        entry.pending_delete = false;
                        entry.restore_mailbox = None;
                        entry.trash_origin = None;
                    }
                }
                catalog
                    .server_folders
                    .retain(|folder| &folder.account_id != id);
                catalog
                    .pending_flags
                    .retain(|pending| &pending.account_id != id);
                catalog
                    .pending_compactions
                    .retain(|pending| &pending.account_id != id);
            }
            Ok(())
        })
    }
    pub fn set_default_account(&mut self, id: &AccountId) -> Result<()> {
        if self.account_record(id).is_none() {
            return Err("Account not found".into());
        }
        self.commit(|catalog| {
            catalog.default_account = Some(id.clone());
            Ok(())
        })
    }
    pub fn remove_account(&mut self, id: &AccountId) -> Result<()> {
        if self.account_record(id).is_none() {
            return Err("Account not found".into());
        }
        self.commit(|catalog| {
            catalog.accounts.retain(|r| &r.id != id);
            if catalog.default_account.as_ref() == Some(id) {
                catalog.default_account = catalog.accounts.first().map(|r| r.id.clone());
            }
            for entry in &mut catalog.messages {
                if entry.account_id.as_ref() == Some(id) {
                    entry.account_id = None;
                    entry.remote = None;
                    entry.remote_problem = None;
                    entry.remote_key = None;
                    entry.restore_mailbox = None;
                    entry.pending_delete = false;
                }
            }
            catalog
                .seen
                .retain(|seen| seen.account_id.as_ref() != Some(id));
            catalog
                .server_folders
                .retain(|folder| &folder.account_id != id);
            catalog
                .pending_flags
                .retain(|change| &change.account_id != id);
            catalog
                .pending_compactions
                .retain(|change| &change.account_id != id);
            Ok(())
        })
    }
    pub fn flush(&self) -> Result<()> {
        atomic_write(
            &self.root.join("index.json"),
            &serde_json::to_vec_pretty(&self.catalog).map_err(|e| e.to_string())?,
        )
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid message id".into());
        }
        Ok(self.root.join("messages").join(format!("{id}.eml")))
    }
    pub fn raw(&self, id: &str) -> Result<Vec<u8>> {
        read_bounded(&self.path(id)?)
    }
    pub fn import(&mut self, raw: &[u8], folder: &str, remote: Option<String>) -> Result<String> {
        let owner = remote
            .as_ref()
            .and_then(|key| {
                self.default_account_id()
                    .and_then(|id| self.account_record(id))
                    .filter(|record| key.starts_with(&format!("{}:", record.account.id())))
                    .or_else(|| {
                        self.accounts()
                            .iter()
                            .find(|r| key.starts_with(&format!("{}:", r.account.id())))
                    })
                    .map(|r| r.id.clone())
            })
            .or_else(|| {
                matches!(folder, "Outbox" | "Sent" | "Drafts")
                    .then(|| self.default_account_id().cloned())
                    .flatten()
            });
        self.import_for(owner.as_ref(), raw, folder, remote)
    }
    pub fn import_for(
        &mut self,
        account: Option<&AccountId>,
        raw: &[u8],
        folder: &str,
        remote: Option<String>,
    ) -> Result<String> {
        if account.is_some_and(|id| self.account_record(id).is_none()) {
            return Err("Account not found".into());
        }
        let parsed = message::parse(raw)?;
        let id = if let Some(account) = account {
            use sha2::{Digest, Sha256};
            let mut hash = Sha256::new();
            hash.update(account.as_str().as_bytes());
            hash.update([0]);
            hash.update(remote.as_deref().unwrap_or_default().as_bytes());
            hash.update([0]);
            hash.update(raw);
            format!("{:x}", hash.finalize())
        } else {
            message::hash(raw)
        };
        if self.catalog.messages.iter().any(|e| e.id == id) {
            self.commit(|catalog| {
                remember(catalog, account, remote.as_deref());
                Ok(())
            })?;
            return Ok(id);
        }
        atomic_write(&self.path(&id)?, raw)?;
        self.commit(|catalog| {
            remember(catalog, account, remote.as_deref());
            catalog.purged.retain(|purged| purged != &id);
            catalog.messages.push(Entry {
                id: id.clone(),
                folder: folder.into(),
                summary: parsed.summary,
                read: false,
                flags: ImapFlags::default(),
                pending_delete: false,
                account_id: account.cloned(),
                remote: account
                    .and(remote.as_ref())
                    .and_then(|key| ImapLocation::from_key(key)),
                remote_problem: None,
                remote_key: remote,
                trash_origin: None,
                restore_mailbox: None,
            });
            Ok(())
        })?;
        Ok(id)
    }
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.catalog.messages.iter().find(|e| e.id == id)
    }
    pub fn mark_remote_uncertain(&mut self, id: &str, reason: String) -> Result<()> {
        let result = self.commit(|catalog| {
            let entry = catalog
                .messages
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or("Message not found")?;
            entry.remote = None;
            entry.remote_problem = Some(reason.clone());
            catalog.pending_flags.retain(|change| {
                change.change.location.remote_key()
                    != entry.remote_key.as_deref().unwrap_or_default()
            });
            Ok(())
        });
        // Even when the disk is unavailable, don't reuse the stale UID in this process.
        if result.is_err()
            && let Some(entry) = self
                .catalog
                .messages
                .iter_mut()
                .find(|entry| entry.id == id)
        {
            entry.remote = None;
            entry.remote_problem = Some(reason);
        }
        result
    }
    /// Local catalog operation for compatibility adapters. UI remote changes are
    /// performed by mail_app before calling trash/restore/permanently_delete.
    pub fn move_to(&mut self, id: &str, folder: &str) -> Result<()> {
        if folder == "Trash" {
            return self.trash(id, None);
        }
        self.commit(|catalog| {
            catalog
                .messages
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or("Message not found")?
                .folder = folder.into();
            Ok(())
        })
    }
    /// Apply only after the remote move has succeeded, if a remote identity exists.
    pub fn trash(&mut self, id: &str, moved: Option<ImapLocation>) -> Result<()> {
        self.commit(|catalog| {
            let entry = catalog
                .messages
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or("Message not found")?;
            if entry.folder == "Trash" {
                return Err("Message is already in Deleted Items".into());
            }
            entry.trash_origin = Some(entry.folder.clone());
            entry.restore_mailbox = moved
                .as_ref()
                .and(entry.remote.as_ref())
                .map(|remote| remote.mailbox.clone());
            entry.folder = "Trash".into();
            if let Some(remote) = moved {
                let old_remote = entry.remote.clone();
                entry.remote_key = Some(remote.remote_key());
                entry.remote = Some(remote.clone());
                entry.flags.deleted = false;
                entry.pending_delete = false;
                let owner = entry.account_id.clone();
                for pending in &mut catalog.pending_flags {
                    if Some(&pending.change.location) == old_remote.as_ref() {
                        pending.change.location = remote.clone();
                    }
                }
                remember(catalog, owner.as_ref(), Some(&remote.remote_key()));
            }
            Ok(())
        })
    }
    pub fn restore(&mut self, id: &str, moved: Option<ImapLocation>) -> Result<()> {
        self.commit(|catalog| {
            let entry = catalog
                .messages
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or("Message not found")?;
            if entry.folder != "Trash" {
                return Err("Only Deleted Items can be restored".into());
            }
            entry.folder = entry.trash_origin.take().unwrap_or_else(|| "Inbox".into());
            entry.restore_mailbox = None;
            let original = entry.remote.clone();
            if let Some(remote) = moved {
                entry.remote_key = Some(remote.remote_key());
                entry.remote = Some(remote);
            }
            entry.flags.deleted = false;
            entry.pending_delete = false;
            let account = entry.account_id.clone();
            let destination = entry.remote.clone();
            let key = destination.as_ref().map(ImapLocation::remote_key);
            for pending in &mut catalog.pending_flags {
                if Some(&pending.change.location) == original.as_ref()
                    && let Some(destination) = &destination
                {
                    pending.change.location = destination.clone();
                }
            }
            catalog
                .pending_compactions
                .retain(|pending| pending.local_id != id || !pending.purge_when_absent);
            remember(catalog, account.as_ref(), key.as_deref());
            Ok(())
        })
    }
    /// Catalog tombstones prevent recovery after an interrupted disk cleanup.
    pub fn permanently_delete(&mut self, id: &str) -> Result<()> {
        let path = self.path(id)?;
        if self.entry(id).is_none_or(|entry| entry.folder != "Trash") {
            return Err("Only Deleted Items can be permanently deleted".into());
        }
        self.commit(|catalog| {
            let remote = catalog
                .messages
                .iter()
                .find(|entry| entry.id == id)
                .and_then(|entry| entry.remote.clone());
            catalog.messages.retain(|entry| entry.id != id);
            catalog.drafts.remove(id);
            catalog
                .pending_flags
                .retain(|pending| Some(&pending.change.location) != remote.as_ref());
            catalog
                .pending_compactions
                .retain(|pending| pending.local_id != id);
            if !catalog.purged.iter().any(|purged| purged == id) {
                catalog.purged.push(id.into());
            }
            Ok(())
        })?;
        // Paths are derived from a validated ID, never from attachment filenames.
        if let Err(error) = std::fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(format!(
                "Message removed from catalog; file cleanup failed: {error}"
            ));
        }
        for kind in ["draft-files", "forward-files"] {
            if let Err(error) = std::fs::remove_dir_all(self.root.join(kind).join(id))
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(format!(
                    "Message removed from catalog; attachment cleanup failed: {error}"
                ));
            }
        }
        Ok(())
    }
    pub fn mark_read(&mut self, id: &str) -> Result<()> {
        let mut flags = self.entry(id).ok_or("Message not found")?.flags.clone();
        flags.seen = true;
        self.set_message_flags(id, flags)
    }
    pub fn mark_unread(&mut self, id: &str) -> Result<()> {
        let mut flags = self.entry(id).ok_or("Message not found")?.flags.clone();
        flags.seen = false;
        self.set_message_flags(id, flags)
    }
    pub fn mark_flagged(&mut self, id: &str, flagged: bool) -> Result<()> {
        let mut flags = self.entry(id).ok_or("Message not found")?.flags.clone();
        flags.flagged = flagged;
        self.set_message_flags(id, flags)
    }
    pub fn known_remote(&self) -> Vec<String> {
        let mut keys: Vec<_> = self
            .catalog
            .seen
            .iter()
            .map(|seen| seen.key.clone())
            .collect();
        keys.extend(
            self.catalog
                .messages
                .iter()
                .filter_map(|e| e.remote_key.clone()),
        );
        keys.sort();
        keys.dedup();
        keys
    }
    pub fn known_remote_for(&self, account: &AccountId) -> Vec<String> {
        let mut keys: Vec<_> = self
            .catalog
            .seen
            .iter()
            .filter(|seen| seen.account_id.as_ref() == Some(account))
            .map(|seen| seen.key.clone())
            .collect();
        keys.extend(
            self.entries()
                .iter()
                .filter(|e| e.account_id.as_ref() == Some(account))
                .filter_map(|e| e.remote_key.clone()),
        );
        keys.sort();
        keys.dedup();
        keys
    }
    pub fn save_draft(&mut self, sender: &str, draft: &crate::message::Draft) -> Result<String> {
        let account = self.default_account_id().cloned();
        self.save_draft_for(account.as_ref(), sender, draft)
    }
    pub fn save_draft_for(
        &mut self,
        account: Option<&AccountId>,
        sender: &str,
        draft: &crate::message::Draft,
    ) -> Result<String> {
        let raw = draft.build_draft(sender)?;
        let id = self.import_for(account, &raw, "Drafts", None)?;
        let parsed = message::parse(&raw)?;
        let mut saved = draft.clone();
        saved.attachments.clear();
        if !parsed.attachments.is_empty() {
            let directory = self.root.join("draft-files").join(&id);
            std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            for (i, attachment) in parsed.attachments.iter().enumerate() {
                let path = directory.join(format!("{i}-{}", attachment.name));
                atomic_write(&path, &attachment.bytes)?;
                saved.attachments.push(path);
            }
        }
        self.commit(|catalog| {
            catalog.drafts.insert(id.clone(), saved);
            Ok(())
        })?;
        Ok(id)
    }
    pub fn draft(&self, id: &str) -> Option<&crate::message::Draft> {
        self.catalog.drafts.get(id)
    }
    pub fn import_tree(&mut self, path: &Path) -> Result<usize> {
        fn walk(store: &mut Store, path: &Path, depth: usize, count: &mut usize) -> Result<()> {
            if depth > 32 {
                return Err("Import directory exceeds 32 levels".into());
            }
            let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            if metadata.file_type().is_symlink() {
                return Ok(());
            }
            if metadata.is_dir() {
                for item in std::fs::read_dir(path).map_err(|e| e.to_string())? {
                    walk(
                        store,
                        &item.map_err(|e| e.to_string())?.path(),
                        depth + 1,
                        count,
                    )?;
                }
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("eml") || e.eq_ignore_ascii_case("nws"))
            {
                store.import(&read_bounded(path)?, "Inbox", None)?;
                *count += 1;
            }
            Ok(())
        }
        let mut count = 0;
        walk(self, path, 0, &mut count)?;
        Ok(count)
    }
}
fn account_name(name: String, account: &Account) -> Result<String> {
    let name = name.trim();
    if name.contains(['\r', '\n', '\0']) {
        return Err("Account name contains control characters".into());
    }
    Ok(if name.is_empty() {
        account.email.clone()
    } else {
        name.into()
    })
}
fn remember(catalog: &mut Catalog, account: Option<&AccountId>, key: Option<&str>) {
    if let Some(key) = key
        && !catalog
            .seen
            .iter()
            .any(|seen| seen.account_id.as_ref() == account && seen.key == key)
    {
        catalog.seen.push(SeenRemote {
            account_id: account.cloned(),
            key: key.into(),
        });
    }
}
pub fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take((MAX_MESSAGE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_MESSAGE {
        Err("File exceeds 50 MiB".into())
    } else {
        Ok(bytes)
    }
}

mod sync;
pub use sync::{PendingCompaction, SyncSummary};
