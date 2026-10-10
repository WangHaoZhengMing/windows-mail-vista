//! Real server mailbox hierarchy, with a local group for imported/outgoing mail.
use crate::{
    account::{AccountId, AccountRecord},
    app::{DataScope, Folder, FolderSelection},
    platform::{icons::TreeIcon, strings::wide},
};
use mail_core::network::{ImapFolder, ImapFolderRole};
use std::collections::HashMap;
use windows_sys::Win32::{
    Foundation::HWND, UI::Controls::*, UI::WindowsAndMessaging::SendMessageW,
};

pub(super) struct TreeItem {
    pub handle: HTREEITEM,
    pub scope: DataScope,
    pub folder: Folder,
    pub selection: FolderSelection,
}
const FOLDERS: [Folder; 7] = [
    Folder::Inbox,
    Folder::Outbox,
    Folder::Sent,
    Folder::Drafts,
    Folder::Trash,
    Folder::Junk,
    Folder::Recovered,
];

pub(super) unsafe fn rebuild(
    tree: HWND,
    accounts: &[AccountRecord],
    scope: &DataScope,
    selection: &FolderSelection,
    folders: &[(AccountId, Vec<ImapFolder>)],
) -> Vec<TreeItem> {
    unsafe {
        SendMessageW(tree, TVM_DELETEITEM, 0, TVI_ROOT);
        let mut items = Vec::new();
        for account in accounts {
            let label = if account.name == account.account.email {
                account.name.clone()
            } else {
                format!("{} ({})", account.name, account.account.email)
            };
            let account_scope = DataScope::Account(account.id.clone());
            let parent = add_item(
                tree,
                &mut items,
                TVI_ROOT,
                &label,
                account_scope.clone(),
                Folder::All,
                FolderSelection::Local(Folder::All),
            );
            let server = folders
                .iter()
                .find(|(id, _)| id == &account.id)
                .map(|(_, folders)| folders.as_slice())
                .unwrap_or_default();
            if server.is_empty() {
                add_local_folders(tree, &mut items, parent, &account_scope);
            } else {
                add_server_folders(tree, &mut items, parent, &account_scope, server);
                for folder in [Folder::Outbox, Folder::Recovered] {
                    add_item(
                        tree,
                        &mut items,
                        parent,
                        folder.title(),
                        account_scope.clone(),
                        folder,
                        FolderSelection::Local(folder),
                    );
                }
                for folder in [Folder::Sent, Folder::Drafts, Folder::Trash] {
                    if !server
                        .iter()
                        .any(|mailbox| role_folder(mailbox.role) == folder)
                    {
                        add_item(
                            tree,
                            &mut items,
                            parent,
                            folder.title(),
                            account_scope.clone(),
                            folder,
                            FolderSelection::Local(folder),
                        );
                    }
                }
            }
            SendMessageW(tree, TVM_EXPAND, TVE_EXPAND as usize, parent);
        }
        let parent = add_item(
            tree,
            &mut items,
            TVI_ROOT,
            "Local Folders",
            DataScope::Local,
            Folder::All,
            FolderSelection::Local(Folder::All),
        );
        add_local_folders(tree, &mut items, parent, &DataScope::Local);
        SendMessageW(tree, TVM_EXPAND, TVE_EXPAND as usize, parent);
        let selected = items.iter().find(|item| &item.scope == scope && selections_equal(&item.selection, selection))
            .or_else(|| items.iter().find(|item| &item.scope == scope && matches!(selection, FolderSelection::Local(folder) if *folder == item.folder)));
        if let Some(item) = selected {
            SendMessageW(tree, TVM_SELECTITEM, TVGN_CARET as usize, item.handle);
            SendMessageW(tree, TVM_ENSUREVISIBLE, 0, item.handle);
        }
        items
    }
}
fn selections_equal(left: &FolderSelection, right: &FolderSelection) -> bool {
    match (left, right) {
        (FolderSelection::ServerMailbox(left), FolderSelection::ServerMailbox(right)) => {
            mail_core::network::mailbox_eq(left, right)
        }
        _ => left == right,
    }
}
fn role_folder(role: Option<ImapFolderRole>) -> Folder {
    match role {
        Some(ImapFolderRole::Inbox) => Folder::Inbox,
        Some(ImapFolderRole::Sent) => Folder::Sent,
        Some(ImapFolderRole::Drafts) => Folder::Drafts,
        Some(ImapFolderRole::Trash) => Folder::Trash,
        Some(ImapFolderRole::Junk) => Folder::Junk,
        _ => Folder::All,
    }
}
unsafe fn add_local_folders(
    tree: HWND,
    items: &mut Vec<TreeItem>,
    parent: HTREEITEM,
    scope: &DataScope,
) {
    for folder in FOLDERS {
        unsafe {
            add_item(
                tree,
                items,
                parent,
                folder.title(),
                scope.clone(),
                folder,
                FolderSelection::Local(folder),
            );
        }
    }
}
unsafe fn add_server_folders(
    tree: HWND,
    items: &mut Vec<TreeItem>,
    root: HTREEITEM,
    scope: &DataScope,
    folders: &[ImapFolder],
) {
    let mut sorted: Vec<_> = folders.iter().collect();
    sorted.sort_by_key(|folder| {
        (
            folder
                .delimiter
                .map_or(0, |delimiter| folder.mailbox.matches(delimiter).count()),
            match folder.role {
                Some(ImapFolderRole::Inbox) => 0,
                Some(ImapFolderRole::Sent) => 1,
                Some(ImapFolderRole::Drafts) => 2,
                Some(ImapFolderRole::Trash) => 3,
                _ => 4,
            },
            folder.display_name.to_lowercase(),
        )
    });
    let mut parents = HashMap::new();
    for folder in sorted {
        let parent = folder
            .delimiter
            .and_then(|delimiter| folder.mailbox.rsplit_once(delimiter))
            .and_then(|(name, _)| parents.get(name).copied())
            .unwrap_or(root);
        let caption = folder
            .delimiter
            .and_then(|delimiter| folder.display_name.rsplit_once(delimiter))
            .map_or(folder.display_name.as_str(), |(_, name)| name);
        let handle = unsafe {
            add_item(
                tree,
                items,
                parent,
                caption,
                scope.clone(),
                role_folder(folder.role),
                FolderSelection::ServerMailbox(folder.mailbox.clone()),
            )
        };
        parents.insert(folder.mailbox.clone(), handle);
        unsafe {
            SendMessageW(tree, TVM_EXPAND, TVE_EXPAND as usize, parent);
        }
    }
}
unsafe fn add_item(
    tree: HWND,
    items: &mut Vec<TreeItem>,
    parent: HTREEITEM,
    label: &str,
    scope: DataScope,
    folder: Folder,
    selection: FolderSelection,
) -> HTREEITEM {
    unsafe {
        let title = wide(label);
        let image = tree_icon(folder, parent == TVI_ROOT, &scope) as i32;
        let item = TVITEMW {
            mask: TVIF_TEXT | TVIF_PARAM | TVIF_IMAGE | TVIF_SELECTEDIMAGE,
            pszText: title.as_ptr().cast_mut(),
            lParam: items.len() as isize,
            iImage: image,
            iSelectedImage: image,
            ..Default::default()
        };
        let insert = TVINSERTSTRUCTW {
            hParent: parent,
            hInsertAfter: TVI_LAST,
            Anonymous: TVINSERTSTRUCTW_0 { item },
        };
        let handle = SendMessageW(tree, TVM_INSERTITEMW, 0, &insert as *const _ as isize);
        items.push(TreeItem {
            handle,
            scope,
            folder,
            selection,
        });
        handle
    }
}

/// Select a named asset from the folder's role.
pub(super) fn tree_icon(folder: Folder, root: bool, scope: &DataScope) -> TreeIcon {
    if root {
        return if matches!(scope, DataScope::Local) {
            TreeIcon::Local
        } else {
            TreeIcon::Account
        };
    }
    match folder {
        Folder::Inbox => TreeIcon::Inbox,
        Folder::Outbox => TreeIcon::Outbox,
        Folder::Sent => TreeIcon::Sent,
        Folder::Drafts => TreeIcon::Drafts,
        Folder::Trash => TreeIcon::Trash,
        Folder::Junk => TreeIcon::Junk,
        Folder::All | Folder::Recovered => TreeIcon::Archive,
    }
}
