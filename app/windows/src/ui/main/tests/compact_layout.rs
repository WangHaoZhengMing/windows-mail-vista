use super::*;
use crate::account::{Account, Incoming, Security};
use std::{
    mem::size_of,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::System::SystemServices::SS_TYPEMASK;

struct Fixture {
    host: Option<Box<WindowHost>>,
    hwnd: HWND,
    instance: HMODULE,
    class: Vec<u16>,
    root: PathBuf,
    first: AccountId,
    second: AccountId,
    long_message: String,
}

// The production minimum follows the physical monitor's DPI. Let this hidden
// fixture simulate every DPI at fixed logical client sizes on any monitor;
// all remaining events use the production dispatcher and native controls.
unsafe extern "system" fn fixture_window_proc(
    hwnd: HWND,
    message: u32,
    wp: usize,
    lp: isize,
) -> isize {
    unsafe {
        if message == WM_GETMINMAXINFO {
            let information = &mut *(lp as *mut MINMAXINFO);
            information.ptMinTrackSize = POINT { x: 1, y: 1 };
            information.ptMaxTrackSize = POINT { x: 32767, y: 32767 };
            return 0;
        }
        window_proc(hwnd, message, wp, lp)
    }
}

impl Fixture {
    fn new(name: &str) -> Self {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "winmail-compact-ui-{name}-{}-{serial}",
            std::process::id()
        ));
        let mut store = Store::open(&root).unwrap();
        let configuration = Account {
            email: "first@example.test".into(),
            username: "first@example.test".into(),
            auth: Default::default(),
            password_dpapi: String::new(),
            incoming: Incoming::Pop3,
            incoming_host: "pop.example.test".into(),
            incoming_port: 995,
            incoming_security: Security::Tls,
            smtp_host: "smtp.example.test".into(),
            smtp_port: 587,
            smtp_security: Security::StartTls,
        };
        let first = store
            .add_account_named("First account".into(), configuration.clone())
            .unwrap();
        let mut second_configuration = configuration;
        second_configuration.email = "second@example.test".into();
        second_configuration.username = "second@example.test".into();
        let second = store
            .add_account_named("Second account".into(), second_configuration)
            .unwrap();
        let long_raw = format!(
            "From: {} <sender@example.test>\r\nTo: {} <recipient@example.test>\r\nSubject: {}\r\n\r\nPlain body for compact header\r\n",
            "Long sender display name ".repeat(16),
            "Long recipient display name ".repeat(16),
            "Needle subject with long words ".repeat(18),
        );
        let long_message = store
            .import_for(Some(&first), long_raw.as_bytes(), "Inbox", None)
            .unwrap();
        let read = store
            .import_for(Some(&first), &raw("Already read"), "Inbox", None)
            .unwrap();
        store.mark_read(&read).unwrap();
        store
            .import_for(Some(&first), &raw("Short unread"), "Inbox", None)
            .unwrap();
        store
            .import_for(Some(&second), &raw("Foreign account only"), "Inbox", None)
            .unwrap();
        let host = Box::new(WindowHost::new(store));
        let class = wide(&format!(
            "WinMailCompactLayout.{name}.{}.{serial}",
            std::process::id()
        ));
        unsafe {
            classic::init();
            let instance = GetModuleHandleW(null());
            let registration = WNDCLASSW {
                lpfnWndProc: Some(fixture_window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                hbrBackground: (COLOR_BTNFACE + 1) as HBRUSH,
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&registration), 0);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("Compact layout fixture").as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                0,
                0,
                1050,
                730,
                null_mut(),
                menus::menu(false),
                instance,
                (&*host as *const WindowHost).cast(),
            );
            assert!(!hwnd.is_null());
            host.with_window(|window| window.refresh());
            Self {
                host: Some(host),
                hwnd,
                instance,
                class,
                root,
                first,
                second,
                long_message,
            }
        }
    }
    fn host(&self) -> &WindowHost {
        self.host.as_ref().unwrap()
    }
    fn resize(&self, width: i32, height: i32, dpi_value: u32) {
        unsafe {
            self.host()
                .with_window(|window| window.update_dpi(dpi_value));
            let mut outer = RECT::default();
            let mut client = RECT::default();
            // A hidden window recalculates its frame on its first resize, and
            // the native menu can wrap at minimum width. Measure the final
            // client area rather than assuming a fixed non-client border.
            for _ in 0..3 {
                assert_ne!(GetWindowRect(self.hwnd, &mut outer), 0);
                assert_ne!(GetClientRect(self.hwnd, &mut client), 0);
                let desired_width =
                    outer.right - outer.left + dpi::scale(width, dpi_value) - client.right;
                let desired_height =
                    outer.bottom - outer.top + dpi::scale(height, dpi_value) - client.bottom;
                assert_ne!(
                    SetWindowPos(
                        self.hwnd,
                        null_mut(),
                        0,
                        0,
                        desired_width,
                        desired_height,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    ),
                    0
                );
                assert_ne!(GetClientRect(self.hwnd, &mut client), 0);
                if client.right == dpi::scale(width, dpi_value)
                    && client.bottom == dpi::scale(height, dpi_value)
                {
                    break;
                }
            }
            self.host().with_window(|window| window.relayout());
            assert_ne!(GetClientRect(self.hwnd, &mut client), 0);
            assert_eq!(dpi::logical(client.right, dpi_value), width);
            assert_eq!(dpi::logical(client.bottom, dpi_value), height);
        }
    }
    fn select_long_message(&self) {
        let (list, index) = self.host().with_window(|window| {
            let count = unsafe { SendMessageW(window.controls.list, LVM_GETITEMCOUNT, 0, 0) };
            let index = (0..count as usize)
                .find(|&index| {
                    window
                        .app
                        .visible_message_id(index)
                        .is_some_and(|id| id.as_str() == self.long_message)
                })
                .unwrap();
            (window.controls.list, index)
        });
        let selected = LVITEMW {
            state: LVIS_SELECTED | LVIS_FOCUSED,
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        unsafe {
            assert_ne!(
                SendMessageW(
                    list,
                    LVM_SETITEMSTATE,
                    index,
                    &selected as *const _ as isize
                ),
                0
            );
        }
        self.host().with_window(|window| {
            assert_eq!(
                window.app.selected_id().unwrap().as_str(),
                self.long_message
            );
        });
    }
    fn switch_account(&self, owner: &AccountId) {
        let (tree, item) = self.host().with_window(|window| {
            let item = window
                .controls
                .tree_items
                .iter()
                .find(|node| {
                    node.scope == DataScope::Account(owner.clone())
                        && node.selection == FolderSelection::Local(Folder::Inbox)
                })
                .unwrap();
            (window.controls.folder, item.handle)
        });
        unsafe {
            assert_ne!(
                SendMessageW(tree, TVM_SELECTITEM, TVGN_CARET as usize, item),
                0
            );
        }
    }
    fn search(&self, query: &str) {
        let edit = self.host().with_window(|window| window.controls.search);
        unsafe {
            assert_ne!(SetWindowTextW(edit, wide(query).as_ptr()), 0);
        }
    }
    fn filter(&self, index: usize) {
        let filter = self.host().with_window(|window| window.controls.filter);
        unsafe {
            assert_ne!(
                SendMessageW(filter, CB_SETCURSEL, index, 0),
                CB_ERR as isize
            );
            SendMessageW(
                self.hwnd,
                WM_COMMAND,
                commands::FILTER as usize | ((CBN_SELCHANGE as usize) << 16),
                filter as isize,
            );
        }
    }
    fn count(&self) -> isize {
        self.host().with_window(|window| unsafe {
            SendMessageW(window.controls.list, LVM_GETITEMCOUNT, 0, 0)
        })
    }
    fn drag_sidebar_to(&self, logical_x: i32, dpi_value: u32) {
        let splitter = self
            .host()
            .with_window(|window| window.controls.sidebar_splitter);
        let packed = |x: i32, y: i32| {
            (dpi::scale(x, dpi_value) as u16 as usize
                | ((dpi::scale(y, dpi_value) as u16 as usize) << 16)) as isize
        };
        unsafe {
            SendMessageW(splitter, WM_LBUTTONDOWN, 0, packed(3, 100));
            SendMessageW(self.hwnd, WM_MOUSEMOVE, 0, packed(logical_x, 200));
            SendMessageW(self.hwnd, WM_LBUTTONUP, 0, packed(logical_x, 200));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
            UnregisterClassW(self.class.as_ptr(), self.instance);
            let mut message = MSG::default();
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {}
        }
        drop(self.host.take());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn raw(subject: &str) -> Vec<u8> {
    format!("From: sender@example.test\r\nTo: recipient@example.test\r\nSubject: {subject}\r\n\r\nCached plain body\r\n").into_bytes()
}
fn rect(window: HWND, main: HWND, dpi_value: u32) -> RECT {
    unsafe {
        let mut bounds = RECT::default();
        let mut origin = POINT::default();
        assert_ne!(GetWindowRect(window, &mut bounds), 0);
        assert_ne!(ClientToScreen(main, &mut origin), 0);
        RECT {
            left: dpi::logical(bounds.left - origin.x, dpi_value),
            top: dpi::logical(bounds.top - origin.y, dpi_value),
            right: dpi::logical(bounds.right - origin.x, dpi_value),
            bottom: dpi::logical(bounds.bottom - origin.y, dpi_value),
        }
    }
}
fn font_weight(window: HWND) -> i32 {
    unsafe {
        let font = SendMessageW(window, WM_GETFONT, 0, 0) as HFONT;
        assert!(!font.is_null());
        let mut information = LOGFONTW::default();
        assert_eq!(
            GetObjectW(
                font,
                size_of::<LOGFONTW>() as i32,
                (&mut information as *mut LOGFONTW).cast(),
            ),
            size_of::<LOGFONTW>() as i32
        );
        information.lfWeight
    }
}
fn assert_compact_geometry(window: &MainWindow, dpi_value: u32, width: i32) {
    let controls = &window.controls;
    let bounds = |control| rect(control, controls.main, dpi_value);
    let menu_bar = bounds(controls.menu_bar);
    let toolbar = bounds(controls.toolbar);
    let folder = bounds(controls.folder);
    let list = bounds(controls.list);
    assert_eq!(menu_bar.top, 0);
    assert_eq!(menu_bar.bottom, 26);
    assert_eq!(toolbar.top, menu_bar.bottom);
    assert_eq!(toolbar.bottom, 58);
    assert_eq!(folder.top, toolbar.bottom);
    assert_eq!(list.top, toolbar.bottom);
    let search = bounds(controls.search_host);
    let filter = bounds(controls.filter);
    assert!(search.top >= menu_bar.top && search.bottom <= menu_bar.bottom);
    assert!(filter.top >= toolbar.top && filter.bottom <= toolbar.bottom);
    assert!(search.left >= 0 && search.right <= width);
    assert!(filter.left >= 0 && filter.right <= width);
    assert_eq!(search.right - search.left, 150);
    assert_eq!(search.bottom - search.top, 22);
    assert_eq!(search.top, 2);
    unsafe {
        assert_eq!(GetParent(controls.search_host), controls.menu_bar);
        assert_eq!(GetParent(controls.filter), controls.main);
        assert!(
            GetMenu(controls.main).is_null(),
            "the native menu is hosted in the client menu row"
        );
        assert!(!window.menu().is_null());
    }
    let edit = bounds(controls.search);
    assert!(edit.left >= search.left && edit.right <= search.right);
    assert!(edit.top >= search.top && edit.bottom <= search.bottom);
    unsafe {
        let count = SendMessageW(controls.toolbar, TB_BUTTONCOUNT, 0, 0);
        assert_eq!(count, 10, "collapsing captions must retain every command");
        let mut last = RECT::default();
        assert_ne!(
            SendMessageW(
                controls.toolbar,
                TB_GETITEMRECT,
                (count - 1) as usize,
                &mut last as *mut _ as isize,
            ),
            0
        );
        assert!(
            dpi::logical(last.right, dpi_value) <= filter.left - 8,
            "toolbar buttons overlap the filter input"
        );
        for index in 0..count {
            let mut button = TBBUTTON::default();
            assert_ne!(
                SendMessageW(
                    controls.toolbar,
                    TB_GETBUTTON,
                    index as usize,
                    &mut button as *mut _ as isize,
                ),
                0
            );
            if width >= 1200 {
                assert_ne!(
                    button.fsStyle & BTNS_SHOWTEXT as u8,
                    0,
                    "wide resize must restore caption for command {} / DPI {dpi_value}",
                    button.idCommand,
                );
            }
            if button.idCommand == i32::from(command_id(Command::NewMessage)) {
                assert_eq!(button.iBitmap, ToolbarIcon::NewMessage as i32);
                assert_ne!(button.fsState & TBSTATE_ENABLED as u8, 0);
            }
        }
        assert_ne!(
            GetWindowLongPtrW(controls.toolbar, GWL_STYLE) as u32 & TBSTYLE_TOOLTIPS,
            0
        );
        let mut caption = [0u16; 64];
        let length = SendMessageW(
            controls.toolbar,
            TB_GETBUTTONTEXTW,
            command_id(Command::Find) as usize,
            caption.as_mut_ptr() as isize,
        );
        assert_eq!(
            String::from_utf16_lossy(&caption[..length as usize]),
            "Find"
        );
    }
    let values = [
        controls.preview_from,
        controls.preview_to,
        controls.preview_subject,
    ];
    assert_eq!(controls.labels.len(), 3);
    let header_top = bounds(controls.labels[0]).top;
    for (&label, value) in controls.labels.iter().zip(values) {
        let label_bounds = bounds(label);
        let value_bounds = bounds(value);
        assert_eq!(label_bounds.top, header_top);
        assert_eq!(value_bounds.top, header_top);
        assert!(label_bounds.right <= value_bounds.left);
        assert!(value_bounds.right <= list.right);
        assert!(value_bounds.right > value_bounds.left);
        assert!(font_weight(label) >= FW_BOLD as i32);
        assert!(font_weight(value) <= FW_MEDIUM as i32);
        unsafe {
            let style = GetWindowLongPtrW(value, GWL_STYLE) as u32;
            assert_eq!(style & SS_TYPEMASK, SS_LEFTNOWORDWRAP);
            assert_ne!(style & SS_ENDELLIPSIS, 0);
        }
    }
    let body = bounds(controls.preview_body);
    assert!(
        (32..=44).contains(&(body.top - list.bottom)),
        "compact body gap {}",
        body.top - list.bottom
    );
    for &label in &controls.labels {
        assert!(bounds(label).bottom <= body.top);
    }
    assert!(body.right <= width);
}

#[test]
fn compact_toolbar_and_single_line_headers_survive_minimum_width_drag_and_dpi_changes() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let fixture = Fixture::new("geometry");
    unsafe {
        let mut minimum = MINMAXINFO::default();
        window_proc(
            fixture.hwnd,
            WM_GETMINMAXINFO,
            0,
            &mut minimum as *mut _ as isize,
        );
        let native_dpi = dpi::window_dpi(fixture.hwnd);
        assert_eq!(minimum.ptMinTrackSize.x, dpi::scale(650, native_dpi));
        assert_eq!(minimum.ptMinTrackSize.y, dpi::scale(420, native_dpi));
    }
    fixture.select_long_message();
    for dpi_value in [96, 144, 192, 384] {
        for (width, height) in [(1050, 730), (650, 420), (1200, 730)] {
            fixture.resize(width, height, dpi_value);
            fixture.host().with_window(|window| {
                assert_compact_geometry(window, dpi_value, width);
                for control in [
                    window.controls.preview_from,
                    window.controls.preview_to,
                    window.controls.preview_subject,
                ] {
                    assert!(
                        unsafe { text(control).len() > 250 },
                        "native control must retain complete long header text"
                    );
                    let visible = rect(control, fixture.hwnd, dpi_value);
                    assert!(
                        visible.bottom - visible.top <= 24,
                        "long header must remain one row"
                    );
                }
            });
            fixture.drag_sidebar_to(width / 2, dpi_value);
            fixture.host().with_window(|window| {
                assert_eq!(window.layout.sidebar, width / 2);
                assert_compact_geometry(window, dpi_value, width);
            });
        }
    }
}

#[test]
fn toolbar_search_forwarding_and_read_filters_keep_account_scope() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let fixture = Fixture::new("filter");
    fixture.resize(1050, 730, 144);
    assert_eq!(fixture.count(), 3);
    fixture.search("Needle");
    assert_eq!(
        fixture.count(),
        1,
        "nested search edit must forward EN_CHANGE"
    );
    fixture.filter(1);
    assert_eq!(fixture.count(), 1);
    fixture.filter(2);
    assert_eq!(fixture.count(), 0);
    fixture.search("");
    assert_eq!(fixture.count(), 1);
    fixture.filter(0);
    assert_eq!(fixture.count(), 3);
    fixture.host().with_window(|window| {
        assert_eq!(
            window.app.scope(),
            &DataScope::Account(fixture.first.clone())
        );
        assert_eq!(unsafe { text(window.controls.labels[0]) }, "From:");
        assert_eq!(unsafe { text(window.controls.labels[1]) }, "To:");
        assert_eq!(unsafe { text(window.controls.labels[2]) }, "Subject:");
    });
    fixture.switch_account(&fixture.second);
    assert_eq!(fixture.count(), 1);
    fixture.search("Needle");
    assert_eq!(fixture.count(), 0);
    fixture.search("Foreign");
    assert_eq!(fixture.count(), 1);
    fixture.host().with_window(|window| {
        assert_eq!(
            window.app.scope(),
            &DataScope::Account(fixture.second.clone())
        );
        assert!(!window.app.busy());
    });
}

