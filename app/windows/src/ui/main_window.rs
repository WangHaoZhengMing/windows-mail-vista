//! Main window: translate native events into application operations.
use super::dispatch::{WindowEvent, WindowHost, window_proc};

unsafe extern "system" fn sidebar_splitter_proc(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _id: usize,
    _data: usize,
) -> isize {
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    unsafe {
        match msg {
            WM_SETCURSOR => {
                SetCursor(LoadCursorW(null_mut(), IDC_SIZEWE));
                return 1;
            }
            WM_LBUTTONDOWN => {
                let parent = GetParent(hwnd);
                let mut point = POINT {
                    x: lp as i16 as i32,
                    y: (lp >> 16) as i16 as i32,
                };
                MapWindowPoints(hwnd, parent, &mut point, 1);
                SendMessageW(
                    parent,
                    msg,
                    wp,
                    ((point.x as u16 as u32) | ((point.y as u16 as u32) << 16)) as isize,
                );
                return 0;
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect);
                let brush = CreateSolidBrush(crate::platform::theme::panel(0x00eee6dd));
                FillRect(dc, &rect, brush);
                DeleteObject(brush);
                let edge = CreateSolidBrush(crate::platform::theme::panel(0x00b9a997));
                for x in [0, rect.right - 1] {
                    FillRect(
                        dc,
                        &RECT {
                            left: x,
                            top: 0,
                            right: x + 1,
                            bottom: rect.bottom,
                        },
                        edge,
                    );
                }
                DeleteObject(edge);
                EndPaint(hwnd, &paint);
                return 0;
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(sidebar_splitter_proc), 1);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}
use super::{
    account_window::{self, AccountChange, AccountSubmission, AccountsSnapshot},
    command_ids::{self, command_id},
    compose_window as compose,
    controls::MainWindowControls,
    folder_tree,
    layout::{DragTarget, PreviewFormat, WindowLayout},
    menus, reader_window,
};
use crate::{
    Result,
    account::AccountId,
    app::{
        AppState, Command, ComposerId, DataScope, DeleteAction, Folder, FolderSelection,
        LaunchAction, MailCompletion, MessageId, ReadFilter, ReplyKind, Selection,
    },
    message::Draft,
    platform::{
        classic,
        controls::{control, set_window_text as set, window_text as text},
        dialogs::file_dialog,
        dpi,
        filter::style_filter,
        html::{HtmlDocument, HtmlStatus, HtmlView, InlineImage, RemoteImagePolicy},
        icons::{self, FolderIcon, IconSet, ToolbarIcon},
        menubar::MenuBar,
        search::SearchBox,
        strings::{command_words, wide},
    },
    store::{Store, default_root},
};
use std::{
    cell::Cell,
    ptr::{null, null_mut},
    rc::Rc,
    sync::mpsc::Sender,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{
        LibraryLoader::*,
        SystemServices::{SS_ENDELLIPSIS, SS_LEFTNOWORDWRAP, SS_NOPREFIX},
        Threading::*,
    },
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{GetActiveWindow, SetFocus},
        WindowsAndMessaging::*,
    },
};

