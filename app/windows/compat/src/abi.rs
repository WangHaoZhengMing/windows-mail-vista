//! Windows Mail ABI adapters over the typed Windows application API.
//! Unimplemented COM classes fail with CLASS_E_CLASSNOTAVAILABLE.
use std::ptr::null_mut;

/// # Safety
/// `command` must be a valid NUL-terminated Windows ANSI string, or null.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoStartOutlookExpress(
    flags: u32,
    command: *const u8,
    show: i32,
) -> i32 {
    match unsafe { crate::mapi::ansi_input(command) } {
        Ok(text) => unsafe {
            CoStartOutlookExpressW(
                flags,
                mail_windows_platform::strings::wide(&text).as_ptr(),
                show,
            )
        },
        Err(_) => 0x80070057u32 as i32,
    }
}

/// # Safety
/// `command` must be a valid NUL-terminated UTF-16 string, or null.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoStartOutlookExpressW(
    _flags: u32,
    command: *const u16,
    show: i32,
) -> i32 {
    let result = std::panic::catch_unwind(|| {
        let command = if command.is_null() {
            String::new()
        } else {
            let mut n = 0;
            unsafe {
                while n < 32768 && *command.add(n) != 0 {
                    n += 1;
                }
            }
            unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(command, n)) }
        };
        crate::ui::run(&command, show)
    });
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(e)) => {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    null_mut(),
                    mail_windows_platform::strings::wide(&e).as_ptr(),
                    mail_windows_platform::strings::wide("Windows Mail").as_ptr(),
                    windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
                );
            }
            0x80004005u32 as i32
        }
        Err(_) => 0x80004005u32 as i32,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CoShutdownOutlookExpress() -> i32 {
    crate::ui::shutdown();
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> i32 {
    i32::from(crate::ui::active() || crate::mapi::busy())
}
/// # Safety
/// `out` must point to a writable COM interface pointer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    _class: *const windows_sys::core::GUID,
    _iid: *const windows_sys::core::GUID,
    out: *mut *mut std::ffi::c_void,
) -> i32 {
    if out.is_null() {
        return 0x80004003u32 as i32;
    }
    unsafe {
        *out = null_mut();
    }
    0x80040111u32 as i32
}
/// # Safety
/// `command` must be a valid NUL-terminated UTF-16 string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IdentifyW(command: *const u16) -> i32 {
    let result = std::panic::catch_unwind(|| {
        if command.is_null() {
            return;
        }
        let mut n = 0;
        unsafe {
            while n < 32768 && *command.add(n) != 0 {
                n += 1;
            }
        }
        let command = unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(command, n)) };
        mail_windows::startup::identify::run(&command);
    });
    if result.is_ok() {
        0
    } else {
        0x80004005u32 as i32
    }
}
