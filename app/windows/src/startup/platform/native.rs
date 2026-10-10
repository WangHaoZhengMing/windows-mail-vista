//! Windows implementation. RVAs refer to analysis/WinMail.annotated.asm.
//! Every function pointer conversion uses an ABI checked at the caller/callee.
use crate::startup::command::{self, Route};
use crate::startup::{HIDDEN_WINDOW, INSTANCE_MUTEX, RESTART, START_EXPORT, START_FLAGS};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Com::*;
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::*;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Debug)]
pub struct Failure {
    pub exit_code: i32,
    pub detail: String,
}
type Result<T> = std::result::Result<T, Failure>;
fn fail(exit_code: i32, detail: impl Into<String>) -> Failure {
    Failure {
        exit_code,
        detail: detail.into(),
    }
}
pub(crate) fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("WINMAIL_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."))
        })
}
pub fn external_runtime() -> bool {
    std::env::var_os("WINMAIL_RUNTIME_DIR").is_some()
}
pub fn instance_names() -> (String, String) {
    if external_runtime() && std::env::var_os("WINMAIL_INSTANCE_ID").is_none() {
        (INSTANCE_MUTEX.into(), HIDDEN_WINDOW.into())
    } else {
        crate::instance_names()
    }
}
pub fn raw_tail() -> Vec<u16> {
    // GetCommandLineW belongs to the process and is NUL terminated.
    unsafe {
        let p = windows_sys::Win32::System::Environment::GetCommandLineW();
        let mut len = 0;
        while *p.add(len) != 0 {
            len += 1;
        }
        command::command_tail(std::slice::from_raw_parts(p, len)).to_vec()
    }
}

pub(crate) struct Library(HMODULE);
impl Library {
    pub(crate) fn load(path: &Path) -> std::result::Result<Self, u32> {
        let path = match std::path::absolute(path) {
            Ok(p) => p,
            Err(_) => return Err(87),
        };
        let path = wide(path.as_os_str());
        // Explicit core location, then its directory and System32 for dependencies.
        let h = unsafe {
            LoadLibraryExW(
                path.as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if h.is_null() {
            Err(unsafe { GetLastError() })
        } else {
            Ok(Self(h))
        }
    }
    pub(crate) fn symbol(&self, name: &[u8]) -> FARPROC {
        unsafe { GetProcAddress(self.0, name.as_ptr()) }
    }
    fn ordinal(&self, ordinal: u16) -> FARPROC {
        unsafe { GetProcAddress(self.0, ordinal as usize as *const u8) }
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.0);
        }
    }
}

