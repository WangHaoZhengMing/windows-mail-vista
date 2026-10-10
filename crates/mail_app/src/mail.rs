use super::{AppState, ComposerId, DataScope, Folder, MessageId, ReplyKind};
use mail_core::{
    Result,
    account::{Account, AccountId, AccountRecord, Authentication, Incoming},
    message::Draft,
    network::{self, Download, ImapChange, ImapLocation},
    store::atomic_write,
};
use std::{path::Path, sync::mpsc};

pub(super) enum WorkerResult {
    Credentials {
        account: AccountId,
        connection: String,
        authentication: Authentication,
        reply: mpsc::Sender<Result<()>>,
    },
    Moved {
        id: MessageId,
        result: Result<ImapChange>,
    },
    Synced {
        account: AccountId,
        result: Result<network::ImapSyncReport>,
    },
    Inbox {
        account: AccountId,
        result: Result<Vec<Download>>,
    },
    Sent {
        account: AccountId,
        id: MessageId,
        composer: ComposerId,
        result: Result<()>,
    },
    Changed {
        account: Option<AccountId>,
        action: DeleteAction,
        completed: Vec<(MessageId, Option<ImapLocation>)>,
        uncertain: Vec<(MessageId, String)>,
        deferred: Vec<(MessageId, ImapChange)>,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteAction {
    Trash,
    Restore,
    PermanentDelete,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DeletePolicy {
    LocalOnly,
    #[default]
    ImapTrash,
}

struct ChangePlan {
    id: MessageId,
    account: Option<AccountId>,
    remote: Option<(Account, ImapLocation)>,
    restore_mailbox: Option<String>,
}

/// Results are applied to the store before being reported to the UI.
pub enum MailCompletion {
    Moved {
        id: MessageId,
        deferred: bool,
        local_update: Result<()>,
    },
    Synced {
        account: AccountId,
        summary: mail_core::store::SyncSummary,
    },
    Received {
        account: AccountId,
        count: usize,
    },
    Sent {
        account: AccountId,
        composer: ComposerId,
        local_update: Result<()>,
    },
    Changed {
        account: Option<AccountId>,
        action: DeleteAction,
        count: usize,
        deferred: usize,
        local_update: Result<()>,
    },
    Failed(String),
    WorkerStopped,
}

impl AppState {
    pub fn account(&self) -> Option<&Account> {
        self.selected_account().map(|record| &record.account)
    }

    pub fn set_account(&mut self, account: Account) -> Result<()> {
        self.ensure_idle()?;
        if let Some(id) = self.active_account_id().cloned() {
            self.store.update_account(&id, account)
        } else {
            self.add_account(account).map(|_| ())
        }
    }

    pub fn accounts(&self) -> &[AccountRecord] {
        self.store.accounts()
    }
    pub fn default_account_id(&self) -> Option<&AccountId> {
        self.store.default_account_id()
    }
    pub fn add_account(&mut self, account: Account) -> Result<AccountId> {
        self.add_account_named(account.email.clone(), account)
    }
    pub fn add_account_named(&mut self, name: String, account: Account) -> Result<AccountId> {
        self.ensure_idle()?;
        self.store.add_account_named(name, account)
    }
    pub fn update_account(&mut self, id: &AccountId, account: Account) -> Result<()> {
        self.ensure_idle()?;
        self.store.update_account(id, account)
    }
    pub fn update_account_named(
        &mut self,
        id: &AccountId,
        name: String,
        account: Account,
    ) -> Result<()> {
        self.ensure_idle()?;
        self.store.update_account_named(id, name, account)
    }
    pub fn set_default_account(&mut self, id: &AccountId) -> Result<()> {
        self.ensure_idle()?;
        self.store.set_default_account(id)
    }
    pub fn remove_account(&mut self, id: &AccountId) -> Result<()> {
        self.ensure_idle()?;
        self.store.remove_account(id)?;
        if self.scope() == &DataScope::Account(id.clone()) {
            let scope = self
                .default_account_id()
                .cloned()
                .map(DataScope::Account)
                .unwrap_or(DataScope::Local);
            self.set_scope(scope)?;
            self.set_folder(Folder::Inbox);
        }
        Ok(())
    }

    pub fn import_path(&mut self, path: &Path) -> Result<usize> {
        self.store.import_tree(path)
    }

    pub fn export_selected(&self, path: &Path) -> Result<()> {
        let id = self.selected_id().ok_or("Select a message first")?;
        self.export_message(id, path)
    }
    pub fn export_message(&self, id: &MessageId, path: &Path) -> Result<()> {
        atomic_write(path, &self.store.raw(id.as_str())?)
    }

    pub fn delete_message(&mut self, id: &MessageId) -> Result<()> {
        self.change_messages(vec![id.clone()], DeleteAction::Trash)
    }
    pub fn restore_message(&mut self, id: &MessageId) -> Result<()> {
        self.change_messages(vec![id.clone()], DeleteAction::Restore)
    }
    pub fn permanently_delete_message(&mut self, id: &MessageId) -> Result<()> {
        self.change_messages(vec![id.clone()], DeleteAction::PermanentDelete)
    }

    pub fn delete_selected(&mut self) -> Result<()> {
        let id = self.selected_id().ok_or("Select a message first")?.clone();
        self.change_messages(vec![id], DeleteAction::Trash)
    }
    pub fn restore_selected(&mut self) -> Result<()> {
        let id = self.selected_id().ok_or("Select a message first")?.clone();
        self.change_messages(vec![id], DeleteAction::Restore)
    }
    pub fn permanently_delete_selected(&mut self) -> Result<()> {
        let id = self.selected_id().ok_or("Select a message first")?.clone();
        self.change_messages(vec![id], DeleteAction::PermanentDelete)
    }
    pub fn empty_trash(&mut self) -> Result<()> {
        let ids: Vec<_> = self
            .store
            .entries()
            .iter()
            .filter(|entry| {
                entry.folder == Folder::Trash.storage_key()
                    && !entry.pending_delete
                    && self.scope().matches(entry)
            })
            .map(|entry| MessageId::from_stored(entry.id.clone()))
            .collect();
        if ids.is_empty()
            && self.store.entries().iter().any(|entry| {
                entry.folder == Folder::Trash.storage_key()
                    && entry.pending_delete
                    && self.scope().matches(entry)
            })
        {
            return Err("Deleted Items contains messages awaiting server cleanup; synchronize or restore them".into());
        }
        self.change_messages(ids, DeleteAction::PermanentDelete)
    }
    pub fn delete_policy(&self) -> DeletePolicy {
        self.delete_policy
    }
    pub fn set_delete_policy(&mut self, policy: DeletePolicy) -> Result<()> {
        self.ensure_idle()?;
        self.delete_policy = policy;
        Ok(())
    }
    fn change_messages(&mut self, ids: Vec<MessageId>, action: DeleteAction) -> Result<()> {
        self.ensure_idle()?;
        let mut plans = Vec::new();
        for id in ids {
            let entry = self.store.entry(id.as_str()).ok_or("Message not found")?;
            let trash = entry.folder == Folder::Trash.storage_key();
            if entry.pending_delete && action == DeleteAction::PermanentDelete {
                return Err("This message is already marked for deletion; synchronize to check server cleanup, or restore it".into());
            }
            if (action == DeleteAction::Trash) == trash {
                return Err(if trash {
                    "Message is already in Deleted Items"
                } else {
                    "Select a message in Deleted Items"
                }
                .into());
            }
            let remote = if self.delete_policy == DeletePolicy::ImapTrash {
                if let Some(problem) = &entry.remote_problem {
                    return Err(problem.clone());
                }
                entry
                    .remote
                    .as_ref()
                    .map(|remote| {
                        let account = entry
                            .account_id
                            .as_ref()
                            .and_then(|id| self.store.account_record(id))
                            .ok_or("Message account no longer exists")?;
                        Ok::<_, String>((account.account.clone(), remote.clone()))
                    })
                    .transpose()?
            } else {
                None
            };
            plans.push(ChangePlan {
                id,
                account: entry.account_id.clone(),
                remote,
                restore_mailbox: entry.restore_mailbox.clone(),
            });
        }
        if !plans.iter().any(|plan| plan.remote.is_some()) {
            for plan in plans {
                self.apply_change(plan.id.as_str(), action, None)?;
            }
            return Ok(());
        }
        let account = plans
            .first()
            .and_then(|plan| plan.account.clone())
            .filter(|id| plans.iter().all(|plan| plan.account.as_ref() == Some(id)));
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut completed = Vec::new();
            let mut uncertain = Vec::new();
            let mut deferred = Vec::new();
            let mut prepared = std::collections::HashMap::<AccountId, Account>::new();
            let mut error = None;
            for plan in plans {
                let result = match plan.remote {
                    None => Ok(ImapChange::Deleted),
                    Some((mut account, location)) => (|| {
                        let owner = plan
                            .account
                            .as_ref()
                            .ok_or("Message account no longer exists")?;
                        if let Some(existing) = prepared.get(owner) {
                            account = existing.clone();
                        } else {
                            super::sync::refresh_credentials(owner, &mut account, &sender)?;
                            prepared.insert(owner.clone(), account.clone());
                        }
                        match action {
                            DeleteAction::Trash => network::imap_trash(&account, &location),
                            DeleteAction::Restore => network::imap_restore(
                                &account,
                                &location,
                                plan.restore_mailbox.as_deref().unwrap_or("INBOX"),
                            ),
                            DeleteAction::PermanentDelete => {
                                network::imap_permanently_delete(&account, &location)
                            }
                        }
                    })(),
                };
                match result {
                    Ok(ImapChange::Moved(location)) => completed.push((plan.id, Some(location))),
                    Ok(ImapChange::Deleted) => completed.push((plan.id, None)),
                    Ok(
                        change @ (ImapChange::MovedDeferred { .. } | ImapChange::MarkedDeleted(_)),
                    ) => deferred.push((plan.id, change)),
                    Ok(ImapChange::Uncertain(reason)) => {
                        uncertain.push((plan.id, reason.clone()));
                        error = Some(reason);
                        break;
                    }
                    Err(failure) => {
                        error = Some(failure);
                        break;
                    }
                }
            }
            let _ = sender.send(WorkerResult::Changed {
                account,
                action,
                completed,
                uncertain,
                deferred,
                error,
            });
        });
        self.worker = Some(receiver);
        Ok(())
    }
    fn apply_change(
        &mut self,
        id: &str,
        action: DeleteAction,
        moved: Option<ImapLocation>,
    ) -> Result<()> {
        match action {
            DeleteAction::Trash => self.store.trash(id, moved),
            DeleteAction::Restore => self.store.restore(id, moved),
            DeleteAction::PermanentDelete => self.store.permanently_delete(id),
        }
    }

    pub fn save_draft(&mut self, draft: &Draft) -> Result<MessageId> {
        let account = self.active_account_id().cloned();
        self.save_draft_for(account.as_ref(), draft)
    }
    pub fn save_draft_for(
        &mut self,
        account: Option<&AccountId>,
        draft: &Draft,
    ) -> Result<MessageId> {
        let sender = account
            .map(|id| self.store.account_record(id).ok_or("Account not found"))
            .transpose()?
            .map(|record| record.account.email.clone())
            .unwrap_or_else(|| "draft@winmail-rust.local".into());
        self.store
            .save_draft_for(account, &sender, draft)
            .map(MessageId::from_stored)
    }

    pub fn reply(&self, kind: ReplyKind) -> Result<Draft> {
        let id = self.selected_id().ok_or("Select a message first")?;
        self.reply_message(id, kind)
    }
    pub fn reply_message(&self, id: &MessageId, kind: ReplyKind) -> Result<Draft> {
        let parsed = self.message(id)?;
        if kind != ReplyKind::Forward {
            let mut draft = Draft::reply(&parsed);
            if kind == ReplyKind::All {
                draft.cc = format!("{},{}", parsed.summary.to, parsed.summary.cc)
                    .trim_matches(',')
                    .to_owned();
            }
            return Ok(draft);
        }
        let mut draft = Draft {
            subject: format!("Fwd: {}", parsed.summary.subject),
            body: format!(
                "\r\n\r\n-------- Forwarded message --------\r\nFrom: {}\r\nTo: {}\r\nDate: {}\r\nSubject: {}\r\n\r\n{}",
                parsed.summary.from,
                parsed.summary.to,
                parsed.summary.date,
                parsed.summary.subject,
                parsed.body
            ),
            ..Draft::default()
        };
        let directory = self.store.root().join("forward-files").join(id.as_str());
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        for (index, attachment) in parsed.attachments.iter().enumerate() {
            let path = directory.join(format!("{index}-{}", attachment.name));
            atomic_write(&path, &attachment.bytes)?;
            draft.attachments.push(path);
        }
        Ok(draft)
    }

    pub fn busy(&self) -> bool {
        self.worker.is_some()
    }

    pub fn receive(&mut self) -> Result<()> {
        let id = self
            .active_account_id()
            .ok_or("Configure an account first")?
            .clone();
        self.receive_for(&id)
    }
    pub fn receive_for(&mut self, id: &AccountId) -> Result<()> {
        self.ensure_idle()?;
        let id = id.clone();
        let mut account = self
            .store
            .account_record(&id)
            .ok_or("Account not found")?
            .account
            .clone();
        let known = self.store.known_remote_for(&id);
        let request = self.store.imap_sync_request(&id)?;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let refreshed = super::sync::refresh_credentials(&id, &mut account, &sender);
            if account.incoming == Incoming::Imap {
                let result = refreshed.and_then(|_| network::sync_imap(&account, &request));
                let _ = sender.send(WorkerResult::Synced {
                    account: id,
                    result,
                });
            } else {
                let result = refreshed.and_then(|_| network::receive(&account, &known));
                let _ = sender.send(WorkerResult::Inbox {
                    account: id,
                    result,
                });
            }
        });
        self.worker = Some(receiver);
        Ok(())
    }

    pub fn send(&mut self, draft: Draft, composer: ComposerId) -> Result<()> {
        let id = self
            .active_account_id()
            .ok_or("Configure an account first")?
            .clone();
        self.send_from(&id, draft, composer)
    }
    pub fn send_from(
        &mut self,
        account_id: &AccountId,
        draft: Draft,
        composer: ComposerId,
    ) -> Result<()> {
        self.ensure_idle()?;
        let mut account = self
            .store
            .account_record(account_id)
            .ok_or("Account not found")?
            .account
            .clone();
        let account_id = account_id.clone();
        let raw = draft.build(&account.email)?;
        let id = self
            .store
            .import_for(Some(&account_id), &raw, Folder::Outbox.storage_key(), None)
            .map(MessageId::from_stored)?;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = super::sync::refresh_credentials(&account_id, &mut account, &sender)
                .and_then(|_| network::smtp_send(&account, &draft, &raw));
            let _ = sender.send(WorkerResult::Sent {
                account: account_id,
                id,
                composer,
                result,
            });
        });
        self.worker = Some(receiver);
        Ok(())
    }

    pub(super) fn ensure_idle(&self) -> Result<()> {
        if self.busy() {
            Err("A mail operation is already running".into())
        } else {
            Ok(())
        }
    }

    pub fn poll(&mut self) -> Option<MailCompletion> {
        let result = loop {
            match self.worker.as_ref()?.try_recv() {
                Ok(WorkerResult::Credentials {
                    account,
                    connection,
                    authentication,
                    reply,
                }) => {
                    let saved = (|| {
                        let mut configuration = self
                            .store
                            .account_record(&account)
                            .ok_or("Account no longer exists")?
                            .account
                            .clone();
                        if configuration.id() != connection {
                            return Err("Account changed during authentication".into());
                        }
                        configuration.auth = authentication;
                        self.store.update_account(&account, configuration)
                    })();
                    let _ = reply.send(saved);
                }
                Ok(result) => break result,
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    return Some(MailCompletion::WorkerStopped);
                }
            }
        };
        self.worker = None;
        Some(match result {
            WorkerResult::Moved { id, result } => self.complete_move(id, result),
            WorkerResult::Credentials { .. } => {
                unreachable!("Credential events are handled before completion")
            }
            WorkerResult::Synced {
                account,
                result: Ok(report),
            } => match self.store.apply_imap_sync(&account, report) {
                Ok(summary) => {
                    self.reconcile_server_selection();
                    MailCompletion::Synced { account, summary }
                }
                Err(error) => MailCompletion::Failed(error),
            },
            WorkerResult::Synced {
                result: Err(error), ..
            } => MailCompletion::Failed(error),
            WorkerResult::Inbox {
                account,
                result: Ok(downloads),
            } => {
                let count = downloads.len();
                for item in downloads {
                    if let Err(error) = self.store.import_for(
                        Some(&account),
                        &item.raw,
                        Folder::Inbox.storage_key(),
                        Some(item.key),
                    ) {
                        return Some(MailCompletion::Failed(error));
                    }
                }
                MailCompletion::Received { account, count }
            }
            WorkerResult::Inbox {
                result: Err(error), ..
            }
            | WorkerResult::Sent {
                result: Err(error), ..
            } => MailCompletion::Failed(format!("Operation failed: {error}")),
            WorkerResult::Sent {
                account,
                id,
                composer,
                result: Ok(()),
            } => MailCompletion::Sent {
                account,
                composer,
                local_update: self.store.move_to(id.as_str(), Folder::Sent.storage_key()),
            },
            WorkerResult::Changed {
                account,
                action,
                completed,
                uncertain,
                deferred,
                error,
            } => {
                let mut count = 0;
                let mut pending = 0;
                let mut local_error = error;
                for (id, outcome) in deferred {
                    let result = match (action, outcome) {
                        (
                            DeleteAction::Trash,
                            ImapChange::MovedDeferred {
                                destination,
                                original,
                            },
                        ) => self
                            .store
                            .trash_deferred(id.as_str(), destination, original),
                        (
                            DeleteAction::Restore,
                            ImapChange::MovedDeferred {
                                destination,
                                original,
                            },
                        ) => self
                            .store
                            .restore_deferred(id.as_str(), destination, original),
                        (DeleteAction::PermanentDelete, ImapChange::MarkedDeleted(source)) => {
                            self.store.defer_permanent_delete(id.as_str(), source)
                        }
                        _ => Err("Unexpected deferred server operation".into()),
                    };
                    match result {
                        Ok(()) => {
                            count += 1;
                            pending += 1;
                        }
                        Err(error) => local_error = Some(error),
                    }
                }
                for (id, reason) in uncertain {
                    if let Err(error) = self.store.mark_remote_uncertain(id.as_str(), reason) {
                        local_error = Some(format!(
                            "Remote outcome is uncertain; recording its blocked UID failed: {error}"
                        ));
                    }
                }
                for (id, location) in completed {
                    match self.apply_change(id.as_str(), action, location) {
                        Ok(()) => count += 1,
                        Err(error) => {
                            local_error = Some(format!(
                                "Remote operation succeeded but local update failed: {error}"
                            ));
                            // Apply other completed remote operations too, so their
                            // successful results are not silently discarded.
                        }
                    }
                }
                MailCompletion::Changed {
                    account,
                    action,
                    count,
                    deferred: pending,
                    local_update: local_error.map_or(Ok(()), Err),
                }
            }
        })
    }
}
