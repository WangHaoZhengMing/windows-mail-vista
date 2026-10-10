#![cfg(windows)]
use mail_core::store::Store;
use std::{path::PathBuf, process::Command};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("winmail-mapi-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn simple_mapi_local_store_contracts() {
    let tmp = Temp::new();
    {
        let mut store = Store::open(&tmp.0).unwrap();
        store.import(b"From: sender@example.com\r\nTo: to@example.com\r\nSubject: MAPI fixture\r\n\r\nhello", "Inbox", None).unwrap();
    }
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "mapi_fixture", "--nocapture"])
        .env("WINMAIL_MAPI_TEST", "1")
        .env("WINMAIL_DATA_DIR", &tmp.0)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    let store = Store::open(&tmp.0).unwrap();
    assert!(store.entries().iter().any(|e| e.folder == "Trash"));
    let entry = store
        .entries()
        .iter()
        .find(|e| e.folder == "Drafts")
        .unwrap();
    assert_eq!(store.draft(&entry.id).unwrap().bcc, "hidden@example.com");
}

#[test]
fn mapi_fixture() {
    if std::env::var_os("WINMAIL_MAPI_TEST").is_none() {
        return;
    }
    use msoe::mapi::*;
    use windows_sys::Win32::System::Mapi::*;
    unsafe {
        let mut session = 0;
        assert_eq!(
            MAPILogon(0, std::ptr::null(), std::ptr::null(), 0, 0, &mut session),
            0
        );
        assert_ne!(session, 0);
        assert_eq!(msoe::abi::DllCanUnloadNow(), 1);
        let mut id = [0; 512];
        assert_eq!(
            MAPIFindNext(
                session,
                0,
                std::ptr::null(),
                std::ptr::null(),
                MAPI_LONG_MSGID,
                0,
                id.as_mut_ptr()
            ),
            0
        );
        let mut mail = std::ptr::null_mut();
        assert_eq!(
            MAPIReadMail(session, 0, id.as_ptr(), MAPI_PEEK, 0, &mut mail),
            0
        );
        assert_eq!(ansi_input((*mail).lpszSubject).unwrap(), "MAPI fixture");
        assert!((*mail).flFlags & MAPI_UNREAD != 0);
        assert_eq!(msoe::mapi::MAPIFreeBuffer(mail.cast()), 0);
        assert_eq!(
            msoe::mapi::MAPIFreeBuffer(mail.cast()),
            MAPI_E_INVALID_MESSAGE
        );
        let mut recipient = std::ptr::null_mut();
        assert_eq!(
            MAPIResolveName(
                session,
                0,
                c"SMTP:hidden@example.com".as_ptr().cast(),
                0,
                0,
                &mut recipient
            ),
            0
        );
        assert_eq!(
            ansi_input((*recipient).lpszAddress).unwrap(),
            "SMTP:hidden@example.com"
        );
        (*recipient).ulRecipClass = MAPI_BCC;
        let to = MapiRecipDesc {
            ulRecipClass: MAPI_TO,
            lpszAddress: c"SMTP:to@example.com".as_ptr().cast_mut().cast(),
            ..Default::default()
        };
        let mut recips = [to, *recipient];
        let mut message = MapiMessage {
            lpszSubject: c"saved".as_ptr().cast_mut().cast(),
            lpszNoteText: c"draft body".as_ptr().cast_mut().cast(),
            nRecipCount: 2,
            lpRecips: recips.as_mut_ptr(),
            ..Default::default()
        };
        let mut draft_id = [0; 512];
        assert_eq!(
            MAPISaveMail(session, 0, &mut message, 0, 0, draft_id.as_mut_ptr()),
            0
        );
        assert_eq!(
            MAPISendMail(session, 0, std::ptr::null_mut(), MAPI_DIALOG, 0),
            MAPI_E_INVALID_MESSAGE
        );
        assert_eq!(msoe::mapi::MAPIFreeBuffer(recipient.cast()), 0);
        assert_eq!(MAPIDeleteMail(session, 0, id.as_ptr(), 0, 0), 0);
        assert_eq!(
            MAPIFindNext(
                session,
                0,
                std::ptr::null(),
                std::ptr::null(),
                0,
                0,
                id.as_mut_ptr()
            ),
            MAPI_E_NO_MESSAGES
        );
        assert_eq!(MAPILogoff(session, 0, 0, 0), 0);
        assert_eq!(MAPILogoff(session, 0, 0, 0), MAPI_E_INVALID_SESSION);
        assert_eq!(msoe::abi::DllCanUnloadNow(), 0);
    }
}

#[test]
#[ignore = "Build WinMail.exe first and set WINMAIL_EXECUTABLE to that executable"]
fn interactive_mapi_opens_gui_and_returns_cancel() {
    let exe = std::env::var_os("WINMAIL_EXECUTABLE")
        .expect("Set WINMAIL_EXECUTABLE to a freshly built WinMail.exe");
    let tmp = Temp::new();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "mapi_dialog_fixture", "--nocapture"])
        .env("WINMAIL_DIALOG_TEST", "1")
        .env("WINMAIL_DATA_DIR", &tmp.0)
        .env("WINMAIL_EXECUTABLE", exe)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn mapi_dialog_fixture() {
    if std::env::var_os("WINMAIL_DIALOG_TEST").is_none() {
        return;
    }
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let thread = std::thread::spawn(|| {
        use windows_sys::Win32::System::Mapi::*;
        let mut subject: Vec<u16> = "Unicode MAPI test 中文"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut mail = MapiMessageW {
            lpszSubject: subject.as_mut_ptr(),
            ..Default::default()
        };
        unsafe { msoe::mapi::MAPISendMailW(0, 0, &mut mail, MAPI_DIALOG, 0) }
    });
    let class = wide(&msoe::instance_names().1);
    let compose_class = wide("WinMailRust.Compose");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    let mut main = std::ptr::null_mut();
    let mut composer = std::ptr::null_mut();
    unsafe {
        while std::time::Instant::now() < deadline && !thread.is_finished() {
            main = FindWindowW(class.as_ptr(), std::ptr::null());
            composer = FindWindowExW(
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                compose_class.as_ptr(),
                std::ptr::null(),
            );
            while !composer.is_null() && GetWindow(composer, GW_OWNER) != main {
                composer = FindWindowExW(
                    std::ptr::null_mut(),
                    composer,
                    compose_class.as_ptr(),
                    std::ptr::null(),
                );
            }
            if !main.is_null() && !composer.is_null() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if !composer.is_null() {
            PostMessageW(composer, WM_CLOSE, 0, 0);
        }
        let finish = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !thread.is_finished() && std::time::Instant::now() < finish {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        if !main.is_null() {
            PostMessageW(main, WM_CLOSE, 0, 0);
        }
    }
    assert!(
        !composer.is_null(),
        "MAPI did not create the compose window"
    );
    assert!(thread.is_finished(), "MAPI did not return after cancel");
    assert_eq!(thread.join().unwrap(), 1);
}
