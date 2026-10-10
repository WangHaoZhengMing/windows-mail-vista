#![cfg(windows)]
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn launcher(runtime: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_winmail-rust"));
    c.env("WINMAIL_RUNTIME_DIR", runtime);
    c.env(
        "WINMAIL_INSTANCE_ID",
        format!("native-test-{}", std::process::id()),
    );
    c
}
fn compile(source: &Path, dll: &Path) {
    let status = Command::new("rustc")
        .arg("--edition=2024")
        .arg("--crate-type=cdylib")
        .arg("--crate-name=winmail_test_core")
        .arg(source)
        .arg("-o")
        .arg(dll)
        .status()
        .unwrap();
    assert!(status.success(), "fixture DLL compilation failed");
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("winmail-rust-tests-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One sequential native test prevents our real instance mutex from racing itself.
#[test]
fn real_loader_restart_hresult_and_ipc_contracts() {
    if !mail_windows::startup::native::manual_launch_allowed() {
        eprintln!("Native launcher test skipped: local ManualLaunchAllowed policy is disabled.");
        return;
    }
    let class =
        wide(&mail_windows::instance_names_for(&format!("native-test-{}", std::process::id())).1);
    assert!(
        unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) }.is_null(),
        "close the existing Windows Mail instance before running native integration tests"
    );
    let scratch = Scratch::new();
    let runtime = scratch.0.join("runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    let missing = launcher(&runtime).output().unwrap();
    assert_eq!(
        missing.status.code(),
        Some(107),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );

    compile(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/core.rs"),
        &runtime.join("msoe.dll"),
    );
    let log = scratch.0.join("core.log");
    let input = "mailto:测试@example.test?subject=您好";
    let output = launcher(&runtime)
        .arg(input)
        .env("WINMAIL_TEST_LOG", &log)
        .env("WINMAIL_TEST_RESTART", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = std::fs::read_to_string(&log).unwrap();
    let expected: Vec<u16> = input.encode_utf16().collect();
    assert!(
        calls.contains(&format!("call=0;flags=63;show=10;tail={expected:?}")),
        "{calls}"
    );
    assert!(calls.contains("call=1;flags=63;show=10;tail=[]"), "{calls}");
    assert_eq!(calls.lines().count(), 2);
    let output = launcher(&runtime)
        .env("WINMAIL_TEST_LOG", &log)
        .env("WINMAIL_TEST_HRESULT", (0x80004005u32 as i32).to_string())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(103));
    assert!(String::from_utf8_lossy(&output.stderr).contains("80004005"));
    let output = launcher(&runtime)
        .env("WINMAIL_TEST_LOG", &log)
        .env("WINMAIL_TEST_HRESULT", (0x80040113u32 as i32).to_string())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));

    let ready = scratch.0.join("receiver.ready");
    let ipc_log = scratch.0.join("ipc.log");
    let mut receiver = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "ipc_receiver_fixture", "--nocapture"])
        .env("WINMAIL_TEST_RECEIVER", &ready)
        .env(
            "WINMAIL_INSTANCE_ID",
            format!("native-test-{}", std::process::id()),
        )
        .env("WINMAIL_TEST_IPC_LOG", &ipc_log)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.is_file() && Instant::now() < deadline {
        if let Some(status) = receiver.try_wait().unwrap() {
            panic!("receiver exited early: {status}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !ready.is_file() {
        let _ = receiver.kill();
        panic!("receiver did not become ready");
    }
    let output = launcher(&runtime).arg(input).output().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let receiver_status = loop {
        if let Some(status) = receiver.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = receiver.kill();
            panic!("receiver failed to exit");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(receiver_status.success());
    let expected: Vec<u16> = input.encode_utf16().chain(Some(0)).collect();
    assert_eq!(
        std::fs::read_to_string(ipc_log).unwrap(),
        format!("id=1;tail={expected:?}")
    );
    assert_eq!(
        std::fs::read_to_string(log).unwrap().lines().count(),
        4,
        "forwarding must not call the core DLL"
    );

    let no_export = scratch.0.join("no_export.rs");
    std::fs::write(
        &no_export,
        "#[unsafe(no_mangle)] pub extern \"system\" fn unrelated() {}",
    )
    .unwrap();
    compile(&no_export, &runtime.join("msoe.dll"));
    let output = launcher(&runtime).output().unwrap();
    assert_eq!(output.status.code(), Some(102));
    assert!(String::from_utf8_lossy(&output.stderr).contains("export missing"));
}

#[test]
fn ipc_receiver_fixture() {
    let Some(ready) = std::env::var_os("WINMAIL_TEST_RECEIVER") else {
        return;
    };
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide(&mail_windows::instance_names().1);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(receiver_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        assert_ne!(RegisterClassW(&wc), 0);
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        assert!(!window.is_null());
        std::fs::write(ready, "ready").unwrap();
        SetTimer(window, 1, 15_000, None);
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn receiver_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        match msg {
            WM_COPYDATA => {
                let packet = &*(lp as *const COPYDATASTRUCT);
                if packet.lpData.is_null() || !packet.cbData.is_multiple_of(2) {
                    return 0;
                }
                let words = std::slice::from_raw_parts(
                    packet.lpData.cast::<u16>(),
                    packet.cbData as usize / 2,
                );
                let saved = std::env::var_os("WINMAIL_TEST_IPC_LOG")
                    .and_then(|path| {
                        std::fs::write(path, format!("id={};tail={words:?}", packet.dwData)).ok()
                    })
                    .is_some();
                DestroyWindow(hwnd);
                isize::from(saved)
            }
            WM_TIMER => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
