//! Application state and mail operations, independent of the window toolkit.
//!
//! UI and compatibility adapters translate their inputs into these types. This
//! layer owns the mail store and background work; it never owns Windows handles.

mod command;
mod mail;
mod movement;
mod state;
mod sync;

pub use command::{Command, LaunchAction, ReplyKind};
pub use mail::{DeleteAction, DeletePolicy, MailCompletion};
pub use mail_core::account::{AccountId, AccountRecord};
pub use state::{
    AppState, ComposerId, DataScope, Folder, FolderSelection, MessageId, ReadFilter, Selection,
    SortColumn, SortDirection, SortSpec,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod sync_tests;