#[test]
fn external_image_notice_uses_one_reserved_row_above_the_body_at_each_dpi() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let fixture = Fixture::new("images");
    fixture.select_long_message();
    for dpi_value in [96, 144, 192, 384] {
        for (width, height) in [(1050, 730), (650, 420)] {
            fixture.resize(width, height, dpi_value);
            fixture.host().with_window(|window| unsafe {
                window.layout.blocked_images = false;
                window.relayout();
                let baseline = rect(window.controls.preview_body, fixture.hwnd, dpi_value);
                window.layout.blocked_images = true;
                window.relayout();
                let body = rect(window.controls.preview_body, fixture.hwnd, dpi_value);
                let notice = rect(window.controls.image_notice, fixture.hwnd, dpi_value);
                let pictures = rect(window.controls.show_pictures, fixture.hwnd, dpi_value);
                let list = rect(window.controls.list, fixture.hwnd, dpi_value);
                assert!((28..=38).contains(&(body.top - baseline.top)));
                assert!(notice.bottom <= body.top);
                assert!(pictures.bottom <= body.top);
                assert!(notice.right <= pictures.left);
                assert!(notice.top >= baseline.top - 2);
                assert!(pictures.right <= list.right);
                assert_ne!(
                    GetWindowLongPtrW(window.controls.image_notice, GWL_STYLE) as u32 & WS_VISIBLE,
                    0
                );
                assert_ne!(
                    GetWindowLongPtrW(window.controls.show_pictures, GWL_STYLE) as u32 & WS_VISIBLE,
                    0
                );
                assert_eq!(text(window.controls.show_pictures), "Show pictures");
                window.layout.blocked_images = false;
                window.relayout();
                assert_eq!(
                    GetWindowLongPtrW(window.controls.image_notice, GWL_STYLE) as u32 & WS_VISIBLE,
                    0
                );
                assert_eq!(
                    GetWindowLongPtrW(window.controls.show_pictures, GWL_STYLE) as u32 & WS_VISIBLE,
                    0
                );
                assert_eq!(
                    rect(window.controls.preview_body, fixture.hwnd, dpi_value).top,
                    baseline.top
                );
            });
        }
    }
}