struct Mutex {
    handle: HANDLE,
    owned: bool,
}
impl Mutex {
    fn acquire() -> Result<Self> {
        let name = wide(instance_names().0);
        let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(fail(100, "CreateMutexW failed"));
        }
        let status = unsafe { WaitForSingleObject(handle, 60_000) };
        if status != WAIT_OBJECT_0 {
            // Original treats WAIT_ABANDONED as code 101; release inherited ownership.
            if status == WAIT_ABANDONED {
                unsafe {
                    ReleaseMutex(handle);
                }
            }
            unsafe {
                CloseHandle(handle);
            }
            return Err(fail(
                101,
                format!("instance mutex wait failed: 0x{status:x}"),
            ));
        }
        Ok(Self {
            handle,
            owned: true,
        })
    }
    fn release(&mut self) {
        if self.owned {
            unsafe {
                ReleaseMutex(self.handle);
            }
            self.owned = false;
        }
    }
}
impl Drop for Mutex {
    fn drop(&mut self) {
        self.release();
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

struct Hook(HHOOK);
impl Drop for Hook {
    fn drop(&mut self) {
        unsafe {
            UnhookWindowsHookEx(self.0);
        }
    }
}
struct Ole;
impl Ole {
    fn initialize() -> std::result::Result<Self, i32> {
        let hr = unsafe { OleInitialize(null()) };
        if hr < 0 { Err(hr) } else { Ok(Self) }
    }
}
impl Drop for Ole {
    fn drop(&mut self) {
        unsafe {
            OleUninitialize();
        }
    }
}

pub fn manual_launch_allowed() -> bool {
    unsafe {
        SHRegGetBoolUSValueA(
            c"Software\\Policies\\Microsoft\\Windows Mail"
                .as_ptr()
                .cast(),
            c"ManualLaunchAllowed".as_ptr().cast(),
            0,
            1,
        ) != 0
    }
}

pub fn run(tail: &[u16], runtime: &Path, show: i32) -> Result<i32> {
    let setup = String::from_utf16_lossy(tail)
        .trim_end_matches('\0')
        .trim()
        .to_ascii_lowercase();
    let operation = match setup.as_str() {
        "/register-app" => Some(crate::integration::register()),
        "/unregister-app" => Some(crate::integration::unregister()),
        "/register-mapi" => Some(crate::integration::register_mapi()),
        "/unregister-mapi" => Some(crate::integration::unregister_mapi()),
        _ => None,
    };
    if let Some(result) = operation {
        return result.map(|()| 0).map_err(|e| fail(102, e));
    }
    let runtime = std::path::absolute(runtime).map_err(|e| fail(102, e.to_string()))?;
    let route = command::route(tail);
    if command::policy_blocks(tail, manual_launch_allowed()) {
        return Err(fail(-1, "ManualLaunchAllowed policy blocks Windows Mail"));
    }
    if unsafe { HeapSetInformation(null_mut(), HeapEnableTerminationOnCorruption, null(), 0) } == 0
    {
        return Err(fail(-1, "HeapSetInformation failed"));
    }
    if unsafe { SetConsoleCtrlHandler(Some(ignore_console_control), 1) } == 0 {
        return Err(fail(-1, "SetConsoleCtrlHandler failed"));
    }
    let result = match route {
        Route::Register => {
            super::registration::update(true);
            Ok(1)
        }
        Route::Unregister => {
            super::registration::update(false);
            Ok(1)
        }
        Route::Identify => identify(tail, &runtime),
        Route::Install => legacy_install(tail, &runtime),
        Route::Launch => launch(tail, &runtime, show),
    };
    unsafe {
        SetConsoleCtrlHandler(Some(ignore_console_control), 0);
    }
    result
}

unsafe extern "system" fn ignore_console_control(_: u32) -> i32 {
    1
}

pub fn show_command() -> i32 {
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    unsafe {
        GetStartupInfoW(&mut startup);
    }
    if startup.dwFlags & STARTF_USESHOWWINDOW != 0 {
        startup.wShowWindow as i32
    } else {
        SW_SHOWDEFAULT
    }
}

fn core(runtime: &Path) -> Result<Library> {
    if !cfg!(target_arch = "x86_64") {
        return Err(fail(102, "The original Windows Mail runtime requires x64"));
    }
    Library::load(&runtime.join("msoe.dll")).map_err(|error| {
        fail(
            crate::startup::loader_exit_code(
                error,
                runtime.join("msoe.dll").is_file(),
                runtime.join("MSOERES.dll").is_file(),
            ),
            format!(
                "LoadLibraryExW({}) failed: Win32 error {error}",
                runtime.join("msoe.dll").display()
            ),
        )
    })
}

fn identify(tail: &[u16], runtime: &Path) -> Result<i32> {
    if !external_runtime() {
        crate::startup::identify::run(&String::from_utf16_lossy(tail));
        return Ok(1);
    }
    // RVA 0x31e8..0x32bd: recognized even if the DLL/ordinal cannot be loaded.
    if let Ok(library) = core(runtime)
        && let Some(p) = library.ordinal(22)
    {
        let f: unsafe extern "system" fn(*const u16) -> i32 = unsafe { std::mem::transmute(p) };
        let mut tail = tail.to_vec();
        tail.push(0);
        unsafe {
            f(tail.as_ptr());
        }
    }
    Ok(1)
}

fn legacy_install(tail: &[u16], runtime: &Path) -> Result<i32> {
    if !external_runtime() {
        return Err(fail(
            102,
            "Legacy OCInstall maintenance has not been rewritten; it requires an explicit WINMAIL_RUNTIME_DIR pointing to the original installation",
        ));
    }
    // Setup-only functions have not been translated; execute the original binary.
    // Never route ordinary launch through this compatibility path.
    use std::os::windows::process::CommandExt;
    let executable = runtime.join("WinMail.exe");
    if std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().and_then(|p| p.canonicalize().ok()))
        == runtime.canonicalize().ok()
    {
        return Err(fail(
            102,
            "OCInstall cannot target the Rust build directory",
        ));
    }
    let args = std::ffi::OsString::from_wide(tail);
    let status = std::process::Command::new(&executable)
        .raw_arg(args)
        .creation_flags(CREATE_NO_WINDOW)
        .current_dir(runtime)
        .status()
        .map_err(|e| {
            fail(
                102,
                format!("legacy OCInstall requires {}: {e}", executable.display()),
            )
        })?;
    Ok(status.code().unwrap_or(102))
}

fn launch(tail: &[u16], runtime: &Path, show: i32) -> Result<i32> {
    let mut instance = Mutex::acquire()?;
    let name = wide(instance_names().1);
    let existing = unsafe { FindWindowW(name.as_ptr(), null()) };
    let mut tail = tail.to_vec();
    tail.push(0);
    if !existing.is_null() {
        let packet = COPYDATASTRUCT {
            dwData: 1,
            cbData: (tail.len() * 2)
                .try_into()
                .map_err(|_| fail(101, "command line too large"))?,
            lpData: tail.as_mut_ptr().cast(),
        };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(existing, &mut pid);
            AllowSetForegroundWindow(pid);
            SetForegroundWindow(existing);
        }
        instance.release();
        drop(instance); // release before synchronous IPC, as in RVA 0x41df.
        let mut result = 0;
        let delivered = unsafe {
            SendMessageTimeoutW(
                existing,
                WM_COPYDATA,
                0,
                &packet as *const _ as isize,
                0,
                1500,
                &mut result,
            )
        };
        return if delivered != 0 {
            Ok(0)
        } else {
            Err(fail(101, "WM_COPYDATA delivery failed"))
        };
    }
    let library = if external_runtime() {
        Some(core(runtime)?)
    } else {
        None
    };
    // The legacy DLL ABI has three parameters. The current Rust UI uses typed
    // arguments directly and does not depend on the compatibility DLL.
    let start: Option<unsafe extern "system" fn(u32, *const u16, i32) -> i32> = match &library {
        Some(library) => {
            let p = library
                .symbol(START_EXPORT)
                .ok_or_else(|| fail(102, "CoStartOutlookExpressW export missing"))?;
            Some(unsafe {
                std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    unsafe extern "system" fn(u32, *const u16, i32) -> i32,
                >(p)
            })
        }
        None => None,
    };
    let hook = if start.is_some() {
        let handle = unsafe {
            SetWindowsHookExW(
                WH_GETMESSAGE,
                Some(message_hook),
                null_mut(),
                GetCurrentThreadId(),
            )
        };
        if handle.is_null() {
            return Err(fail(
                104,
                format!("SetWindowsHookExW failed: {}", unsafe { GetLastError() }),
            ));
        }
        Some(Hook(handle))
    } else {
        None
    };
    let ole = match Ole::initialize() {
        Ok(ole) => ole,
        Err(_) => return Ok(0), // Original RVA 0x42fd -> 0x439f suppresses this failure.
    };
    let result = if let Some(start) = start {
        let empty = [0u16];
        let mut args = tail.as_ptr();
        let result = loop {
            let hr = unsafe { start(START_FLAGS, args, show) };
            args = empty.as_ptr(); // The historical restart clears the launch arguments.
            if hr != RESTART {
                break hr;
            }
        };
        let code = crate::startup::normalize_core_result(result);
        if code == 0 {
            Ok(0)
        } else {
            Err(fail(
                code,
                format!("legacy mail DLL returned HRESULT 0x{:08x}", result as u32),
            ))
        }
    } else {
        let command = String::from_utf16_lossy(&tail[..tail.len() - 1]);
        crate::ui::run(&command, show)
            .map(|()| 0)
            .map_err(|detail| fail(103, detail))
    };
    if library.is_some() {
        unsafe {
            let inetcomm = GetModuleHandleW(wide("inetcomm.dll").as_ptr());
            if !inetcomm.is_null()
                && let Some(p) = GetProcAddress(inetcomm, 702usize as *const u8)
            {
                let cleanup: unsafe extern "system" fn() = std::mem::transmute(p);
                cleanup();
            }
            CoFreeUnusedLibraries();
            CoFreeUnusedLibrariesEx(0, 0);
        }
    }
    drop(ole);
    if library.is_some() {
        unsafe {
            CoFreeAllLibraries();
        }
    }
    drop(instance);
    drop(library);
    drop(hook);
    result
}

