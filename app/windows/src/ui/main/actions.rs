//! Mail commands, readers, composers, and account changes.
use super::*;

impl MainWindow {
    pub(super) fn new_message(&mut self, draft: Draft) {
        let account = self
            .app
            .selected_account_id()
            .cloned()
            .or_else(|| self.app.active_account_id().cloned());
        self.new_message_from(draft, account);
    }
    pub(super) fn new_message_from(&mut self, draft: Draft, account: Option<AccountId>) {
        match compose::Window::open_for(
            self.controls.main,
            draft,
            self.app.accounts().to_vec(),
            account,
        ) {
            Ok(window) => {
                let id = ComposerId::new(self.composers.len());
                self.composers.push(window);
                self.active_composer = Some(id);
            }
            Err(error) => self.status(&error),
        }
    }
    pub(super) fn open_message(&mut self, id: MessageId) -> Result<()> {
        self.readers.retain(reader::Window::active);
        if let Some(reader) = self.readers.iter().find(|reader| reader.id() == &id) {
            reader.activate();
            return Ok(());
        }
        match self.app.read_message(&id)? {
            Selection::Draft(draft) => {
                let sender = self
                    .app
                    .message_entry(&id)
                    .and_then(|entry| entry.account_id.clone())
                    .or_else(|| self.app.default_account_id().cloned());
                self.new_message_from(draft, sender);
            }
            Selection::Message(message) => {
                self.readers
                    .push(reader::Window::open(self.controls.main, id, message)?);
            }
        }
        self.refresh();
        Ok(())
    }
    pub(super) fn save_message_attachments(&self, owner: HWND, id: &MessageId) -> Result<()> {
        let message = self.app.message(id)?;
        let attachments: Vec<_> = message.downloadable_attachments().collect();
        if attachments.is_empty() {
            return Err("This message has no attachments".into());
        }
        for attachment in attachments {
            if let Some(path) = unsafe { file_dialog(owner, true, "All files\0*.*\0\0") } {
                crate::store::atomic_write(&path, &attachment.bytes)?;
            }
        }
        Ok(())
    }
    pub(super) fn reader_command(&mut self, reader: HWND, command: Command) -> Result<()> {
        let id = self
            .readers
            .iter()
            .find(|window| window.hwnd() == reader && window.active())
            .ok_or("Reading window has closed")?
            .id()
            .clone();
        let entry = self
            .app
            .message_entry(&id)
            .ok_or("Message no longer exists")?;
        let sender = entry
            .account_id
            .clone()
            .or_else(|| self.app.default_account_id().cloned());
        let trash = entry.folder == Folder::Trash.storage_key();
        match command {
            Command::Reply | Command::ReplyAll | Command::Forward => {
                let kind = match command {
                    Command::Reply => ReplyKind::Sender,
                    Command::ReplyAll => ReplyKind::All,
                    _ => ReplyKind::Forward,
                };
                let draft = self.app.reply_message(&id, kind)?;
                self.new_message_from(draft, sender);
            }
            Command::SaveAttachments => self.save_message_attachments(reader, &id)?,
            Command::Export => {
                if let Some(path) = unsafe { file_dialog(reader, true, "EML files\0*.eml\0\0") } {
                    self.app.export_message(&id, &path)?;
                }
            }
            Command::Delete => {
                if trash {
                    if !self.confirm("Permanently delete this message? This cannot be undone.") {
                        return Ok(());
                    }
                    self.app.permanently_delete_message(&id)?;
                } else {
                    self.app.delete_message(&id)?;
                }
                if self.app.busy() {
                    self.pending_reader_deletion = Some(PendingReaderDeletion {
                        message: id,
                        action: if trash {
                            DeleteAction::PermanentDelete
                        } else {
                            DeleteAction::Trash
                        },
                    });
                } else if let Some(window) =
                    self.readers.iter().find(|window| window.hwnd() == reader)
                {
                    window.close();
                }
                self.refresh();
            }
            _ => return Err("This action is not available in a reading window".into()),
        }
        self.update_actions();
        Ok(())
    }
    pub(super) fn composer(&self) -> Result<&compose::Window> {
        self.active_composer
            .and_then(|id| self.composers.get(id.index()))
            .filter(|window| window.active())
            .ok_or_else(|| "Choose Create Mail to compose a message first".into())
    }
    pub(super) fn close_composer(&mut self) -> Result<()> {
        if let Ok(window) = self.composer() {
            let draft = window.dirty().then(|| window.draft());
            if let Some(draft) = draft {
                let account = self.composer()?.sender_id();
                self.app.save_draft_for(account.as_ref(), &draft)?;
            }
            self.composer()?.close();
            self.refresh();
        }
        Ok(())
    }
    pub(super) fn apply(&mut self, command: &str) {
        for word in command_words(command) {
            let result = (|| -> Result<()> {
                match LaunchAction::parse(&word)? {
                    LaunchAction::Ignore => {}
                    LaunchAction::Compose(draft) => self.new_message(draft),
                    LaunchAction::Import(path) => {
                        let count = self.app.import_path(&path)?;
                        self.app.set_scope(DataScope::Local)?;
                        self.app.set_folder(Folder::Inbox);
                        self.rebuild_folders();
                        self.refresh();
                        self.status(&format!("Imported {count} EML/NWS files"));
                    }
                    LaunchAction::Unsupported(argument) => {
                        self.status(&format!("Unsupported runtime argument: {argument}"))
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                self.status(&error);
            }
        }
    }
    pub(super) fn action(&mut self, hwnd: HWND, command: Command) -> Result<()> {
        match command {
            Command::NewMessage => self.new_message(Draft::default()),
            Command::Reply | Command::ReplyAll | Command::Forward => {
                let kind = match command {
                    Command::Reply => ReplyKind::Sender,
                    Command::ReplyAll => ReplyKind::All,
                    _ => ReplyKind::Forward,
                };
                let draft = self.app.reply(kind)?;
                self.new_message(draft);
            }
            Command::Import => {
                if let Some(path) = unsafe {
                    file_dialog(hwnd, false, "EML files\0*.eml;*.nws\0All files\0*.*\0\0")
                } {
                    let count = self.app.import_path(&path)?;
                    self.app.set_scope(DataScope::Local)?;
                    self.app.set_folder(Folder::Inbox);
                    self.rebuild_folders();
                    self.refresh();
                    self.status(&format!("Imported {count} EML/NWS files"));
                }
            }
            Command::Export => {
                self.app.selected_id().ok_or("Select a message first")?;
                if let Some(path) = unsafe { file_dialog(hwnd, true, "EML files\0*.eml\0\0") } {
                    self.app.export_selected(&path)?;
                    self.status("Message exported");
                }
            }
            Command::Attach => {
                if let Some(path) = unsafe { file_dialog(hwnd, false, "All files\0*.*\0\0") } {
                    self.composer()?.attach(path);
                    self.status(&format!("{} attachment(s) added", self.composer()?.count()));
                }
            }
            Command::SaveAttachments => {
                let id = self.app.selected_id().ok_or("Select a message first")?.clone();
                self.save_message_attachments(hwnd, &id)?;
            }
            Command::OpenMessage => {
                let id = self.app.selected_id().ok_or("Select a message first")?.clone();
                self.open_message(id)?;
            }
            Command::ShowPictures => {
                let document = self.preview_document.as_mut().ok_or("Select an HTML message first")?;
                document.remote_images = RemoteImagePolicy::Allow;
                if let Some(view) = &self.html_view { view.render(document.clone())?; }
                self.html_status_changed();
            }
            Command::ConfigureAccount => unsafe {
                accounts::open(hwnd, self.app.accounts().to_vec(), self.app.default_account_id().cloned(), self.account_tx.clone())?;
            },
            Command::Receive => {
                self.app.receive()?;
                if let Some(account) = self.app.active_account_id() {
                    self.startup_sync.retain(|queued| queued != account);
                }
                if let Some(account) = self.app.active_account_id().cloned() { self.sync_continuation.insert(account, SyncProgress { remaining: 49, downloaded: 0 }); }
                self.status("Synchronizing server folders and message flags…");
            }
            Command::CancelSync => {
                self.startup_sync.clear();
                self.sync_continuation.clear();
                self.app.cancel_sync();
                self.status("Stopping synchronization; completed downloads are saved…");
                self.update_actions();
            }
            Command::MarkRead | Command::MarkUnread => {
                self.app.mark_selected_read(command == Command::MarkRead)?;
                self.refresh();
                self.status("Read status saved; server update will run on the next synchronization");
            }
            Command::ToggleFlag => {
                self.app.toggle_selected_flag()?;
                self.refresh();
                self.status("Flag saved; server update will run on the next synchronization");
            }
            Command::SaveDraft | Command::Send => {
                let draft = self.composer()?.draft();
                let account = self.composer()?.sender_id();
                if command == Command::Send {
                    self.app.send_from(
                        account.as_ref().ok_or("Configure and select a sending account first")?,
                        draft,
                        self.active_composer.ok_or("Choose Create Mail first")?,
                    )?;
                    self.refresh();
                    self.status("Sending mail…");
                } else {
                    if self.app.busy() {
                        return Err("A mail operation is already running".into());
                    }
                    self.app.save_draft_for(account.as_ref(), &draft)?;
                    self.refresh();
                    self.composer()?.saved();
                    self.status("Draft saved");
                }
            }
            Command::Delete => {
                if self.selected_is_trash() {
                    return self.action(hwnd, Command::DeletePermanently);
                }
                self.app.delete_selected()?;
                if !self.app.busy() { self.clear_preview(); }
                self.refresh();
                self.status(if self.app.busy() { "Moving message to the server's Deleted Items…" } else { "Moved to Deleted Items" });
            }
            Command::Restore => {
                self.app.restore_selected()?;
                if !self.app.busy() { self.clear_preview(); }
                self.refresh();
                self.status(if self.app.busy() { "Restoring message on the server…" } else { "Message restored" });
            }
            Command::DeletePermanently => {
                if !self.selected_is_trash() {
                    return Err("Move the message to Deleted Items before permanently deleting it".into());
                }
                self.app.selected_id().ok_or("Select a message first")?;
                if self.confirm("Permanently delete this message? This cannot be undone.") {
                    self.app.permanently_delete_selected()?;
                    if !self.app.busy() { self.clear_preview(); }
                    self.refresh();
                    self.status(if self.app.busy() { "Permanently deleting message on the server…" } else { "Message permanently deleted" });
                }
            }
            Command::EmptyTrash => {
                if self.confirm("Permanently delete all messages in Deleted Items for this account or local folder? This cannot be undone.") {
                    self.app.empty_trash()?;
                    self.refresh();
                    self.status(if self.app.busy() { "Emptying Deleted Items…" } else { "Deleted Items emptied" });
                }
            }
            Command::Find => unsafe {
                SetFocus(self.controls.search);
            },
            Command::About => unsafe {
                crate::ui::about::show(hwnd);
            },
            Command::Exit => unsafe {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            },
            Command::ToggleFolders => {
                self.layout.show_folders = !self.layout.show_folders;
                self.relayout();
            }
            Command::TogglePreview => {
                self.layout.show_preview = !self.layout.show_preview;
                self.relayout();
            }
            Command::Refresh => self.refresh(),
            Command::CloseComposer => self.close_composer()?,
        }
        self.update_actions();
        Ok(())
    }
    pub(super) fn apply_account_change(
        &mut self,
        change: AccountChange,
    ) -> Result<AccountsSnapshot> {
        match change {
            AccountChange::Add { name, account } => {
                let id = self.app.add_account_named(name, account)?;
                self.app.set_scope(DataScope::Account(id.clone()))?;
                self.app.set_folder(Folder::Inbox);
                self.startup_sync.push_back(id);
            }
            AccountChange::Update { id, name, account } => {
                self.app.update_account_named(&id, name, account)?
            }
            AccountChange::Remove { id } => self.app.remove_account(&id)?,
            AccountChange::SetDefault { id } => self.app.set_default_account(&id)?,
        }
        self.rebuild_folders();
        for composer in &self.composers {
            if composer.active() {
                composer.set_accounts(self.app.accounts().to_vec(), self.app.default_account_id());
            }
        }
        self.clear_preview();
        self.refresh();
        self.status("Accounts updated");
        Ok(AccountsSnapshot {
            accounts: self.app.accounts().to_vec(),
            default_account: self.app.default_account_id().cloned(),
        })
    }
}
