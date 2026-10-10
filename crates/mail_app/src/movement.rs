//! Folder moves retain the local identity while applying the server's new UID.
use super::{AppState, Folder, MailCompletion, MessageId, mail::WorkerResult};
use mail_core::{
    Result,
    network::{self, ImapChange},
};
use std::sync::mpsc;

impl AppState {
    pub fn move_selected_to(&mut self, mailbox: &str) -> Result<()> {
        let id = self.selected_id().ok_or("Select a message first")?.clone();
        self.move_message_to(&id, mailbox)
    }
    pub fn move_message_to(&mut self, id: &MessageId, mailbox: &str) -> Result<()> {
        self.ensure_idle()?;
        let entry = self.message_entry(id).ok_or("Message not found")?;
        if let Some(problem) = &entry.remote_problem {
            return Err(problem.clone());
        }
        let owner = entry
            .account_id
            .clone()
            .ok_or("Local messages cannot be moved to a server mailbox")?;
        let target = self
            .store
            .server_folders(&owner)
            .into_iter()
            .find(|folder| network::mailbox_eq(&folder.mailbox, mailbox) && folder.selectable)
            .ok_or("Select a synchronized, writable server mailbox")?;
        if target.role == Some(network::ImapFolderRole::Trash) {
            return self.delete_message(id);
        }
        let source = entry
            .remote
            .clone()
            .ok_or("Upload of local messages is not implemented")?;
        if network::mailbox_eq(&source.mailbox, mailbox) {
            return Err("Message is already in this mailbox".into());
        }
        let restore = entry.folder == Folder::Trash.storage_key();
        let mut account = self
            .store
            .account_record(&owner)
            .ok_or("Account not found")?
            .account
            .clone();
        let id = id.clone();
        let target = mailbox.to_owned();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result =
                super::sync::refresh_credentials(&owner, &mut account, &sender).and_then(|_| {
                    if restore {
                        network::imap_restore(&account, &source, &target)
                    } else {
                        network::imap_move(&account, &source, &target)
                    }
                });
            let _ = sender.send(WorkerResult::Moved { id, result });
        });
        self.worker = Some(receiver);
        Ok(())
    }
    pub(super) fn complete_move(
        &mut self,
        id: MessageId,
        result: Result<ImapChange>,
    ) -> MailCompletion {
        let mut deferred = false;
        let local_update = match result {
            Ok(ImapChange::Moved(destination)) => self.store.move_remote(id.as_str(), destination),
            Ok(ImapChange::MovedDeferred {
                destination,
                original,
            }) => {
                deferred = true;
                self.store
                    .move_remote_deferred(id.as_str(), destination, original)
            }
            Ok(ImapChange::Uncertain(reason)) => self
                .store
                .mark_remote_uncertain(id.as_str(), reason.clone())
                .and(Err(reason)),
            Err(error) => Err(error),
            _ => Err("Unexpected server move result".into()),
        };
        MailCompletion::Moved {
            id,
            deferred,
            local_update,
        }
    }
    pub fn mark_selected_read(&mut self, read: bool) -> Result<()> {
        let id = self.selected_id().ok_or("Select a message first")?.clone();
        if read {
            self.store.mark_read(id.as_str())
        } else {
            self.store.mark_unread(id.as_str())
        }
    }
    pub fn toggle_selected_flag(&mut self) -> Result<()> {
        let id = self.selected_id().ok_or("Select a message first")?.clone();
        let flagged = !self
            .message_entry(&id)
            .ok_or("Message not found")?
            .flags
            .flagged;
        self.store.mark_flagged(id.as_str(), flagged)
    }
}