/// RVA 0x5fd0: call the next hook first, then consume messages handled by the host.
unsafe extern "system" fn message_hook(code: i32, removed: usize, data: isize) -> isize {
    unsafe {
        let next = CallNextHookEx(null_mut(), code, removed, data);
        if code >= 0 && removed == PM_REMOVE as usize && data != 0 {
            let msg = &mut *(data as *mut MSG);
            if filter_message(msg) {
                msg.message = WM_NULL;
            }
        }
        next
    }
}

unsafe fn filter_message(source: &MSG) -> bool {
    if !((0x100..=0x109).contains(&source.message)
        || (0x200..=0x20e).contains(&source.message)
        || matches!(source.message, 0xa1 | 0xa4 | 0xa7))
    {
        return false;
    }
    unsafe {
        let mut host = source.hwnd;
        let property = wide("OETLWindow");
        while !host.is_null() && GetPropW(host, property.as_ptr()).is_null() {
            host = GetParent(host);
        }
        if host.is_null() {
            return false;
        }
        let thread = GetWindowThreadProcessId(host, null_mut());
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(thread, &mut info) != 0
            && info.flags & GUI_INMENUMODE != 0
            && (info.hwndMenuOwner.is_null()
                || info.hwndMenuOwner == host
                || IsChild(host, info.hwndMenuOwner) != 0)
        {
            return false;
        }
        // The original TLS modeless-dialog collection has no populated entries in
        // the recovered EXE. That private collection is not exposed across DLLs.
        let mut msg = *source;
        msg.time = u32::MAX;
        msg.pt = POINT { x: -1, y: -1 };
        let mut handled = 0i32;
        SendMessageW(
            host,
            0x9c3f,
            &mut msg as *mut _ as usize,
            &mut handled as *mut _ as isize,
        ) != 0
    }
}

pub fn probe(runtime: &Path) -> Result<()> {
    let library = core(runtime)?;
    if library.symbol(START_EXPORT).is_none() {
        return Err(fail(102, "CoStartOutlookExpressW export missing"));
    }
    println!(
        "Loaded {}; CoStartOutlookExpressW found. Mail entry point was not called.",
        runtime.join("msoe.dll").display()
    );
    Ok(())
}

use std::os::windows::ffi::OsStringExt;
