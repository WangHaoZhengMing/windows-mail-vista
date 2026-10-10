use super::*;
use crate::{
    app::{AppState, ReadFilter},
    platform::controls::window_text,
    store::Store,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "winmail-reader-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        )))
    }
    fn message(&self, raw: &[u8]) -> (MessageId, Message) {
        let mut store = Store::open(&self.0).unwrap();
        store.import(raw, "Inbox", None).unwrap();
        let mut app = AppState::new(store);
        app.list_messages(ReadFilter::All, "");
        app.select_message(0).unwrap();
        (
            app.selected_id().unwrap().clone(),
            app.selected_message().unwrap(),
        )
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Owner {
    hwnd: HWND,
    commands: Box<RefCell<Vec<(usize, isize)>>>,
}
impl Owner {
    fn new() -> Self {
        unsafe {
            let commands = Box::new(RefCell::new(Vec::new()));
            let class = wide("WinMailRust.ReaderTestOwner");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(owner_proc),
                hInstance: GetModuleHandleW(null()),
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("").as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                100,
                100,
                null_mut(),
                null_mut(),
                wc.hInstance,
                (&*commands as *const RefCell<Vec<(usize, isize)>>).cast(),
            );
            assert!(!hwnd.is_null());
            Self { hwnd, commands }
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.hwnd) };
    }
}

