//! Background synchronization and worker completions.
use super::*;

impl MainWindow {
    pub(super) fn poll(&mut self) {
        if self.app.can_start_sync() {
            while let Some(account) = self.startup_sync.pop_front() {
                // An account may have been removed while earlier accounts synchronized.
                if !self
                    .app
                    .accounts()
                    .iter()
                    .any(|record| record.id == account)
                {
                    continue;
                }
                match self.app.receive_for(&account) {
                    Ok(()) => {
                        self.sync_continuation.insert(
                            account.clone(),
                            SyncProgress {
                                remaining: 49,
                                downloaded: 0,
                            },
                        );
                        self.status(&format!(
                            "{}: starting background synchronization…",
                            self.account_label(&account)
                        ));
                        self.update_actions();
                        if !self.app.can_start_sync() {
                            break;
                        }
                    }
                    Err(error) => self.status(&error),
                }
            }
        }
        // Drain bursts of download updates so completion does not sit behind
        // hundreds of status messages, while keeping each UI tick bounded.
        for _ in 0..64 {
            let Some(completion) = self.app.poll() else {
                break;
            };
            let progress = matches!(completion, MailCompletion::Progress { .. });
            self.mail_completed(completion);
            if !progress {
                break;
            }
        }
    }
    pub(super) fn mail_completed(&mut self, completion: MailCompletion) {
        let completion = match completion {
            MailCompletion::Progress { account, message } => {
                self.status(&format!("{}: {message}", self.account_label(&account)));
                return;
            }
            completion => completion,
        };
        if let Some(pending) = self.pending_reader_deletion.take() {
            let entry = self.app.message_entry(&pending.message);
            let finished = match pending.action {
                DeleteAction::Trash => {
                    entry.is_some_and(|entry| entry.folder == Folder::Trash.storage_key())
                }
                DeleteAction::PermanentDelete => entry.is_none(),
                DeleteAction::Restore => false,
            };
            if finished {
                for reader in self
                    .readers
                    .iter()
                    .filter(|reader| reader.id() == &pending.message)
                {
                    reader.close();
                }
            }
        }
        match completion {
            MailCompletion::Cancelled { account } => {
                self.sync_continuation.remove(&account);
                self.refresh();
                self.status(&format!(
                    "{}: synchronization stopped; completed downloads will resume next time",
                    self.account_label(&account)
                ));
            }
            MailCompletion::SyncFailed { account, error } => {
                self.sync_continuation.remove(&account);
                self.refresh();
                self.status(&format!("{}: {error}", self.account_label(&account)));
            }
            MailCompletion::Progress { .. } => unreachable!("Progress handled above"),
            MailCompletion::Synced { account, summary } => {
                self.rebuild_folders();
                self.refresh();
                let progress = self.sync_continuation.remove(&account);
                let remaining = progress.as_ref().map_or(0, |progress| progress.remaining);
                let downloaded =
                    summary.downloaded + progress.map_or(0, |progress| progress.downloaded);
                if summary.has_more
                    && summary.downloaded > 0
                    && summary.warnings.is_empty()
                    && remaining > 0
                {
                    match self.app.receive_for(&account) {
                        Ok(()) => {
                            self.sync_continuation.insert(
                                account.clone(),
                                SyncProgress {
                                    remaining: remaining - 1,
                                    downloaded,
                                },
                            );
                            self.status(&format!(
                                "{}: synchronized {} messages, downloading the next batch…",
                                self.account_label(&account),
                                downloaded
                            ));
                        }
                        Err(error) => self.status(&error),
                    }
                } else {
                    let detail = if !summary.warnings.is_empty() {
                        format!(" · {}", summary.warnings.join("; "))
                    } else if summary.has_more {
                        " · More messages remain; choose Send/Receive to continue".into()
                    } else {
                        String::new()
                    };
                    self.status(&format!("{}: synchronized {} new messages, {} recovered, {} pending server operations{}", self.account_label(&account), downloaded, summary.recovered, summary.pending, detail));
                }
            }
            MailCompletion::Moved {
                deferred,
                local_update,
                ..
            } => {
                self.refresh();
                match local_update {
                    Ok(()) => self.status(if deferred {
                        "Message moved; source copy awaits server cleanup"
                    } else {
                        "Message moved"
                    }),
                    Err(error) => self.status(&error),
                }
            }
            MailCompletion::Received { account, count } => {
                self.sync_continuation.remove(&account);
                self.refresh();
                self.status(&format!(
                    "Received {count} messages for {}",
                    self.account_label(&account)
                ));
            }
            MailCompletion::Sent {
                composer,
                local_update,
                ..
            } => {
                if let Some(window) = self.composers.get(composer.index()) {
                    window.sent();
                    window.saved();
                    window.close();
                }
                match local_update {
                    Ok(()) => self.status("SMTP server accepted the message"),
                    Err(error) => self.status(&format!(
                        "Message accepted by SMTP; local Sent update failed: {error}"
                    )),
                }
                self.refresh();
            }
            MailCompletion::Failed(error) => {
                for window in &self.composers {
                    window.set_sending(false);
                }
                self.sync_continuation.clear();
                self.refresh();
                self.status(&error);
            }
            MailCompletion::Changed {
                action,
                count,
                deferred,
                local_update,
                ..
            } => {
                self.refresh();
                match local_update {
                    Ok(()) if deferred > 0 => self.status(&format!("{count} message operation(s) accepted; {deferred} source copy/copies awaiting server cleanup")),
                    Ok(()) => self.status(&format!(
                        "{count} message(s) {}",
                        match action {
                            DeleteAction::Trash => "moved to Deleted Items",
                            DeleteAction::Restore => "restored",
                            DeleteAction::PermanentDelete => "permanently deleted",
                        }
                    )),
                    Err(error) => self.status(&error),
                }
            }
            MailCompletion::WorkerStopped => {
                for window in &self.composers {
                    window.set_sending(false);
                }
                self.sync_continuation.clear();
                self.update_actions();
                self.status("Mail worker stopped unexpectedly");
            }
        }
    }
}
