//! Folder tree, message list, preview, and action presentation.
use super::*;

impl MainWindow {
    pub(super) fn rebuild_folders(&mut self) {
        let suppressed = self.updating.replace(true);
        unsafe {
            self.controls.tree_items = folder_tree::rebuild(
                self.controls.folder,
                self.app.accounts(),
                self.app.scope(),
                self.app.folder_selection(),
                &self
                    .app
                    .accounts()
                    .iter()
                    .map(|account| {
                        (
                            account.id.clone(),
                            self.app
                                .server_folders(&account.id)
                                .into_iter()
                                .cloned()
                                .collect(),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        }
        self.updating.set(suppressed);
    }
    pub(super) fn relayout(&self) {
        unsafe {
            let mut rect = RECT::default();
            GetClientRect(self.controls.main, &mut rect);
            self.layout
                .arrange(&self.controls, self.metrics.dpi, rect.right, rect.bottom);
            InvalidateRect(self.controls.main, null(), 1);
        }
    }
    pub(super) fn status(&self, value: &str) {
        unsafe {
            SendMessageW(
                self.controls.status,
                SB_SETTEXTW,
                2,
                wide(value).as_ptr() as isize,
            );
        }
    }
    pub(super) fn account_label(&self, id: &AccountId) -> String {
        self.app
            .accounts()
            .iter()
            .find(|account| &account.id == id)
            .map(|account| account.name.clone())
            .unwrap_or_else(|| "Account".into())
    }
    pub(super) fn location(&self) -> String {
        let name = match self.app.scope() {
            DataScope::Account(id) => self.account_label(id),
            DataScope::Local => "Local Folders".into(),
            DataScope::All => "All Accounts".into(),
        };
        if self.app.folder_selection() == &FolderSelection::Local(Folder::All) {
            name
        } else {
            format!("{} - {name}", self.app.folder_title())
        }
    }
    pub(super) fn clear_preview(&mut self) {
        self.preview_has_html = false;
        self.preview_document = None;
        self.preview_message = None;
        self.layout.blocked_images = false;
        self.layout.preview_format = PreviewFormat::PlainText;
        if let Some(view) = &self.html_view {
            let _ = view.set_content("", Vec::new());
        }
        unsafe {
            for control in [
                self.controls.preview_from,
                self.controls.preview_to,
                self.controls.preview_subject,
                self.controls.preview_body,
            ] {
                set(control, "");
            }
        }
        self.relayout();
    }
    pub(super) fn render_body(&mut self, message: &crate::message::Message) {
        unsafe {
            set(self.controls.preview_body, &edit_lines(&message.body));
        }
        let current = self.app.selected_id().cloned();
        let image_policy = if self.preview_message == current {
            self.preview_document
                .as_ref()
                .map_or(RemoteImagePolicy::Block, |document| document.remote_images)
        } else {
            RemoteImagePolicy::Block
        };
        self.preview_message = current;
        self.preview_document = None;
        self.preview_has_html = message.html.is_some();
        self.layout.blocked_images = false;
        self.layout.preview_format = PreviewFormat::PlainText;
        if let Some(html) = &message.html {
            if self.html_view.is_none() {
                match unsafe {
                    HtmlView::new(
                        self.controls.main,
                        self.controls.main,
                        crate::ui::shared::commands::HTML_STATUS,
                    )
                } {
                    Ok(view) => {
                        self.controls.preview_html = view.hwnd();
                        unsafe {
                            SetWindowLongPtrW(view.hwnd(), GWLP_ID, 205);
                        }
                        self.html_view = Some(view);
                    }
                    Err(error) => {
                        self.status(&format!(
                            "HTML view unavailable; showing plain text: {error}"
                        ));
                        self.relayout();
                        return;
                    }
                }
            }
            let images = message
                .inline_images()
                .filter_map(|attachment| {
                    attachment
                        .content_id
                        .as_ref()
                        .map(|content_id| InlineImage {
                            content_id: content_id.clone(),
                            content_type: attachment.content_type.clone(),
                            bytes: attachment.bytes.clone(),
                        })
                })
                .collect();
            let document = HtmlDocument {
                html: html.clone(),
                inline_images: images,
                remote_images: image_policy,
            };
            self.preview_document = Some(document.clone());
            if let Some(view) = &self.html_view
                && let Err(error) = view.render(document)
            {
                self.status(&format!("Cannot display HTML; showing plain text: {error}"));
            }
        } else if let Some(view) = &self.html_view {
            let _ = view.set_content("", Vec::new());
        }
        self.html_status_changed();
    }
    pub(super) fn html_status_changed(&mut self) {
        self.layout.blocked_images = self.preview_has_html
            && self
                .html_view
                .as_ref()
                .is_some_and(HtmlView::has_blocked_images);
        self.layout.preview_format = match self.html_view.as_ref().map(HtmlView::status) {
            Some(HtmlStatus::Loading) if self.preview_has_html => {
                self.status("Loading message…");
                PreviewFormat::Loading
            }
            Some(HtmlStatus::Ready) if self.preview_has_html => {
                if self.layout.preview_format == PreviewFormat::Loading {
                    self.status("Ready");
                }
                PreviewFormat::Html
            }
            Some(HtmlStatus::Failed(error)) if self.preview_has_html => {
                self.status(&format!(
                    "HTML view unavailable; showing plain text: {error}"
                ));
                PreviewFormat::PlainText
            }
            _ => PreviewFormat::PlainText,
        };
        self.relayout();
    }
    pub(super) fn confirm(&self, prompt: &str) -> bool {
        unsafe {
            MessageBoxW(
                self.controls.main,
                wide(prompt).as_ptr(),
                wide("Windows Mail Vista").as_ptr(),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            ) == IDYES
        }
    }
    pub(super) fn selected_is_trash(&self) -> bool {
        self.app
            .selected_id()
            .and_then(|id| self.app.message_entry(id))
            .is_some_and(|entry| entry.folder == Folder::Trash.storage_key())
    }
    pub(super) fn selected_pending_delete(&self) -> bool {
        self.app
            .selected_id()
            .and_then(|id| self.app.message_entry(id))
            .is_some_and(|entry| entry.pending_delete)
    }
    pub(super) fn update_actions(&self) {
        let selected = self.app.selected_id().is_some();
        let idle = !self.app.busy();
        unsafe {
            let menu = self.menu();
            for (command, enabled) in [
                (Command::OpenMessage, selected),
                (Command::SaveAttachments, selected),
                (
                    Command::Delete,
                    selected && idle && !self.selected_pending_delete(),
                ),
                (
                    Command::Restore,
                    selected && idle && self.selected_is_trash(),
                ),
                (
                    Command::DeletePermanently,
                    selected && idle && self.selected_is_trash() && !self.selected_pending_delete(),
                ),
                (Command::EmptyTrash, idle),
                (
                    Command::Receive,
                    self.app.can_start_sync()
                        && self
                            .app
                            .active_account_id()
                            .is_some_and(|account| !self.app.account_syncing(account)),
                ),
                (
                    Command::CancelSync,
                    self.app.syncing() || !self.startup_sync.is_empty(),
                ),
                (Command::Reply, selected),
                (Command::ReplyAll, selected),
                (Command::Forward, selected),
                (Command::MarkRead, selected),
                (Command::MarkUnread, selected),
                (Command::ToggleFlag, selected),
            ] {
                let id = command_id(command);
                EnableMenuItem(
                    menu,
                    u32::from(id),
                    MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED },
                );
                SendMessageW(
                    self.controls.toolbar,
                    TB_ENABLEBUTTON,
                    id as usize,
                    enabled as isize,
                );
            }
            let activity = if self.app.syncing() {
                format!("Sync: {}", self.app.sync_count())
            } else if self.app.busy() {
                "Working…".into()
            } else {
                "Ready".into()
            };
            SendMessageW(
                self.controls.status,
                SB_SETTEXTW,
                1,
                wide(&activity).as_ptr() as isize,
            );
        }
    }
    pub(super) fn refresh(&mut self) {
        unsafe {
            self.updating.set(true);
            let selected = self.app.selected_id().map(|id| id.as_str().to_owned());
            let filter = match SendMessageW(self.controls.filter, CB_GETCURSEL, 0, 0) {
                1 => ReadFilter::Unread,
                2 => ReadFilter::Read,
                _ => ReadFilter::All,
            };
            let entries = self.app.list_messages(filter, &text(self.controls.search));
            if let Some(tooltip) = &self.list_tooltip {
                tooltip.hide();
            }
            self.list_rows = entries
                .iter()
                .map(|entry| {
                    let mut row = super::message_list::Row::new(
                        &entry.summary.from,
                        &entry.summary.date,
                        entry.read,
                    );
                    row.priority = entry.summary.priority;
                    row.attachment = entry.summary.has_attachments.unwrap_or(false);
                    row.flagged = entry.flags.flagged;
                    row
                })
                .collect();
            if self.app.selected_id().is_none() {
                self.clear_preview();
            }
            SendMessageW(self.controls.list, WM_SETREDRAW, 0, 0);
            SendMessageW(self.controls.list, LVM_DELETEALLITEMS, 0, 0);
            for (index, entry) in entries.iter().enumerate() {
                let empty = wide("");
                let item = LVITEMW {
                    mask: LVIF_TEXT | LVIF_IMAGE,
                    iItem: index as i32,
                    pszText: empty.as_ptr().cast_mut(),
                    iImage: -2, // I_IMAGENONE: the envelope belongs to From.
                    ..Default::default()
                };
                SendMessageW(
                    self.controls.list,
                    LVM_INSERTITEMW,
                    0,
                    &item as *const _ as isize,
                );
                for (column, value) in [
                    (
                        2,
                        &if entry.flags.flagged {
                            "⚑".to_owned()
                        } else {
                            String::new()
                        },
                    ),
                    (3, &message_display::sender(&entry.summary.from)),
                    (4, &entry.summary.subject),
                    (5, &message_display::received(&entry.summary.date)),
                ] {
                    let value = wide(value);
                    let item = LVITEMW {
                        iSubItem: column,
                        pszText: value.as_ptr().cast_mut(),
                        ..Default::default()
                    };
                    SendMessageW(
                        self.controls.list,
                        LVM_SETITEMTEXTW,
                        index,
                        &item as *const _ as isize,
                    );
                }
                let image = LVITEMW {
                    mask: LVIF_IMAGE,
                    iItem: index as i32,
                    iSubItem: 3,
                    iImage: if entry.read {
                        FolderIcon::ReadMessage
                    } else {
                        FolderIcon::UnreadMessage
                    } as i32,
                    ..Default::default()
                };
                SendMessageW(
                    self.controls.list,
                    LVM_SETITEMW,
                    0,
                    &image as *const _ as isize,
                );
                if selected.as_ref() == Some(&entry.id) {
                    let state = LVITEMW {
                        state: LVIS_SELECTED | LVIS_FOCUSED,
                        stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                        ..Default::default()
                    };
                    SendMessageW(
                        self.controls.list,
                        LVM_SETITEMSTATE,
                        index,
                        &state as *const _ as isize,
                    );
                }
            }
            SendMessageW(self.controls.list, WM_SETREDRAW, 1, 0);
            InvalidateRect(self.controls.list, null(), 1);
            let unread = entries.iter().filter(|e| !e.read).count();
            SendMessageW(
                self.controls.status,
                SB_SETTEXTW,
                0,
                wide(&format!("{} message(s), {unread} unread", entries.len())).as_ptr() as isize,
            );
            let location = self.location();
            SetWindowTextW(
                self.controls.main,
                wide(&format!("{location} - Windows Mail Vista")).as_ptr(),
            );

            self.updating.set(false);
            self.update_actions();
        }
    }
    pub(super) fn list_notification(&mut self, lp: isize) -> Option<isize> {
        unsafe {
            if lp == 0 {
                return None;
            }
            let header = (lp as *const NMHDR).read_unaligned();
            if header.hwndFrom == self.controls.list && header.code == NM_CUSTOMDRAW {
                let draw = (lp as *const NMLVCUSTOMDRAW).read_unaligned();
                match draw.nmcd.dwDrawStage {
                    CDDS_PREPAINT => return Some(CDRF_NOTIFYITEMDRAW as isize),
                    CDDS_ITEMPREPAINT => return Some(CDRF_NOTIFYSUBITEMDRAW as isize),
                    stage if stage == CDDS_ITEMPREPAINT | CDDS_SUBITEM => {
                        let unread = self
                            .list_rows
                            .get(draw.nmcd.dwItemSpec)
                            .is_some_and(|row| row.unread);
                        if (0..3).contains(&draw.iSubItem) {
                            if let Some(row) = self.list_rows.get(draw.nmcd.dwItemSpec) {
                                super::message_list::draw_status(self.controls.list, &draw, row);
                                return Some(CDRF_SKIPDEFAULT as isize);
                            }
                        }
                        let weight = if unread && matches!(draw.iSubItem, 3 | 4) {
                            dpi::FontWeight::Bold
                        } else {
                            dpi::FontWeight::Regular
                        };
                        let font = self.metrics.font(weight);
                        if !font.is_null() {
                            SelectObject(draw.nmcd.hdc, font);
                        }
                        return Some(CDRF_NEWFONT as isize);
                    }
                    _ => return Some(CDRF_DODEFAULT as isize),
                }
            }
            if self
                .list_tooltip
                .as_ref()
                .is_some_and(|tooltip| tooltip.window == header.hwndFrom)
                && header.code == TTN_GETDISPINFOW
            {
                static EMPTY: [u16; 1] = [0];
                let tip = super::message_list::hovered(self.controls.list)
                    .and_then(|(index, column)| {
                        let row = self.list_rows.get(index)?;
                        match column {
                            3 => Some(row.from_tip.as_ptr()),
                            5 => Some(row.date_tip.as_ptr()),
                            _ => None,
                        }
                    })
                    .unwrap_or(EMPTY.as_ptr());
                let notification = lp as *mut NMTTDISPINFOW;
                std::ptr::addr_of_mut!((*notification).lpszText).write_unaligned(tip.cast_mut());
                return Some(0);
            }
            None
        }
    }
    pub(super) fn select(&mut self) -> Result<()> {
        let index = unsafe {
            SendMessageW(
                self.controls.list,
                LVM_GETNEXTITEM,
                usize::MAX,
                LVNI_SELECTED as isize,
            )
        };
        if index < 0 {
            return Ok(());
        }
        match self.app.select_message(index as usize)? {
            Selection::Draft(draft) => {
                self.clear_preview();
                unsafe {
                    set(self.controls.preview_to, &draft.to);
                    set(self.controls.preview_subject, &draft.subject);
                    set(self.controls.preview_body, &edit_lines(&draft.body));
                }
                self.status("Double-click or press Enter to edit this draft");
            }
            Selection::Message(parsed) => {
                if let Some(row) = self.list_rows.get_mut(index as usize) {
                    row.unread = false;
                }
                unsafe {
                    set(self.controls.preview_from, &parsed.summary.from);
                    set(self.controls.preview_to, &parsed.summary.to);
                    set(self.controls.preview_subject, &parsed.summary.subject);
                    let item = LVITEMW {
                        mask: LVIF_IMAGE,
                        iItem: index as i32,
                        iSubItem: 3,
                        iImage: FolderIcon::ReadMessage as i32,
                        ..Default::default()
                    };
                    SendMessageW(
                        self.controls.list,
                        LVM_SETITEMW,
                        0,
                        &item as *const _ as isize,
                    );
                    InvalidateRect(self.controls.list, null(), 0);
                }
                if self.selected_pending_delete() {
                    self.status(
                        "Marked for deletion; awaiting server cleanup. Restore is available.",
                    );
                } else {
                    self.status(&format!(
                        "{} · {} attachment(s)",
                        parsed.summary.date,
                        parsed.downloadable_attachments().count()
                    ));
                }
                self.render_body(&parsed);
            }
        }
        self.update_actions();
        Ok(())
    }
    pub(super) fn show_message_menu(&mut self, control: HWND, mut point: POINT) {
        if control != self.controls.list {
            return;
        }
        unsafe {
            if point.x == -1 && point.y == -1 {
                let mut rect = RECT::default();
                GetWindowRect(control, &mut rect);
                point = POINT {
                    x: rect.left + dpi::scale(30, self.metrics.dpi),
                    y: rect.top + dpi::scale(30, self.metrics.dpi),
                };
            } else {
                let mut hit = LVHITTESTINFO {
                    pt: point,
                    ..Default::default()
                };
                ScreenToClient(control, &mut hit.pt);
                let index =
                    SendMessageW(control, LVM_SUBITEMHITTEST, 0, &mut hit as *mut _ as isize);
                if index < 0 {
                    return;
                }
                let suppressed = self.updating.replace(true);
                let state = LVITEMW {
                    state: LVIS_SELECTED | LVIS_FOCUSED,
                    stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                    ..Default::default()
                };
                SendMessageW(
                    control,
                    LVM_SETITEMSTATE,
                    index as usize,
                    &state as *const _ as isize,
                );
                self.updating.set(suppressed);
                if let Err(error) = self.select() {
                    self.status(&error);
                    return;
                }
            }
            let popup = self.message_context_menu();
            let command = TrackPopupMenu(
                popup,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                self.controls.main,
                null(),
            );
            DestroyMenu(popup);
            if (6000..10000).contains(&command) {
                if let Some((mailbox, _)) = self
                    .move_destinations()
                    .get((command - 6000) as usize)
                    .cloned()
                {
                    match self.app.move_selected_to(&mailbox) {
                        Ok(()) => self.status("Moving message on the server…"),
                        Err(error) => self.status(&error),
                    }
                    self.update_actions();
                }
            } else if let Some(command) = commands::decode(command as u16)
                && let Err(error) = self.action(self.controls.main, command)
            {
                self.status(&error);
            }
        }
    }
    pub(super) unsafe fn message_context_menu(&self) -> HMENU {
        unsafe {
            let popup = CreatePopupMenu();
            let selected = self.app.selected_id().is_some() && !self.app.busy();
            let trash = self.selected_is_trash();
            for (label, command, enabled) in [
                ("Open", Command::OpenMessage, selected),
                ("Reply", Command::Reply, selected),
                ("Reply All", Command::ReplyAll, selected),
                ("Forward", Command::Forward, selected),
                ("Mark as Read", Command::MarkRead, selected),
                ("Mark as Unread", Command::MarkUnread, selected),
                ("Toggle Flag", Command::ToggleFlag, selected),
                (
                    if trash {
                        "Delete Permanently…"
                    } else {
                        "Move to Deleted Items"
                    },
                    Command::Delete,
                    selected && !self.selected_pending_delete(),
                ),
                ("Restore", Command::Restore, selected && trash),
                ("Save Attachments…", Command::SaveAttachments, selected),
                ("Export Message…", Command::Export, selected),
            ] {
                AppendMenuW(
                    popup,
                    MF_STRING | if enabled { MF_ENABLED } else { MF_GRAYED },
                    command_id(command) as usize,
                    wide(label).as_ptr(),
                );
            }
            let destinations = self.move_destinations();
            if !destinations.is_empty() {
                let submenu = CreatePopupMenu();
                for (index, (_, caption)) in destinations.iter().enumerate() {
                    AppendMenuW(submenu, MF_STRING, 6000 + index, wide(caption).as_ptr());
                }
                AppendMenuW(
                    popup,
                    MF_POPUP | if selected { MF_ENABLED } else { MF_GRAYED },
                    submenu as usize,
                    wide("Move to Folder").as_ptr(),
                );
            }
            popup
        }
    }
    pub(super) fn move_destinations(&self) -> Vec<(String, String)> {
        let Some(entry) = self
            .app
            .selected_id()
            .and_then(|id| self.app.message_entry(id))
        else {
            return Vec::new();
        };
        let Some(owner) = &entry.account_id else {
            return Vec::new();
        };
        let Some(remote) = &entry.remote else {
            return Vec::new();
        };
        if entry.remote_problem.is_some() {
            return Vec::new();
        }
        self.app
            .server_folders(owner)
            .into_iter()
            .filter(|folder| {
                folder.selectable
                    && !mail_core::network::mailbox_eq(&folder.mailbox, &remote.mailbox)
            })
            .take(4000)
            .map(|folder| (folder.mailbox.clone(), folder.display_name.clone()))
            .collect()
    }
}
