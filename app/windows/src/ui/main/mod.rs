//! Main window lifecycle and native event routing.
mod actions;
mod controls;
mod dispatch;
mod folder_tree;
mod layout;
mod message_list;
mod sync;
mod toolbar;
mod view;

use self::dispatch::{WindowEvent, WindowHost, window_proc};

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
use self::{
    controls::MainWindowControls,
    layout::{DragTarget, PreviewFormat, WindowLayout},
};
use crate::ui::shared::{
    commands::{self, command_id},
    menus, message_display,
};
use crate::ui::{
    accounts::{self, AccountChange, AccountSubmission, AccountsSnapshot},
    compose, reader, settings,
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
    readers: Vec<reader::Window>,
    pending_reader_deletion: Option<PendingReaderDeletion>,
    sync_continuation: std::collections::HashMap<AccountId, SyncProgress>,
    startup_sync: std::collections::VecDeque<AccountId>,
    search_box: Option<SearchBox>,
    menu_bar: Option<MenuBar>,
    list_rows: Vec<self::message_list::Row>,
    list_tooltip: Option<Box<self::message_list::Tooltip>>,
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
        if !settings::Preferences::exists() {
            let has_accounts = state.with_window(|window| !window.app.accounts().is_empty());
            super::setup::open(hwnd, has_accounts);
        }
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
                let preferences = settings::Preferences::load();
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
                    ("Print", commands::PRINT, ToolbarIcon::Print, false),
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
                    ("Contacts", commands::CONTACTS, ToolbarIcon::Contacts, false),
                    ("Find", command_id(Command::Find), ToolbarIcon::Find, true),
                ],
            );
            self.controls.toolbar = bar;
            self.icons.push(icons);
            match SearchBox::create(search_parent, commands::SEARCH) {
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
                        commands::SEARCH,
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
                commands::FILTER,
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
                commands::FOLDER,
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
                commands::LIST,
            );
            self.list_tooltip = self::message_list::Tooltip::install(hwnd, self.controls.list);
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
}

impl MainWindow {
    fn on_event(&mut self, event: WindowEvent) {
        if matches!(event, WindowEvent::Settings) {
            unsafe {
                settings::open(self.controls.main);
            }
            return;
        }
        if matches!(event, WindowEvent::SettingsChanged) {
            let preferences = settings::Preferences::load();
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
#[path = "tests/window.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/account_navigation.rs"]
mod account_navigation_tests;

#[cfg(test)]
#[path = "tests/message_interaction.rs"]
mod message_interaction_tests;

#[cfg(test)]
#[path = "tests/server_folders.rs"]
mod server_folder_tests;

#[cfg(test)]
#[path = "tests/compact_layout.rs"]
mod compact_layout_tests;
