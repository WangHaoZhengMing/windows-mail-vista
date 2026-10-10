#![cfg(windows)]
use mail_core::{
    account::{Account, Incoming, Security},
    store::Store,
};
use std::{
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant},
};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("winmail-ui-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe fn text(window: HWND) -> String {
    unsafe {
        // GetWindowTextW intentionally cannot retrieve another process's edit controls.
        let mut buf = vec![0u16; SendMessageW(window, WM_GETTEXTLENGTH, 0, 0).max(0) as usize + 1];
        let n = SendMessageW(window, WM_GETTEXT, buf.len(), buf.as_mut_ptr() as isize);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}
#[track_caller]
fn until(mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while !check() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "Windows UI timed out"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn independent_window_import_ipc_draft_reopen_and_clean_shutdown() {
    if !mail_windows::startup::native::manual_launch_allowed() {
        eprintln!("UI test skipped due to local launch policy");
        return;
    }
    let tmp = Temp::new();
    let store_root = tmp.0.join("store");
    let eml = tmp.0.join("sample.eml");
    std::fs::write(&eml,b"From: sender@example.com\r\nTo: to@example.com\r\nSubject: Imported fixture\r\n\r\nReadable body\r\n").unwrap();
    {
        let mut store = Store::open(&store_root).unwrap();
        let mut account = Account {
            email: "self@example.com".into(),
            username: "self".into(),
            auth: Default::default(),
            password_dpapi: String::new(),
            incoming: Incoming::Imap,
            incoming_host: "127.0.0.1".into(),
            incoming_port: 993,
            incoming_security: Security::Tls,
            smtp_host: "127.0.0.1".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
        };
        account.set_password("fixture-only").unwrap();
        store.set_account(account).unwrap();
    }
    let key = format!("rust-ui-{}", std::process::id());
    let class = wide(&mail_windows::instance_names_for(&key).1);
    let mut command = Command::new(env!("CARGO_BIN_EXE_WinMail"));
    command
        .env_remove("WINMAIL_RUNTIME_DIR")
        .env("WINMAIL_DATA_DIR", &store_root)
        .env("WINMAIL_INSTANCE_ID", &key)
        .arg(&eml);
    let mut running = Running(command.spawn().unwrap());
    let mut hwnd = std::ptr::null_mut();
    until(|| {
        hwnd = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
        !hwnd.is_null()
    });
    let mut list = std::ptr::null_mut();
    until(|| {
        list = unsafe { GetDlgItem(hwnd, 100) };
        !list.is_null()
    });
    until(|| unsafe { SendMessageW(list, LVM_GETITEMCOUNT, 0, 0) == 1 });
    unsafe {
        SendMessageW(list, WM_KEYDOWN, 0x24, 0);
    }
    assert_eq!(unsafe { text(GetDlgItem(hwnd, 203)) }, "Imported fixture");
    assert!(unsafe { text(GetDlgItem(hwnd, 204)) }.contains("Readable body"));
    unsafe {
        SendMessageW(hwnd, WM_COMMAND, 111, 0);
    }
    let compose_class = wide("WinMailRust.Compose");
    let mut composer = unsafe { FindWindowW(compose_class.as_ptr(), std::ptr::null()) };
    assert!(!composer.is_null());
    assert_eq!(
        unsafe { text(GetDlgItem(composer, 201)) },
        "sender@example.com"
    );
    assert_eq!(
        unsafe { text(GetDlgItem(composer, 203)) },
        "Re: Imported fixture"
    );
    assert!(unsafe { text(GetDlgItem(composer, 204)) }.contains("> Readable body"));
    let forwarded = Command::new(env!("CARGO_BIN_EXE_WinMail"))
        .env_remove("WINMAIL_RUNTIME_DIR").env("WINMAIL_DATA_DIR",&store_root).env("WINMAIL_INSTANCE_ID",&key)
        .arg("/mailurl:mailto:to@example.com?bcc=hidden%40example.com&subject=测试 with spaces&body=persisted").output().unwrap();
    assert!(
        forwarded.status.success(),
        "{}",
        String::from_utf8_lossy(&forwarded.stderr)
    );
    until(|| unsafe {
        composer = FindWindowW(compose_class.as_ptr(), std::ptr::null());
        text(GetDlgItem(composer, 203)) == "测试 with spaces"
    });
    assert_eq!(
        unsafe { text(GetDlgItem(composer, 205)) },
        "hidden@example.com"
    );
    unsafe {
        SendMessageW(hwnd, WM_COMMAND, 107, 0);
    }
    // Imported mail stays Local; the composer's From account owns its saved draft.
    // Navigate through the account's expanded children using real tree keyboard input.
    unsafe {
        let tree = GetDlgItem(hwnd, 110);
        SendMessageW(tree, WM_KEYDOWN, 0x24, 0); // Home: first account root.
        for _ in 0..4 {
            SendMessageW(tree, WM_KEYDOWN, 0x28, 0);
        } // Drafts.
    }
    until(|| unsafe {
        text(hwnd).contains("Drafts") && SendMessageW(list, LVM_GETITEMCOUNT, 0, 0) == 1
    });
    // Select, then activate the persisted draft in its own editor.
    unsafe {
        SendMessageW(list, WM_KEYDOWN, 0x24, 0);
        SendMessageW(list, WM_KEYDOWN, 0x0d, 0);
    }
    until(|| unsafe {
        composer = FindWindowW(compose_class.as_ptr(), std::ptr::null());
        text(GetDlgItem(composer, 205)) == "hidden@example.com"
    });
    assert_eq!(
        unsafe { text(GetDlgItem(composer, 205)) },
        "hidden@example.com"
    );
    assert_eq!(unsafe { text(GetDlgItem(composer, 204)) }, "persisted");
    let html = tmp.0.join("html.eml");
    std::fs::write(&html, b"From: html@example.com\r\nTo: self@example.com\r\nSubject: HTML fixture\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<table><tr><td style=\"color:blue\">Rendered body</td></tr></table>").unwrap();
    let forwarded = Command::new(env!("CARGO_BIN_EXE_WinMail"))
        .env_remove("WINMAIL_RUNTIME_DIR")
        .env("WINMAIL_DATA_DIR", &store_root)
        .env("WINMAIL_INSTANCE_ID", &key)
        .arg(&html)
        .output()
        .unwrap();
    assert!(forwarded.status.success());
    until(|| unsafe { SendMessageW(list, LVM_GETITEMCOUNT, 0, 0) == 2 });
    unsafe {
        SendMessageW(list, WM_KEYDOWN, 0x24, 0);
    }
    until(|| unsafe { text(GetDlgItem(hwnd, 203)) == "HTML fixture" });
    until(|| unsafe {
        let preview = GetDlgItem(hwnd, 205);
        !preview.is_null() && IsWindowVisible(preview) != 0
    });
    assert!(unsafe { text(GetDlgItem(hwnd, 204)) }.contains("Rendered body"));
    assert_eq!(
        unsafe { IsWindowVisible(GetDlgItem(hwnd, 204)) },
        0,
        "The HTML control must replace the plain-text fallback once ready"
    );
    unsafe {
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
    }
    until(|| running.0.try_wait().unwrap().is_some());
    assert!(running.0.wait().unwrap().success());
    let store = Store::open(&store_root).unwrap();
    assert_eq!(store.entries().len(), 3);
    let entry = store
        .entries()
        .iter()
        .find(|e| e.folder == "Drafts")
        .unwrap();
    assert_eq!(store.draft(&entry.id).unwrap().bcc, "hidden@example.com");
    assert_eq!(entry.account_id.as_ref(), store.default_account_id());
}