static MAIN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn edit_lines(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

struct SyncProgress {
    remaining: usize,
    downloaded: usize,
}

struct PendingReaderDeletion {
    message: MessageId,
    action: DeleteAction,
}

pub(super) struct MainWindow {
    app: AppState,
    controls: MainWindowControls,
    layout: WindowLayout,
    metrics: dpi::Resources,
    icons: Vec<HIMAGELIST>,
    updating: Rc<Cell<bool>>,
    composers: Vec<compose::Window>,
    active_composer: Option<ComposerId>,
    pending: Vec<String>,
    account_tx: Sender<AccountSubmission>,
    html_view: Option<HtmlView>,
    preview_has_html: bool,
    preview_document: Option<HtmlDocument>,
    preview_message: Option<MessageId>,
    readers: Vec<reader_window::Window>,
    pending_reader_deletion: Option<PendingReaderDeletion>,
    sync_continuation: std::collections::HashMap<AccountId, SyncProgress>,
    startup_sync: std::collections::VecDeque<AccountId>,
    search_box: Option<SearchBox>,
    menu_bar: Option<MenuBar>,
    list_rows: Vec<super::message_list::Row>,
    list_tooltip: Option<Box<super::message_list::Tooltip>>,
}

pub fn active() -> bool {
    MAIN.load(std::sync::atomic::Ordering::SeqCst) != 0
}
pub fn shutdown() {
    let h = MAIN.load(std::sync::atomic::Ordering::SeqCst);
    if h != 0 {
        unsafe {
            PostMessageW(h as HWND, WM_CLOSE, 0, 0);
        }
    }
}

pub fn run(command: &str, show: i32) -> Result<()> {
    let _dpi_context = dpi::ThreadContext::enter();
    let store = Store::open(&default_root())?;
    let state = Box::new(WindowHost::new(store));
    unsafe {
        classic::init();
        let instance = GetModuleHandleW(null());
        let names = crate::instance_names();
        let name = wide(&names.1);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_BTNFACE + 1) as HBRUSH,
            hIcon: null_mut(),
            lpszClassName: name.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err("Cannot register mail window".into());
        }
        let hwnd = CreateWindowExW(
            0,
            name.as_ptr(),
            wide("Local Folders - Windows Mail Vista").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1050,
            730,
            null_mut(),
            menus::menu(false),
            instance,
            (&*state as *const WindowHost).cast(),
        );
        if hwnd.is_null() {
            return Err(format!("Cannot create mail window: {}", GetLastError()));
        }
        let initial_dpi = dpi::window_dpi(hwnd);
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            dpi::scale(1050, initial_dpi),
            dpi::scale(730, initial_dpi),
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        MAIN.store(hwnd as usize, std::sync::atomic::Ordering::SeqCst);
        state.with_window(|window| {
            window.refresh();
            window.updating.set(false);
            window.apply(command);
            window.startup_sync = window
                .app
                .accounts()
                .iter()
                .map(|record| record.id.clone())
                .collect();
        });
        ShowWindow(hwnd, if show == 0 { SW_SHOWNORMAL } else { show });
        UpdateWindow(hwnd);
        SetTimer(hwnd, 1, 100, None);
        // The main/IPC window is ready. Release the launcher's same-thread lock.
        let lock = OpenMutexW(MUTEX_MODIFY_STATE, 0, wide(&names.0).as_ptr());
        if !lock.is_null() {
            ReleaseMutex(lock);
            CloseHandle(lock);
        }
        let mut msg = std::mem::zeroed();
        loop {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result == -1 {
                DestroyWindow(hwnd);
                MAIN.store(0, std::sync::atomic::Ordering::SeqCst);
                return Err("Windows message loop failed".into());
            }
            if result == 0 {
                break;
            }
            if state
                .with_window(|window| window.readers.iter().any(|reader| reader.translate(&msg)))
            {
                continue;
            }
            if state.with_window(|window| {
                window
                    .menu_bar
                    .as_ref()
                    .is_some_and(|bar| bar.translate(&msg))
            }) {
                continue;
            }
            let active = GetActiveWindow();
            let dialog = if active.is_null() { hwnd } else { active };
            if IsDialogMessageW(dialog, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        MAIN.store(0, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}
impl MainWindow {
    pub(super) fn new(
        store: Store,
        account_tx: Sender<AccountSubmission>,
        updating: Rc<Cell<bool>>,
    ) -> Self {
        let default_account = store.default_account_id().cloned();
        let mut app = AppState::new(store);
        let scope = default_account
            .map(DataScope::Account)
            .unwrap_or(DataScope::Local);
        if app.set_scope(scope).is_ok() && app.active_account_id().is_some() {
            app.set_folder(Folder::Inbox);
        }
        Self {
            app,
            controls: MainWindowControls::default(),
            layout: {
                let preferences = account_window::settings::Preferences::load();
                crate::platform::theme::set_dark(preferences.dark_mode);
                WindowLayout {
                    show_folders: preferences.folders,
                    show_preview: preferences.preview,
                    ..WindowLayout::default()
                }
            },
            metrics: dpi::Resources::default(),
            icons: Vec::new(),
            updating,
            composers: Vec::new(),
            active_composer: None,
            pending: Vec::new(),
            account_tx,
            html_view: None,
            preview_has_html: false,
            preview_document: None,
            preview_message: None,
            readers: Vec::new(),
            pending_reader_deletion: None,
            sync_continuation: Default::default(),
            startup_sync: Default::default(),
            search_box: None,
            menu_bar: None,
            list_rows: Vec::new(),
            list_tooltip: None,
        }
    }
    unsafe fn menu(&self) -> HMENU {
        unsafe {
            self.menu_bar
                .as_ref()
                .map_or_else(|| GetMenu(self.controls.main), MenuBar::menu)
        }
    }
    unsafe fn update_dpi(&mut self, value: u32) {
        unsafe {
            self.updating.set(true);
            let previous = self.metrics.dpi;
            self.metrics.update(self.controls.main, value);
            for label in &self.controls.labels {
                self.metrics.apply_font(*label, dpi::FontWeight::Bold);
            }
            if let Some(search) = &self.search_box {
                search.update_dpi(self.metrics.dpi);
            }
            if let Some(menu_bar) = &self.menu_bar {
                menu_bar.update_dpi(self.metrics.dpi);
            }

            let toolbar_icons = classic::resize_toolbar(self.controls.toolbar, self.metrics.dpi);
            let folder_icons = icons::images_for_dpi(IconSet::Folders, self.metrics.dpi);
            let tree_icons = icons::images(IconSet::FolderTree, dpi::scale(20, self.metrics.dpi));
            SendMessageW(
                self.controls.folder,
                TVM_SETIMAGELIST,
                TVSIL_NORMAL as usize,
                tree_icons,
            );
            SendMessageW(
                self.controls.folder,
                TVM_SETITEMHEIGHT,
                dpi::scale(20, self.metrics.dpi) as usize,
                0,
            );
            SendMessageW(
                self.controls.list,
                LVM_SETIMAGELIST,
                LVSIL_SMALL as usize,
                folder_icons,
            );
            for index in 0..6 {
                let width = SendMessageW(self.controls.list, LVM_GETCOLUMNWIDTH, index, 0) as i32;
                SendMessageW(
                    self.controls.list,
                    LVM_SETCOLUMNWIDTH,
                    index,
                    dpi::scale(dpi::logical(width, previous), self.metrics.dpi) as isize,
                );
            }
            for old in std::mem::replace(
                &mut self.icons,
                vec![toolbar_icons, folder_icons, tree_icons],
            ) {
                ImageList_Destroy(old);
            }
            self.updating.set(false);
        }
    }
    unsafe fn setup(&mut self, hwnd: HWND) {
        unsafe {
            self.controls.main = hwnd;
            let search_parent = if let Ok(menu_bar) = MenuBar::create(hwnd) {
                self.controls.menu_bar = menu_bar.hwnd();
                self.menu_bar = Some(menu_bar);
                self.controls.menu_bar
            } else {
                hwnd
            };
            let (bar, icons) = classic::toolbar(
                hwnd,
                &[
                    (
                        "Create Mail",
                        command_id(Command::NewMessage),
                        ToolbarIcon::NewMessage,
                        true,
                    ),
                    (
                        "Reply",
                        command_id(Command::Reply),
                        ToolbarIcon::Reply,
                        true,
                    ),
                    (
                        "Reply All",
                        command_id(Command::ReplyAll),
                        ToolbarIcon::ReplyAll,
                        true,
                    ),
                    (
                        "Forward",
                        command_id(Command::Forward),
                        ToolbarIcon::Forward,
                        true,
                    ),
                    ("Print", command_ids::PRINT, ToolbarIcon::Print, false),
                    (
                        "Delete",
                        command_id(Command::Delete),
                        ToolbarIcon::Delete,
                        true,
                    ),
                    (
                        "Send/Receive",
                        command_id(Command::Receive),
                        ToolbarIcon::SendReceive,
                        true,
                    ),
                    (
                        "Stop sync",
                        command_id(Command::CancelSync),
                        ToolbarIcon::StopSync,
                        false,
                    ),
                    (
                        "Contacts",
                        command_ids::CONTACTS,
                        ToolbarIcon::Contacts,
                        false,
                    ),
                    ("Find", command_id(Command::Find), ToolbarIcon::Find, true),
                ],
            );
            self.controls.toolbar = bar;
            self.icons.push(icons);
            match SearchBox::create(search_parent, command_ids::SEARCH) {
                Ok(search) => {
                    self.controls.search_host = search.hwnd();
                    self.controls.search = search.edit();
                    self.search_box = Some(search);
                }
                Err(_) => {
                    self.controls.search = control(
                        search_parent,
                        "EDIT",
                        "",
                        ES_AUTOHSCROLL as u32 | WS_TABSTOP,
                        command_ids::SEARCH,
                    );
                    self.controls.search_host = self.controls.search;
                    SendMessageW(
                        self.controls.search,
                        EM_SETCUEBANNER,
                        0,
                        wide("Search mail").as_ptr() as isize,
                    );
                }
            }
            self.controls.filter = control(
                hwnd,
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP,
                command_ids::FILTER,
            );
            style_filter(self.controls.filter);
            for label in [
                "Show All Messages",
                "Show Unread Messages",
                "Show Read Messages",
            ] {
                SendMessageW(
                    self.controls.filter,
                    CB_ADDSTRING,
                    0,
                    wide(label).as_ptr() as isize,
                );
            }
            SendMessageW(self.controls.filter, CB_SETCURSEL, 0, 0);
            self.controls.folder = control(
                hwnd,
                "SysTreeView32",
                "Folders",
                TVS_HASBUTTONS | TVS_HASLINES | TVS_LINESATROOT | TVS_SHOWSELALWAYS | WS_TABSTOP,
                command_ids::FOLDER,
            );
            let folder_icons = icons::images_for_dpi(IconSet::Folders, dpi::window_dpi(hwnd));
            self.icons.push(folder_icons);
            let tree_icons =
                icons::images(IconSet::FolderTree, dpi::scale(20, dpi::window_dpi(hwnd)));
            self.icons.push(tree_icons);
            SendMessageW(
                self.controls.folder,
                TVM_SETIMAGELIST,
                TVSIL_NORMAL as usize,
                tree_icons,
            );
            self.rebuild_folders();
            self.controls.sidebar_splitter =
                control(hwnd, "STATIC", "", 0x0100 /* SS_NOTIFY */, 2199);
            windows_sys::Win32::UI::Shell::SetWindowSubclass(
                self.controls.sidebar_splitter,
                Some(sidebar_splitter_proc),
                1,
                0,
            );
            SendMessageW(
                self.controls.folder,
                TVM_SETITEMHEIGHT,
                dpi::scale(20, dpi::window_dpi(hwnd)) as usize,
                0,
            );
            self.controls.list = control(
                hwnd,
                "SysListView32",
                "Messages",
                LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS | LVS_SHAREIMAGELISTS | WS_TABSTOP,
                command_ids::LIST,
            );
            self.list_tooltip = super::message_list::Tooltip::install(hwnd, self.controls.list);
            SendMessageW(
                self.controls.list,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                0,
                (LVS_EX_FULLROWSELECT
                    | LVS_EX_DOUBLEBUFFER
                    | LVS_EX_HEADERDRAGDROP
                    | LVS_EX_SUBITEMIMAGES) as isize,
            );
            SendMessageW(
                self.controls.list,
                LVM_SETIMAGELIST,
                LVSIL_SMALL as usize,
                folder_icons,
            );
            for (index, (label, width)) in [
                ("!", 22),
                ("📎", 22),
                ("⚐", 22),
                ("From", 140),
                ("Subject", 420),
                ("Received", 96),
            ]
            .iter()
            .enumerate()
            {
                let label = wide(label);
                let column = LVCOLUMNW {
                    mask: LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM | LVCF_FMT,
                    fmt: if index < 3 {
                        LVCFMT_CENTER
                    } else {
                        LVCFMT_LEFT
                    },
                    pszText: label.as_ptr().cast_mut(),
                    cx: *width,
                    iSubItem: index as i32,
                    ..Default::default()
                };
                SendMessageW(
                    self.controls.list,
                    LVM_INSERTCOLUMNW,
                    index,
                    &column as *const _ as isize,
                );
            }
            for label in ["From:", "To:", "Subject:"] {
                self.controls
                    .labels
                    .push(control(hwnd, "STATIC", label, SS_LEFTNOWORDWRAP, 0));
            }
            self.controls.preview_from = control(
                hwnd,
                "STATIC",
                "",
                SS_LEFTNOWORDWRAP | SS_ENDELLIPSIS | SS_NOPREFIX,
                201,
            );
            self.controls.preview_to = control(
                hwnd,
                "STATIC",
                "",
                SS_LEFTNOWORDWRAP | SS_ENDELLIPSIS | SS_NOPREFIX,
                202,
            );
            self.controls.preview_subject = control(
                hwnd,
                "STATIC",
                "",
                SS_LEFTNOWORDWRAP | SS_ENDELLIPSIS | SS_NOPREFIX,
                203,
            );
            self.controls.image_notice = control(
                hwnd,
                "STATIC",
                "External images are blocked to protect your privacy.",
                SS_LEFTNOWORDWRAP,
                0,
            );
            self.controls.show_pictures = control(
                hwnd,
                "BUTTON",
                "Show pictures",
                WS_TABSTOP | BS_PUSHBUTTON as u32,
                command_id(Command::ShowPictures),
            );
            self.controls.preview_body = control(
                hwnd,
                "EDIT",
                "",
                ES_MULTILINE as u32
                    | ES_READONLY as u32
                    | ES_AUTOVSCROLL as u32
                    | WS_VSCROLL
                    | WS_TABSTOP,
                204,
            );
            SendMessageW(
                self.controls.preview_body,
                EM_SETLIMITTEXT,
                8 * 1024 * 1024,
                0,
            );
            self.controls.status = control(hwnd, "msctls_statusbar32", "", SBARS_SIZEGRIP, 4000);
            self.update_dpi(dpi::window_dpi(hwnd));
        }
    }
    fn rebuild_folders(&mut self) {
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
    fn relayout(&self) {
        unsafe {
            let mut rect = RECT::default();
            GetClientRect(self.controls.main, &mut rect);
            self.layout
                .arrange(&self.controls, self.metrics.dpi, rect.right, rect.bottom);
            InvalidateRect(self.controls.main, null(), 1);
        }
    }
    fn status(&self, value: &str) {
        unsafe {
            SendMessageW(
                self.controls.status,
                SB_SETTEXTW,
                2,
                wide(value).as_ptr() as isize,
            );
        }
    }
    fn account_label(&self, id: &AccountId) -> String {
        self.app
            .accounts()
            .iter()
            .find(|account| &account.id == id)
            .map(|account| account.name.clone())
            .unwrap_or_else(|| "Account".into())
    }
    fn location(&self) -> String {
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
    fn clear_preview(&mut self) {
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
    fn render_body(&mut self, message: &crate::message::Message) {
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
                    HtmlView::new(self.controls.main, self.controls.main, super::HTML_STATUS)
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
    fn html_status_changed(&mut self) {
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
    fn confirm(&self, prompt: &str) -> bool {
        unsafe {
            MessageBoxW(
                self.controls.main,
                wide(prompt).as_ptr(),
                wide("Windows Mail Vista").as_ptr(),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            ) == IDYES
        }
    }
    fn selected_is_trash(&self) -> bool {
        self.app
            .selected_id()
            .and_then(|id| self.app.message_entry(id))
            .is_some_and(|entry| entry.folder == Folder::Trash.storage_key())
    }
    fn selected_pending_delete(&self) -> bool {
        self.app
            .selected_id()
            .and_then(|id| self.app.message_entry(id))
            .is_some_and(|entry| entry.pending_delete)
    }
    fn update_actions(&self) {
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
    fn refresh(&mut self) {
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
                    (3, &super::message_list::sender(&entry.summary.from)),
                    (4, &entry.summary.subject),
                    (5, &super::message_list::received(&entry.summary.date)),
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
    fn select(&mut self) -> Result<()> {
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
    fn show_message_menu(&mut self, control: HWND, mut point: POINT) {
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
            } else if let Some(command) = command_ids::decode(command as u16)
                && let Err(error) = self.action(self.controls.main, command)
            {
                self.status(&error);
            }
        }
    }
    unsafe fn message_context_menu(&self) -> HMENU {
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
    fn move_destinations(&self) -> Vec<(String, String)> {
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
    fn new_message(&mut self, draft: Draft) {
        let account = self
            .app
            .selected_account_id()
            .cloned()
            .or_else(|| self.app.active_account_id().cloned());
        self.new_message_from(draft, account);
    }
    fn new_message_from(&mut self, draft: Draft, account: Option<AccountId>) {
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
    fn open_message(&mut self, id: MessageId) -> Result<()> {
        self.readers.retain(reader_window::Window::active);
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
                self.readers.push(reader_window::Window::open(
                    self.controls.main,
                    id,
                    message,
                )?);
            }
        }
        self.refresh();
        Ok(())
    }
    fn save_message_attachments(&self, owner: HWND, id: &MessageId) -> Result<()> {
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
    fn reader_command(&mut self, reader: HWND, command: Command) -> Result<()> {
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
    fn composer(&self) -> Result<&compose::Window> {
        self.active_composer
            .and_then(|id| self.composers.get(id.index()))
            .filter(|window| window.active())
            .ok_or_else(|| "Choose Create Mail to compose a message first".into())
    }
    fn close_composer(&mut self) -> Result<()> {
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
    fn apply(&mut self, command: &str) {
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
    fn action(&mut self, hwnd: HWND, command: Command) -> Result<()> {
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
                account_window::open(hwnd, self.app.accounts().to_vec(), self.app.default_account_id().cloned(), self.account_tx.clone())?;
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
                super::about::show(hwnd);
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
                self.app.set_scope(DataScope::Account(id))?;
                self.app.set_folder(Folder::Inbox);
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
    fn poll(&mut self) {
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
    fn mail_completed(&mut self, completion: MailCompletion) {
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
                self.sync_continuation.clear();
                self.update_actions();
                self.status("Mail worker stopped unexpectedly");
            }
        }
    }
}

impl MainWindow {
    pub(super) fn on_event(&mut self, event: WindowEvent) {
        if matches!(event, WindowEvent::Settings) {
            unsafe {
                account_window::settings::open(self.controls.main);
            }
            return;
        }
        if matches!(event, WindowEvent::SettingsChanged) {
            let preferences = account_window::settings::Preferences::load();
            crate::platform::theme::set_dark(preferences.dark_mode);
            unsafe {
                crate::platform::theme::refresh();
            }
            self.layout.show_folders = preferences.folders;
            self.layout.show_preview = preferences.preview;
            self.relayout();
            return;
        }
        let hwnd = self.controls.main;
        match event {
            WindowEvent::Settings | WindowEvent::SettingsChanged => unreachable!(),
            WindowEvent::Create(hwnd) => unsafe {
                self.setup(hwnd);
                crate::platform::theme::refresh();
            },
            WindowEvent::Resize { width, height } => unsafe {
                self.layout
                    .arrange(&self.controls, self.metrics.dpi, width, height);
            },
            WindowEvent::Command { command, composer } => {
                if let Some(hwnd) = composer
                    && let Some(index) = self
                        .composers
                        .iter()
                        .position(|window| window.hwnd() == hwnd)
                {
                    self.active_composer = Some(ComposerId::new(index));
                }
                if let Err(error) = self.action(hwnd, command) {
                    self.status(&error);
                    unsafe {
                        MessageBoxW(
                            hwnd,
                            wide(&error).as_ptr(),
                            wide("Windows Mail Vista").as_ptr(),
                            MB_ICONERROR,
                        );
                    }
                }
            }
            WindowEvent::Refresh => self.refresh(),
            WindowEvent::FolderSelected(index) => {
                if let Some(item) = self.controls.tree_items.get(index) {
                    let scope = item.scope.clone();
                    let folder = item.selection.clone();
                    if let Err(error) = self.app.set_scope(scope) {
                        self.status(&error);
                    } else {
                        self.app.set_folder_selection(folder);
                        self.clear_preview();
                        self.refresh();
                    }
                }
            }
            WindowEvent::ReaderCommand { command, reader } => {
                if let Err(error) = self.reader_command(reader, command) {
                    self.status(&error);
                }
            }
            WindowEvent::MessageActivated(index) => {
                if let Some(id) = self.app.visible_message_id(index).cloned()
                    && let Err(error) = self.open_message(id)
                {
                    self.status(&error);
                }
            }
            WindowEvent::MessageSelected => {
                if let Err(error) = self.select() {
                    self.status(&error);
                }
                self.update_actions();
            }
            WindowEvent::HtmlStatusChanged => self.html_status_changed(),
            WindowEvent::ContextMenu { control, point } => self.show_message_menu(control, point),
            WindowEvent::Sort(column) => {
                self.app.sort_by(column);
                self.refresh();
            }
            WindowEvent::DpiChanged { value, suggested } => unsafe {
                self.update_dpi(value);
                dpi::suggested(hwnd, &suggested as *const RECT as isize);
                self.relayout();
            },
            WindowEvent::BeginDrag { x, y } => unsafe {
                let x = dpi::logical(x, self.metrics.dpi);
                let y = dpi::logical(y, self.metrics.dpi);
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect);
                let bottom = dpi::logical(rect.bottom, self.metrics.dpi);
                let width = dpi::logical(rect.right, self.metrics.dpi);
                let sidebar = self.layout.sidebar.clamp(120, (width / 2).max(120));
                if self.layout.show_folders && x >= sidebar && x < sidebar + 7 {
                    self.layout.drag = Some(DragTarget::Sidebar);
                } else if self.layout.show_preview
                    && (y - (bottom - 24 - self.layout.preview_height)).abs() < 7
                {
                    self.layout.drag = Some(DragTarget::Preview);
                }
                if self.layout.drag.is_some() {
                    windows_sys::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd);
                }
            },
            WindowEvent::Drag { x, y } => {
                if let Some(target) = self.layout.drag {
                    unsafe {
                        let mut rect = RECT::default();
                        GetClientRect(hwnd, &mut rect);
                        let width = dpi::logical(rect.right, self.metrics.dpi);
                        let height = dpi::logical(rect.bottom, self.metrics.dpi);
                        match target {
                            DragTarget::Sidebar => {
                                self.layout.sidebar = dpi::logical(x, self.metrics.dpi)
                                    .clamp(120, (width / 2).max(120))
                            }
                            DragTarget::Preview => {
                                self.layout.preview_height =
                                    (height - 24 - dpi::logical(y, self.metrics.dpi))
                                        .clamp(170, (height - 180).max(170))
                            }
                        }
                    }
                    self.relayout();
                }
            }
            WindowEvent::EndDrag => {
                self.layout.drag = None;
                unsafe {
                    windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                }
            }
            WindowEvent::Launch(command) => self.pending.push(command),
            WindowEvent::ProcessLaunches => {
                for command in std::mem::take(&mut self.pending) {
                    self.apply(&command);
                }
                unsafe {
                    SetForegroundWindow(hwnd);
                }
            }
            WindowEvent::Poll => self.poll(),
            WindowEvent::Close => {
                if self.app.busy() && !self.app.syncing() {
                    unsafe {
                        MessageBoxW(
                            hwnd,
                            wide("Wait for the active mail operation to finish before closing.")
                                .as_ptr(),
                            wide("Windows Mail Vista").as_ptr(),
                            MB_OK,
                        );
                    }
                    return;
                }
                self.app.cancel_sync();
                self.startup_sync.clear();
                for index in 0..self.composers.len() {
                    self.active_composer = Some(ComposerId::new(index));
                    if let Err(error) = self.close_composer() {
                        self.status(&error);
                        return;
                    }
                }
                for reader in &self.readers {
                    reader.close();
                }
                self.readers.clear();
                self.html_view = None;
                self.controls.preview_html = null_mut();
                unsafe {
                    DestroyWindow(hwnd);
                }
            }
            WindowEvent::Destroy => unsafe {
                for reader in &self.readers {
                    reader.close();
                }
                self.readers.clear();
                self.html_view = None;
                self.controls.preview_html = null_mut();
                for icons in self.icons.drain(..) {
                    ImageList_Destroy(icons);
                }
                PostQuitMessage(0);
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{Account, Incoming, Security};
    use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;

    #[test]
    fn reentrant_events_own_payloads_and_account_saves_reply_synchronously() {
        let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
        let _context = dpi::ThreadContext::enter();
        let root = std::env::temp_dir().join(format!("winmail-dispatch-{}", std::process::id()));
        let host = Box::new(WindowHost::new(Store::open(&root).unwrap()));
        unsafe {
            classic::init();
            let class = wide(&format!("WinMailDispatchTest.{}", std::process::id()));
            let instance = GetModuleHandleW(null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&wc), 0);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("Dispatch test").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                1050,
                730,
                null_mut(),
                null_mut(),
                instance,
                (&*host as *const WindowHost).cast(),
            );
            assert!(!hwnd.is_null());
            let mut account = Account {
                email: "dispatch@example.com".into(),
                username: "dispatch".into(),
                auth: Default::default(),
                password_dpapi: String::new(),
                incoming: Incoming::Imap,
                incoming_host: "localhost".into(),
                incoming_port: 993,
                incoming_security: Security::Tls,
                smtp_host: "localhost".into(),
                smtp_port: 587,
                smtp_security: Security::StartTls,
            };
            account.set_password("fixture secret").unwrap();
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            host.with_window(|window| {
                // Synchronous native reentry queues the command until this borrow ends.
                SendMessageW(
                    hwnd,
                    WM_COMMAND,
                    command_id(Command::TogglePreview) as usize,
                    0,
                );
                assert!(window.layout.show_preview);
                let mut words = wide("/mail");
                let packet = COPYDATASTRUCT {
                    dwData: 1,
                    cbData: (words.len() * 2) as u32,
                    lpData: words.as_mut_ptr().cast(),
                };
                assert_eq!(
                    SendMessageW(hwnd, WM_COPYDATA, 0, &packet as *const _ as isize),
                    1
                );
                words.fill(b'X' as u16);
                window
                    .account_tx
                    .send(AccountSubmission {
                        change: AccountChange::Add {
                            name: account.email.clone(),
                            account: account.clone(),
                        },
                        reply: reply_tx,
                    })
                    .unwrap();
                SendMessageW(hwnd, account_window::ACCOUNT_SUBMITTED, 0, 0);
                assert!(reply_rx.try_recv().unwrap().err().unwrap().contains("busy"));
            });
            host.with_window(|window| {
                assert!(!window.layout.show_preview);
                assert_eq!(window.pending, ["/mail"]);
                assert!(
                    window.app.account().is_none(),
                    "Rejected saves must never execute later"
                );
            });
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            let account_tx = host.with_window(|window| window.account_tx.clone());
            account_tx
                .send(AccountSubmission {
                    change: AccountChange::Add {
                        name: account.email.clone(),
                        account,
                    },
                    reply: reply_tx,
                })
                .unwrap();
            SendMessageW(hwnd, account_window::ACCOUNT_SUBMITTED, 0, 0);
            assert!(reply_rx.try_recv().unwrap().is_ok());
            host.with_window(|window| {
                assert_eq!(window.app.account().unwrap().email, "dispatch@example.com")
            });

            // Destruction cancels queued UI work instead of accessing dead controls.
            host.with_window(|window| {
                SendMessageW(
                    hwnd,
                    WM_COMMAND,
                    command_id(Command::TogglePreview) as usize,
                    0,
                );
                DestroyWindow(hwnd);
                assert!(!window.layout.show_preview);
            });
            host.with_window(|window| {
                assert!(!window.layout.show_preview);
                assert!(window.icons.is_empty());
            });
            assert_eq!(IsWindow(hwnd), 0);
            UnregisterClassW(class.as_ptr(), instance);
            let mut msg = MSG::default();
            // Remove the test's posted IPC and quit messages from this thread.
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {}
        }
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dpi_changes_resize_real_windows_fonts_and_images_without_losing_edits() {
        let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
        let _context = dpi::ThreadContext::enter();
        let root = std::env::temp_dir().join(format!(
            "winmail-dpi-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let host = Box::new(WindowHost::new(Store::open(&root).unwrap()));
        unsafe {
            classic::init();
            let class = wide(&format!("WinMailDpiTest.{}", std::process::id()));
            let instance = GetModuleHandleW(null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&wc), 0);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("DPI test").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                1050,
                730,
                null_mut(),
                null_mut(),
                instance,
                (&*host as *const WindowHost).cast(),
            );
            assert!(!hwnd.is_null());
            let composer = compose::Window::open(
                hwnd,
                Draft {
                    subject: "DPI 中文 fixture".into(),
                    body: "Preserved edits".into(),
                    ..Default::default()
                },
            )
            .unwrap();
            let account_tx = host.with_window(|window| window.account_tx.clone());
            let account = account_window::open(hwnd, Vec::new(), None, account_tx).unwrap();
            SendMessageW(account, WM_COMMAND, account_window::ADD_ACCOUNT as usize, 0);
            let wizard = FindWindowW(wide("WinMailRustAccountWizard").as_ptr(), null());
            assert_eq!(GetWindow(wizard, GW_OWNER), account);
            SendMessageW(wizard, WM_COMMAND, 760, 0);
            SetWindowTextW(GetDlgItem(wizard, 763), wide("dpi@example.test").as_ptr());
            SendMessageW(wizard, WM_COMMAND, 760, 0);
            let properties = FindWindowW(wide("WinMailRustAccountProperties").as_ptr(), null());
            assert_eq!(GetWindow(properties, GW_OWNER), account);
            assert_ne!(properties, account);
            let mut before = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
            // RichEdit initializes a process font cache at each new DPI. Warm
            // all sizes once, then measure repeated transitions for leaks.
            for pass in 0..3 {
                for value in [96, 120, 144, 168, 192, 288, 384, 96] {
                    for (window, width, height) in [
                        (hwnd, 1050, 730),
                        (composer.hwnd(), 800, 660),
                        (account, 800, 480),
                        (properties, 820, 600),
                    ] {
                        let rect = RECT {
                            left: 0,
                            top: 0,
                            right: dpi::scale(width, value),
                            bottom: dpi::scale(height, value),
                        };
                        SendMessageW(
                            window,
                            WM_DPICHANGED,
                            (value | (value << 16)) as usize,
                            &rect as *const _ as isize,
                        );
                    }
                    let (search, list, folder, toolbar, icons) = host.with_window(|window| {
                        (
                            window.controls.search_host,
                            window.controls.list,
                            window.controls.folder,
                            window.controls.toolbar,
                            window.icons.clone(),
                        )
                    });
                    let mut rect = RECT::default();
                    GetWindowRect(search, &mut rect);
                    assert_eq!(rect.right - rect.left, dpi::scale(150, value));
                    assert_eq!(rect.bottom - rect.top, dpi::scale(22, value));
                    assert_eq!(
                        SendMessageW(list, LVM_GETCOLUMNWIDTH, 3, 0),
                        dpi::scale(140, value) as isize
                    );
                    for (image, set) in
                        icons
                            .iter()
                            .zip([IconSet::Toolbar, IconSet::Folders, IconSet::FolderTree])
                    {
                        let (mut width, mut height) = (0, 0);
                        assert_ne!(ImageList_GetIconSize(*image, &mut width, &mut height), 0);
                        let logical_size = if set != IconSet::Folders { 20 } else { 16 };
                        assert_eq!(
                            (width, height),
                            (
                                dpi::scale(logical_size, value),
                                dpi::scale(logical_size, value)
                            )
                        );
                        assert_eq!(ImageList_GetImageCount(*image), set.count() as i32);
                    }
                    assert_eq!(icons.len(), 3);
                    // Both controls must retain their dedicated image lists after
                    // each DPI change, without retaining a stale image list.
                    for (attached, owned, set) in [
                        (
                            SendMessageW(folder, TVM_GETIMAGELIST, TVSIL_NORMAL as usize, 0),
                            icons[2],
                            IconSet::FolderTree,
                        ),
                        (
                            SendMessageW(list, LVM_GETIMAGELIST, LVSIL_SMALL as usize, 0),
                            icons[1],
                            IconSet::Folders,
                        ),
                    ] {
                        assert_eq!(attached, owned);
                        assert_eq!(ImageList_GetImageCount(attached), set.count() as i32);
                    }
                    host.with_window(|window| {
                        for node in &window.controls.tree_items {
                            let mut item = TVITEMW {
                                mask: TVIF_IMAGE | TVIF_SELECTEDIMAGE,
                                hItem: node.handle,
                                ..Default::default()
                            };
                            assert_ne!(
                                SendMessageW(folder, TVM_GETITEMW, 0, &mut item as *mut _ as isize),
                                0
                            );
                            let expected = folder_tree::tree_icon(
                                node.folder,
                                node.folder == Folder::All
                                    && matches!(
                                        node.selection,
                                        FolderSelection::Local(Folder::All)
                                    ),
                                &node.scope,
                            );
                            assert_eq!(item.iImage, expected as i32);
                            assert_eq!(item.iSelectedImage, expected as i32);
                        }
                    });
                    // Verify both actual toolbar controls use complete lists at the
                    // current DPI, rather than only checking the owned handles.
                    for bar in [toolbar, GetDlgItem(composer.hwnd(), classic::TOOLBAR)] {
                        let images = SendMessageW(bar, TB_GETIMAGELIST, 0, 0);
                        assert_ne!(images, 0);
                        assert_eq!(
                            ImageList_GetImageCount(images),
                            IconSet::Toolbar.count() as i32
                        );
                        let (mut width, mut height) = (0, 0);
                        assert_ne!(ImageList_GetIconSize(images, &mut width, &mut height), 0);
                        assert_eq!(
                            (width, height),
                            (dpi::scale(20, value), dpi::scale(20, value))
                        );
                    }
                    let font =
                        SendMessageW(GetDlgItem(composer.hwnd(), 203), WM_GETFONT, 0, 0) as HFONT;
                    let mut info = LOGFONTW::default();
                    assert_ne!(
                        GetObjectW(
                            font,
                            std::mem::size_of::<LOGFONTW>() as i32,
                            (&mut info as *mut LOGFONTW).cast()
                        ),
                        0
                    );
                    assert_eq!(info.lfHeight, -dpi::scale(12, value));
                    // RichEdit owns character formats rather than a GDI font
                    // handle. Point size must survive monitor DPI changes.
                    {
                        use windows::Win32::UI::Controls::RichEdit::{
                            CHARFORMATW, EM_GETCHARFORMAT, SCF_SELECTION,
                        };
                        let mut format = CHARFORMATW {
                            cbSize: std::mem::size_of::<CHARFORMATW>() as u32,
                            ..Default::default()
                        };
                        SendMessageW(
                            GetDlgItem(composer.hwnd(), 204),
                            EM_GETCHARFORMAT,
                            SCF_SELECTION as usize,
                            &mut format as *mut _ as isize,
                        );
                        assert_eq!(format.yHeight, 240);
                    }
                    GetWindowRect(GetDlgItem(properties, 300), &mut rect);
                    assert_eq!(rect.right - rect.left, dpi::scale(596, value));
                    assert_eq!(composer.draft().subject, "DPI 中文 fixture");
                    assert_eq!(composer.draft().body, "Preserved edits");
                    let mut icon = ICONINFO::default();
                    assert_ne!(
                        GetIconInfo(
                            SendMessageW(hwnd, WM_GETICON, ICON_BIG as usize, 0) as HICON,
                            &mut icon
                        ),
                        0
                    );
                    let mut bitmap = BITMAP::default();
                    GetObjectW(
                        icon.hbmColor,
                        std::mem::size_of::<BITMAP>() as i32,
                        (&mut bitmap as *mut BITMAP).cast(),
                    );
                    assert_eq!(bitmap.bmWidth, dpi::scale(32, value));
                    DeleteObject(icon.hbmColor);
                    DeleteObject(icon.hbmMask);
                }
                if pass == 0 {
                    before = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
                }
            }
            let after = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
            assert!(
                after <= before + 4,
                "GDI resources grew from {before} to {after}"
            );
            DestroyWindow(properties);
            DestroyWindow(account);
            composer.close();
            DestroyWindow(hwnd);
            UnregisterClassW(class.as_ptr(), instance);
            let mut msg = MSG::default();
            PeekMessageW(&mut msg, null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE);
        }
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn runtime_command_parser_preserves_quoted_url_and_file_arguments() {
        let words = super::command_words(
            r#"/mail "/mailurl:mailto:to@example.com?subject=你好 world" /eml:"C:\mail folder\你好.eml""#,
        );
        assert_eq!(
            words,
            [
                "/mail",
                "/mailurl:mailto:to@example.com?subject=你好 world",
                r"/eml:C:\mail folder\你好.eml"
            ]
        );
    }
}

#[cfg(test)]
#[path = "main_window_tests.rs"]
mod account_navigation_tests;

#[cfg(test)]
#[path = "message_interaction_tests.rs"]
mod message_interaction_tests;

#[cfg(test)]
#[path = "server_folder_tests.rs"]
mod server_folder_tests;

#[cfg(test)]
#[path = "compact_layout_tests.rs"]
mod compact_layout_tests;