#[test]
fn visible_toolbar_inputs_receive_native_hits_after_the_toolbar_repaints() {
    let _lock = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    let fixture = Fixture::new("visible-inputs");
    fixture.resize(1050, 730, 96);
    unsafe {
        ShowWindow(fixture.hwnd, SW_SHOWNA);
        SetWindowPos(
            fixture.hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        UpdateWindow(fixture.hwnd);
        RedrawWindow(
            fixture.hwnd,
            null(),
            null_mut(),
            RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_ERASE | RDW_UPDATENOW,
        );
    }
    fixture.host().with_window(|window| unsafe {
        InvalidateRect(window.controls.toolbar, null(), 1);
        UpdateWindow(window.controls.toolbar);
        for input in [window.controls.search_host, window.controls.filter] {
            assert_ne!(IsWindowVisible(input), 0);
            let bounds = rect(input, fixture.hwnd, 96);
            let mut center = POINT {
                x: (bounds.left + bounds.right) / 2,
                y: (bounds.top + bounds.bottom) / 2,
            };
            ClientToScreen(fixture.hwnd, &mut center);
            let hit = WindowFromPoint(center);
            let mut class = [0u16; 80];
            let length = GetClassNameW(hit, class.as_mut_ptr(), class.len() as i32);
            assert!(
                hit == input || IsChild(input, hit) != 0,
                "visible input is covered by {} (native sibling id {})",
                String::from_utf16_lossy(&class[..length as usize]),
                GetDlgCtrlID(hit),
            );
            let dc = GetDC(input);
            assert!(!dc.is_null());
            let mut drawable = RECT::default();
            let region = GetClipBox(dc, &mut drawable);
            ReleaseDC(input, dc);
            assert_ne!(
                region, NULLREGION,
                "visible input has no drawable paint region"
            );
        }
    });
}