unsafe extern "system" fn owner_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if msg == WM_NCCREATE {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (*(lp as *const CREATESTRUCTW)).lpCreateParams as _,
            );
        }
        if msg == READER_DISPATCH {
            let pointer =
                GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<Vec<(usize, isize)>>;
            (*pointer).borrow_mut().push((wp, lp));
            return 0;
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn pump() {
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
            if message.message != WM_QUIT {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

#[test]
fn reader_is_read_only_routes_its_identity_and_scales_at_high_dpi() {
    let _guard = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _dpi = dpi::ThreadContext::enter();
    unsafe { classic::init() };
    let temp = Temp::new();
    let (id, message) = temp.message(b"Subject: Independent reader\r\nFrom: sender@example.com\r\nTo: me@example.com\r\nDate: Sat, 10 Oct 2026 10:00:00 +0800\r\n\r\nfirst line\r\nsecond line");
    let owner = Owner::new();
    let window = Window::create(owner.hwnd, id.clone(), message, false).unwrap();
    let hwnd = window.hwnd();
    assert_eq!(window.id(), &id);
    unsafe {
        assert_eq!(window_text(hwnd), "Independent reader - Windows Mail Vista");
        assert_eq!(
            window_text(GetDlgItem(hwnd, SUBJECT as i32)),
            "Independent reader"
        );
        assert_eq!(
            window_text(GetDlgItem(hwnd, FROM as i32)),
            "sender@example.com"
        );
        assert_eq!(window_text(GetDlgItem(hwnd, TO as i32)), "me@example.com");
        let body = GetDlgItem(hwnd, BODY as i32);
        assert_eq!(window_text(body), "first line\r\nsecond line");
        assert_eq!(
            GetWindowLongW(GetDlgItem(hwnd, TO as i32), GWL_STYLE) as u32 & WS_VISIBLE,
            0
        );
        SendMessageW(hwnd, WM_COMMAND, SHOW_DETAILS as usize, 0);
        assert_ne!(
            GetWindowLongW(GetDlgItem(hwnd, TO as i32), GWL_STYLE) as u32 & WS_VISIBLE,
            0
        );
        SendMessageW(hwnd, WM_COMMAND, SHOW_DETAILS as usize, 0);
        SendMessageW(hwnd, WM_COMMAND, COPY_BODY as usize, 0);
        assert_eq!(
            window_text(window.state.controls.status.get()),
            "Message text copied"
        );
        {
            use windows_sys::Win32::System::{DataExchange::*, Memory::*, Ole::CF_UNICODETEXT};
            assert_ne!(OpenClipboard(hwnd), 0);
            let memory = GetClipboardData(CF_UNICODETEXT as u32);
            assert!(!memory.is_null());
            let pointer = GlobalLock(memory) as *const u16;
            assert!(!pointer.is_null());
            let expected = wide("first line\r\nsecond line");
            let actual = std::slice::from_raw_parts(pointer, expected.len()).to_vec();
            GlobalUnlock(memory);
            CloseClipboard();
            assert_eq!(actual, expected);
        }

        assert_ne!(
            GetWindowLongW(body, GWL_STYLE) as u32 & ES_READONLY as u32,
            0
        );
        SendMessageW(hwnd, WM_COMMAND, command_id(Command::Reply) as usize, 0);
        SendMessageW(
            hwnd,
            WM_COMMAND,
            command_id(Command::SaveAttachments) as usize,
            0,
        );
        // Commands are queued: the reader can return from its own callback
        // before the owner creates a composer or removes a reader window.
        assert!(owner.commands.borrow().is_empty());
        pump();
        assert_eq!(
            &*owner.commands.borrow(),
            &[
                (command_id(Command::Reply) as usize, hwnd as isize),
                (command_id(Command::SaveAttachments) as usize, hwnd as isize)
            ]
        );
        let suggested = RECT {
            left: 10,
            top: 10,
            right: 1810,
            bottom: 1450,
        };
        SendMessageW(
            hwnd,
            WM_DPICHANGED,
            192 | (192 << 16),
            &suggested as *const _ as isize,
        );
        assert_eq!(window.state.metrics.borrow().dpi, 192);
        let mut subject = RECT::default();
        GetWindowRect(GetDlgItem(hwnd, SUBJECT as i32), &mut subject);
        assert_eq!(subject.bottom - subject.top, 50);
        let message = MSG {
            hwnd: body,
            message: WM_KEYDOWN,
            wParam: VK_ESCAPE as usize,
            ..Default::default()
        };
        assert!(window.translate(&message));
        assert!(!window.active());
        assert!(window.hwnd().is_null());
    }
}

#[test]
fn html_reader_uses_renderer_and_preserves_plain_fallback() {
    let _guard = super::super::WINDOW_TEST_LOCK.lock().unwrap();
    let _dpi = dpi::ThreadContext::enter();
    unsafe { classic::init() };
    let temp = Temp::new();
    let (id, message) = temp.message(b"Subject: HTML reader\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<table><tr><td>HTML text</td></tr></table><img src=\"https://example.invalid/blocked.png\">");
    let owner = Owner::new();
    let window = Window::create(owner.hwnd, id, message, false).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pump();
        let status = window.state.renderer.borrow().as_ref().unwrap().status();
        match status {
            HtmlStatus::Ready => break,
            HtmlStatus::Failed(error) => panic!("MSHTML reader failed: {error}"),
            HtmlStatus::Loading => {
                assert!(
                    Instant::now() < deadline,
                    "HTML reader did not become ready"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    unsafe {
        let body = GetDlgItem(window.hwnd(), BODY as i32);
        assert!(window_text(body).contains("HTML text"));
        assert_eq!(GetWindowLongW(body, GWL_STYLE) as u32 & WS_VISIBLE, 0);
        let renderer = window.state.renderer.borrow();
        let renderer = renderer.as_ref().unwrap();
        assert_ne!(
            GetWindowLongW(renderer.hwnd(), GWL_STYLE) as u32 & WS_VISIBLE,
            0
        );
        assert!(renderer.has_blocked_images());
        assert_ne!(
            GetWindowLongW(GetDlgItem(window.hwnd(), SHOW_PICTURES as i32), GWL_STYLE) as u32
                & WS_VISIBLE,
            0
        );
    }
    // WebView2 is opt-in; switching back destroys its host even while its
    // asynchronous controller is still being initialized.
    assert_eq!(window.state.engine.get(), WindowsHtmlEngine::Mshtml);
    unsafe {
        SendMessageW(window.hwnd(), WM_COMMAND, SWITCH_RENDERER as usize, 0);
    }
    assert_eq!(window.state.engine.get(), WindowsHtmlEngine::WebView2);
    assert_eq!(
        unsafe { GetWindowLongW(window.state.controls.body.get(), GWL_STYLE) as u32 & WS_VISIBLE },
        0
    );
    assert_ne!(
        unsafe {
            GetWindowLongW(window.state.controls.loading.get(), GWL_STYLE) as u32 & WS_VISIBLE
        },
        0
    );
    let web_host = window.state.renderer.borrow().as_ref().unwrap().hwnd();
    unsafe {
        SendMessageW(window.hwnd(), WM_COMMAND, SWITCH_RENDERER as usize, 0);
    }
    assert_eq!(unsafe { IsWindow(web_host) }, 0);
    assert_eq!(window.state.engine.get(), WindowsHtmlEngine::Mshtml);
    pump();
    // Also exercise a fully initialized WebView, then release it on close.
    unsafe {
        SendMessageW(window.hwnd(), WM_COMMAND, SWITCH_RENDERER as usize, 0);
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pump();
        match window.state.renderer.borrow().as_ref().unwrap().status() {
            HtmlStatus::Ready => break,
            HtmlStatus::Failed(error) => panic!("WebView reader failed: {error}"),
            HtmlStatus::Loading => {
                assert!(
                    Instant::now() < deadline,
                    "WebView reader did not become ready"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    let web_host = window.state.renderer.borrow().as_ref().unwrap().hwnd();
    window.close();
    assert_eq!(unsafe { IsWindow(web_host) }, 0);
    pump();
    assert!(window.state.renderer.borrow().is_none());
    assert!(!window.active());
}
