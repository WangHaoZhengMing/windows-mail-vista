use super::mail::WorkerResult;
use mail_core::{
    Result,
    account::{AccountId, AccountRecord},
    message::{self, Draft},
    store::{Entry, Store},
};
use std::sync::mpsc::Receiver;

/// Persistent message identity; distinct from native row indices and composers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MessageId(String);

impl MessageId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn from_stored(value: String) -> Self {
        Self(value)
    }
}

/// Stable slot in the UI's composer collection, never a native window handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComposerId(usize);

impl ComposerId {
    pub fn new(index: usize) -> Self {
        Self(index)
    }

    pub fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Folder {
    #[default]
    All,
    Inbox,
    Outbox,
    Sent,
    Trash,
    Drafts,
    Junk,
    Recovered,
}

/// A local logical folder or one exact server mailbox (wire name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderSelection {
    Local(Folder),
    ServerMailbox(String),
}

impl Folder {
    /// Storage keys preserve the existing catalog format.
    pub fn storage_key(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Inbox => "Inbox",
            Self::Outbox => "Outbox",
            Self::Sent => "Sent",
            Self::Trash => "Trash",
            Self::Drafts => "Drafts",
            Self::Junk => "Junk",
            Self::Recovered => "Recovered",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::All => "Local Folders",
            Self::Sent => "Sent Items",
            Self::Trash => "Deleted Items",
            other => other.storage_key(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ReadFilter {
    #[default]
    All,
    Unread,
    Read,
}

impl ReadFilter {
    fn matches(self, entry: &Entry) -> bool {
        match self {
            Self::All => true,
            Self::Unread => !entry.read,
            Self::Read => entry.read,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Priority,
    Attachment,
    Flag,
    From,
    Subject,
    Received,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    fn reversed(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortSpec {
    pub column: SortColumn,
    pub direction: SortDirection,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            column: SortColumn::Received,
            direction: SortDirection::Descending,
        }
    }
}

pub enum Selection {
    Draft(Draft),
    Message(message::Message),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataScope {
    All,
    Local,
    Account(AccountId),
}

impl DataScope {
    pub(super) fn matches(&self, entry: &Entry) -> bool {
        match self {
            Self::All => true,
            Self::Local => entry.account_id.is_none(),
            Self::Account(id) => entry.account_id.as_ref() == Some(id),
        }
    }
}

/// Owns durable mail state and the currently displayed message identities.
/// Window handles, layout, and edit controls belong to the UI adapter instead.
pub struct AppState {
    pub(super) store: Store,
    folder: FolderSelection,
    scope: DataScope,
    pub(super) delete_policy: super::mail::DeletePolicy,
    sort: SortSpec,
    selected: Option<MessageId>,
    visible: Vec<MessageId>,
    pub(super) worker: Option<Receiver<WorkerResult>>,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        let account = store.default_account_id().cloned();
        let mut app = Self {
            store,
            folder: FolderSelection::Local(if account.is_some() {
                Folder::Inbox
            } else {
                Folder::All
            }),
            scope: account.map(DataScope::Account).unwrap_or(DataScope::Local),
            delete_policy: super::mail::DeletePolicy::default(),
            sort: SortSpec::default(),
            selected: None,
            visible: Vec::new(),
            worker: None,
        };
        app.reconcile_server_selection();
        app
    }

    pub fn scope(&self) -> &DataScope {
        &self.scope
    }

    pub fn set_scope(&mut self, scope: DataScope) -> Result<()> {
        if let DataScope::Account(id) = &scope
            && self.store.account_record(id).is_none()
        {
            return Err("Account not found".into());
        }
        self.scope = scope;
        self.selected = None;
        self.visible.clear();
        Ok(())
    }

    pub fn selected_account(&self) -> Option<&AccountRecord> {
        self.active_account_id()
            .and_then(|id| self.store.account_record(id))
    }

    /// Account used for a new composer or receive operation.
    pub fn active_account_id(&self) -> Option<&AccountId> {
        match &self.scope {
            DataScope::Account(id) => Some(id),
            DataScope::All | DataScope::Local => self.store.default_account_id(),
        }
    }

    /// A selected local message deliberately has no owning account.
    pub fn selected_account_id(&self) -> Option<&AccountId> {
        if let Some(id) = self.selected_id() {
            self.store
                .entry(id.as_str())
                .and_then(|entry| entry.account_id.as_ref())
        } else {
            self.active_account_id()
        }
    }

    pub fn has_local_messages(&self) -> bool {
        self.store
            .entries()
            .iter()
            .any(|entry| entry.account_id.is_none())
    }

    pub fn folder(&self) -> Folder {
        match self.folder {
            FolderSelection::Local(folder) => folder,
            FolderSelection::ServerMailbox(_) => Folder::All,
        }
    }

    pub fn folder_selection(&self) -> &FolderSelection {
        &self.folder
    }
    pub fn set_folder(&mut self, folder: Folder) {
        self.set_folder_selection(FolderSelection::Local(folder));
        self.reconcile_server_selection();
    }
    pub fn set_folder_selection(&mut self, folder: FolderSelection) {
        self.folder = folder;
        self.selected = None;
        self.visible.clear();
    }
    pub(super) fn reconcile_server_selection(&mut self) {
        let DataScope::Account(owner) = &self.scope else {
            return;
        };
        let folders = self.store.server_folders(owner);
        match &self.folder {
            FolderSelection::ServerMailbox(mailbox)
                if !folders
                    .iter()
                    .any(|folder| mail_core::network::mailbox_eq(&folder.mailbox, mailbox)) =>
            {
                self.folder = FolderSelection::Local(Folder::Recovered);
                self.selected = None;
                self.visible.clear();
            }
            FolderSelection::ServerMailbox(mailbox) => {
                if let Some(folder) = folders
                    .into_iter()
                    .find(|folder| mail_core::network::mailbox_eq(&folder.mailbox, mailbox))
                {
                    self.folder = FolderSelection::ServerMailbox(folder.mailbox.clone());
                }
            }
            FolderSelection::Local(local) if *local != Folder::All => {
                if let Some(folder) = folders.into_iter().find(|folder| {
                    folder.role.is_some() && folder.local_folder_key() == local.storage_key()
                }) {
                    self.folder = FolderSelection::ServerMailbox(folder.mailbox.clone());
                }
            }
            _ => {}
        }
    }
    pub fn server_folders(&self, id: &AccountId) -> Vec<&mail_core::network::ImapFolder> {
        self.store.server_folders(id)
    }
    pub fn folder_title(&self) -> String {
        match &self.folder {
            FolderSelection::Local(folder) => folder.title().into(),
            FolderSelection::ServerMailbox(mailbox) => self
                .active_account_id()
                .and_then(|id| {
                    self.store
                        .server_folders(id)
                        .into_iter()
                        .find(|folder| mail_core::network::mailbox_eq(&folder.mailbox, mailbox))
                })
                .map_or_else(|| mailbox.clone(), |folder| folder.display_name.clone()),
        }
    }

    /// Current ordering, available to UI backends without duplicating sort state.
    pub fn sort(&self) -> SortSpec {
        self.sort
    }

    pub fn sort_by(&mut self, column: SortColumn) {
        self.sort = SortSpec {
            column,
            direction: if column == self.sort.column {
                self.sort.direction.reversed()
            } else {
                SortDirection::Ascending
            },
        };
    }

    pub fn selected_id(&self) -> Option<&MessageId> {
        self.selected.as_ref()
    }

    pub fn list_messages(&mut self, filter: ReadFilter, query: &str) -> Vec<Entry> {
        let query = query.to_lowercase();
        let local_role_key = match &self.folder {
            FolderSelection::ServerMailbox(mailbox) => self
                .active_account_id()
                .and_then(|account| {
                    self.store
                        .server_folders(account)
                        .into_iter()
                        .find(|folder| mail_core::network::mailbox_eq(&folder.mailbox, mailbox))
                })
                .filter(|folder| folder.role.is_some())
                .map(|folder| folder.local_folder_key()),
            _ => None,
        };
        let mut entries: Vec<_> = self
            .store
            .entries()
            .iter()
            .filter(|entry| self.scope.matches(entry))
            .filter(|entry| match &self.folder {
                FolderSelection::Local(folder) => {
                    *folder == Folder::All || entry.folder == folder.storage_key()
                }
                FolderSelection::ServerMailbox(mailbox) => {
                    entry.remote.as_ref().is_some_and(|remote| {
                        mail_core::network::mailbox_eq(&remote.mailbox, mailbox)
                    }) || (entry.remote.is_none()
                        && local_role_key
                            .as_ref()
                            .is_some_and(|key| key == &entry.folder))
                }
            })
            .filter(|entry| filter.matches(entry))
            .filter(|entry| {
                query.is_empty()
                    || format!("{} {}", entry.summary.from, entry.summary.subject)
                        .to_lowercase()
                        .contains(&query)
            })
            .cloned()
            .collect();
        entries.sort_by(|left, right| {
            let order = match self.sort.column {
                SortColumn::From => left
                    .summary
                    .from
                    .to_lowercase()
                    .cmp(&right.summary.from.to_lowercase()),
                SortColumn::Subject => left
                    .summary
                    .subject
                    .to_lowercase()
                    .cmp(&right.summary.subject.to_lowercase()),
                SortColumn::Flag => left.flags.flagged.cmp(&right.flags.flagged),
                // These columns have no stored values yet. Preserve the classic
                // UI's date fallback until their behavior is implemented.
                SortColumn::Priority | SortColumn::Attachment | SortColumn::Received => {
                    mailparse::dateparse(&left.summary.date)
                        .unwrap_or(0)
                        .cmp(&mailparse::dateparse(&right.summary.date).unwrap_or(0))
                }
            };
            match self.sort.direction {
                SortDirection::Ascending => order,
                SortDirection::Descending => order.reverse(),
            }
        });
        // Undated imports historically appear newest-imported first.
        if self.sort.column == SortColumn::Received
            && entries.iter().all(|entry| entry.summary.date.is_empty())
        {
            entries.reverse();
        }
        self.visible = entries
            .iter()
            .map(|entry| MessageId::from_stored(entry.id.clone()))
            .collect();
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !self.visible.contains(id))
        {
            self.selected = None;
        }
        entries
    }

    pub fn visible_message_id(&self, index: usize) -> Option<&MessageId> {
        self.visible.get(index)
    }

    pub fn message_entry(&self, id: &MessageId) -> Option<&Entry> {
        self.store.entry(id.as_str())
    }

    pub fn message(&self, id: &MessageId) -> Result<message::Message> {
        message::parse(&self.store.raw(id.as_str())?)
    }

    /// Reading a separate window does not change the main window's selection.
    pub fn read_message(&mut self, id: &MessageId) -> Result<Selection> {
        let entry = self.store.entry(id.as_str()).ok_or("Message not found")?;
        if entry.folder == Folder::Drafts.storage_key()
            && let Some(draft) = self.store.draft(id.as_str()).cloned()
        {
            return Ok(Selection::Draft(draft));
        }
        let parsed = self.message(id)?;
        self.store.mark_read(id.as_str())?;
        Ok(Selection::Message(parsed))
    }

    pub fn select_message(&mut self, index: usize) -> Result<Selection> {
        let id = self
            .visible_message_id(index)
            .ok_or("Selection no longer exists")?
            .clone();
        let message = self.read_message(&id)?;
        self.selected = Some(id);
        Ok(message)
    }

    pub fn selected_message(&self) -> Result<message::Message> {
        let id = self.selected_id().ok_or("Select a message first")?;
        message::parse(&self.store.raw(id.as_str())?)
    }
}
