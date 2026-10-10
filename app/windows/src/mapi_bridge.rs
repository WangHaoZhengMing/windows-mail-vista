//! File-backed, per-user compose requests shared by Simple MAPI and the UI.
//! Only the GUI sends; the caller waits for SMTP success or user cancellation.
use crate::{Result, message::Draft, platform::strings::wide};
use std::{
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{System::Registry::*, UI::WindowsAndMessaging::*};
const CLIENT: &str = r"Software\WindowsMailRust";
fn executable() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("WINMAIL_EXECUTABLE") {
        return Ok(path.into());
    }
    unsafe {
        for (root, key, name) in [
            (HKEY_CURRENT_USER, CLIENT, "Executable"),
            (
                HKEY_LOCAL_MACHINE,
                r"Software\Clients\Mail\Windows Mail Rust",
                "Executable",
            ),
        ] {
            let mut data = vec![0u16; 32768];
            let mut size = (data.len() * 2) as u32;
            if RegGetValueW(
                root,
                wide(key).as_ptr(),
                wide(name).as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                data.as_mut_ptr().cast(),
                &mut size,
            ) == 0
            {
                let end = data.iter().position(|&c| c == 0).unwrap_or(data.len());
                return Ok(String::from_utf16(&data[..end])
                    .map_err(|e| e.to_string())?
                    .into());
            }
        }
    }
    Err("Register Windows Mail before using Simple MAPI".into())
}
fn root() -> PathBuf {
    std::env::temp_dir().join("WinMailRust-compose")
}
pub struct Request {
    directory: PathBuf,
}
impl Request {
    pub fn create(mut draft: Draft) -> Result<Self> {
        let root = root();
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let directory = root.join(format!("{}-{tick}", std::process::id()));
        std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
        let request = Self { directory };
        for (i, path) in draft.attachments.iter_mut().enumerate() {
            // Preserve display names; one directory per attachment avoids collisions.
            let folder = request.directory.join(format!("attachment-{i}"));
            std::fs::create_dir(&folder).map_err(|e| e.to_string())?;
            let copy = folder.join(path.file_name().ok_or("Invalid attachment filename")?);
            std::fs::copy(&*path, &copy).map_err(|e| e.to_string())?;
            *path = copy;
        }
        let data = serde_json::to_vec(&draft).map_err(|e| e.to_string())?;
        crate::store::atomic_write(&request.path(), &data)?;
        Ok(request)
    }
    pub fn path(&self) -> PathBuf {
        self.directory.join("request.json")
    }
    pub fn result(&self) -> Option<u32> {
        std::fs::read_to_string(self.directory.join("result"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
pub struct Completion {
    directory: PathBuf,
    completed: bool,
}
impl Completion {
    pub fn load(path: &Path) -> Result<(Draft, Self)> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let expected = root().canonicalize().map_err(|e| e.to_string())?;
        let directory = path.parent().ok_or("Invalid compose request")?;
        if directory.parent() != Some(expected.as_path())
            || path.file_name() != Some(std::ffi::OsStr::new("request.json"))
        {
            return Err("Compose request is outside the request directory".into());
        }
        let mut completion = Self {
            directory: directory.to_owned(),
            completed: false,
        };
        let load = (|| {
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            if bytes.len() > 1024 * 1024 {
                return Err("Compose request is too large".into());
            }
            let draft: Draft = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            for attachment in &draft.attachments {
                if !attachment
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(directory)
                {
                    return Err("Attachment is outside the request directory".into());
                }
            }
            Ok(draft)
        })();
        match load {
            Ok(draft) => Ok((draft, completion)),
            Err(error) => {
                completion.finish(2);
                Err(error)
            }
        }
    }
    pub fn ready(&self) {
        let _ = crate::store::atomic_write(&self.directory.join("ready"), b"1");
    }
    pub fn finish(&mut self, code: u32) {
        if !self.completed {
            self.completed = crate::store::atomic_write(
                &self.directory.join("result"),
                code.to_string().as_bytes(),
            )
            .is_ok();
        }
    }
}
impl Drop for Completion {
    fn drop(&mut self) {
        self.finish(1);
    }
}
/// Called on the external caller's thread. Pump its messages while the GUI owns the draft.
pub fn send_dialog(draft: Draft, owner: usize) -> std::result::Result<u32, String> {
    use std::os::windows::process::CommandExt;
    let exe = executable()?;
    let request = Request::create(draft)?;
    let mut child = std::process::Command::new(exe)
        .arg(format!("/mapi-request:{}", request.path().display()))
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|e| e.to_string())?;
    let _modal = ModalOwner::new(owner);
    let start = Instant::now();
    let class = wide(&crate::instance_names().1);
    loop {
        if let Some(code) = request.result() {
            return Ok(code);
        }
        let ready = request.directory.join("ready").is_file();
        if ready && unsafe { FindWindowW(class.as_ptr(), null()) }.is_null() {
            return Err("Mail client closed before the request completed".into());
        }
        if !ready && start.elapsed() > Duration::from_secs(30) {
            return Err("Mail client did not accept the compose request".into());
        }
        if !ready
            && let Some(status) = child.try_wait().map_err(|e| e.to_string())?
            && !status.success()
        {
            return Err("Mail client could not start".into());
        }
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    PostQuitMessage(msg.wParam as i32);
                    return Ok(1);
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
struct ModalOwner(windows_sys::Win32::Foundation::HWND);
impl ModalOwner {
    fn new(owner: usize) -> Self {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled};
        let window = owner as windows_sys::Win32::Foundation::HWND;
        unsafe {
            if IsWindow(window) != 0 && IsWindowEnabled(window) != 0 {
                EnableWindow(window, 0);
                Self(window)
            } else {
                Self(null_mut())
            }
        }
    }
}
impl Drop for ModalOwner {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() && IsWindow(self.0) != 0 {
                windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(self.0, 1);
                SetForegroundWindow(self.0);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compose_results_and_attachment_copies_survive_caller_files() {
        let source =
            std::env::temp_dir().join(format!("winmail-bridge-{}.txt", std::process::id()));
        std::fs::write(&source, b"attachment").unwrap();
        let request = Request::create(Draft {
            to: "a@example.com".into(),
            attachments: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
        std::fs::remove_file(source).unwrap();
        let (draft, mut completion) = Completion::load(&request.path()).unwrap();
        assert_eq!(std::fs::read(&draft.attachments[0]).unwrap(), b"attachment");
        completion.finish(0);
        drop(completion);
        assert_eq!(request.result(), Some(0));
    }
    #[test]
    fn dropping_composer_reports_cancel_and_outside_request_is_rejected() {
        let request = Request::create(Draft::default()).unwrap();
        let (_, completion) = Completion::load(&request.path()).unwrap();
        drop(completion);
        assert_eq!(request.result(), Some(1));
        assert!(Completion::load(Path::new("Cargo.toml")).is_err());
    }
}
