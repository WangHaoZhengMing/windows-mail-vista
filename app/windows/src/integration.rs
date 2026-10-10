//! Default-app registration. Only explicit setup/settings actions write the registry.
use crate::{Result, platform::strings::wide};
use std::{
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{System::Registry::*, UI::Shell::*};
pub const APP_NAME: &str = "Windows Mail Rust";
const APP_KEY: &str = r"Software\WindowsMailRust";
const MAIL_KEY: &str = r"Software\Clients\Mail\Windows Mail Rust";

fn set(root: HKEY, path: &str, name: &str, value: &str) -> Result<()> {
    set_view(root, path, name, value, 0)
}
fn set_view(root: HKEY, path: &str, name: &str, value: &str, view: u32) -> Result<()> {
    unsafe {
        let mut key = null_mut();
        let error = RegCreateKeyExW(
            root,
            wide(path).as_ptr(),
            0,
            null(),
            0,
            KEY_SET_VALUE | view,
            null(),
            &mut key,
            null_mut(),
        );
        if error != 0 {
            return Err(format!("Registry access failed ({error}): {path}"));
        }
        let data = wide(value);
        let error = RegSetValueExW(
            key,
            wide(name).as_ptr(),
            0,
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        );
        RegCloseKey(key);
        if error != 0 {
            return Err(format!("Registry write failed ({error}): {path}"));
        }
        Ok(())
    }
}
fn entries(executable: &Path) -> Result<Vec<(String, String, String)>> {
    let exe = executable
        .to_str()
        .ok_or("Installation path must be Unicode")?;
    if exe.contains(['"', '\r', '\n']) {
        return Err("Invalid installation path".into());
    }
    let mut result = Vec::new();
    let mut add =
        |path: String, name: &str, value: String| result.push((path, name.to_owned(), value));
    add(
        format!(r"{APP_KEY}\Capabilities"),
        "ApplicationName",
        APP_NAME.into(),
    );
    add(
        format!(r"{APP_KEY}\Capabilities"),
        "ApplicationDescription",
        "Rust desktop mail client: IMAP, POP3 and SMTP".into(),
    );
    add(
        format!(r"{APP_KEY}\Capabilities"),
        "ApplicationIcon",
        format!("\"{exe}\",0"),
    );
    add(
        format!(r"{APP_KEY}\Capabilities\URLAssociations"),
        "mailto",
        "WindowsMailRust.Mailto".into(),
    );
    add(
        format!(r"{APP_KEY}\Capabilities\FileAssociations"),
        ".eml",
        "WindowsMailRust.EML".into(),
    );
    add(
        format!(r"{APP_KEY}\Capabilities\Startmenu"),
        "Mail",
        APP_NAME.into(),
    );
    add(
        r"Software\RegisteredApplications".into(),
        APP_NAME,
        format!(r"{APP_KEY}\Capabilities"),
    );
    add(APP_KEY.into(), "Executable", exe.into());
    for (progid, title, protocol) in [
        ("WindowsMailRust.Mailto", "Windows Mail URL", true),
        ("WindowsMailRust.EML", "Internet Mail Message", false),
    ] {
        let path = format!(r"Software\Classes\{progid}");
        add(path.clone(), "", title.into());
        if protocol {
            add(path.clone(), "URL Protocol", String::new());
        }
        add(format!(r"{path}\DefaultIcon"), "", format!("\"{exe}\",0"));
        add(
            format!(r"{path}\shell\open\command"),
            "",
            format!("\"{exe}\" \"%1\""),
        );
    }
    add(
        r"Software\Classes\.eml\OpenWithProgids".into(),
        "WindowsMailRust.EML",
        String::new(),
    );
    add(MAIL_KEY.into(), "", APP_NAME.into());
    add(
        format!(r"{MAIL_KEY}\shell\open\command"),
        "",
        format!("\"{exe}\""),
    );
    add(
        format!(r"{MAIL_KEY}\Protocols\mailto\shell\open\command"),
        "",
        format!("\"{exe}\" \"%1\""),
    );
    Ok(result)
}
pub fn register() -> Result<()> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    for (path, name, value) in entries(&exe)? {
        set(HKEY_CURRENT_USER, &path, &name, &value)?;
    }
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null());
    }
    Ok(())
}
pub fn unregister() -> Result<()> {
    unsafe {
        let mut key = null_mut();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide(r"Software\RegisteredApplications").as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        ) == 0
        {
            RegDeleteValueW(key, wide(APP_NAME).as_ptr());
            RegCloseKey(key);
        }
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide(r"Software\Classes\.eml\OpenWithProgids").as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        ) == 0
        {
            RegDeleteValueW(key, wide("WindowsMailRust.EML").as_ptr());
            RegCloseKey(key);
        }
        for path in [
            APP_KEY,
            MAIL_KEY,
            r"Software\Classes\WindowsMailRust.Mailto",
            r"Software\Classes\WindowsMailRust.EML",
        ] {
            let error = RegDeleteTreeW(HKEY_CURRENT_USER, wide(path).as_ptr());
            if error != 0 && error != 2 {
                return Err(format!("Registry removal failed ({error}): {path}"));
            }
        }
        SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null());
    }
    Ok(())
}
pub fn open_defaults() -> Result<()> {
    register()?;
    let uri = "ms-settings:defaultapps?registeredAppUser=Windows%20Mail%20Rust";
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            wide("open").as_ptr(),
            wide(uri).as_ptr(),
            null(),
            null(),
            1,
        )
    } as isize;
    if result <= 32 {
        Err(format!("Cannot open Default apps settings ({result})"))
    } else {
        Ok(())
    }
}
fn read_view(root: HKEY, path: &str, name: &str, view: u32) -> Option<String> {
    unsafe {
        let mut key = null_mut();
        if RegOpenKeyExW(
            root,
            wide(path).as_ptr(),
            0,
            KEY_QUERY_VALUE | view,
            &mut key,
        ) != 0
        {
            return None;
        }
        let mut data = vec![0u16; 32768];
        let mut size = (data.len() * 2) as u32;
        let mut kind = 0;
        let error = RegQueryValueExW(
            key,
            wide(name).as_ptr(),
            null(),
            &mut kind,
            data.as_mut_ptr().cast(),
            &mut size,
        );
        RegCloseKey(key);
        if error != 0 || kind != REG_SZ {
            return None;
        }
        let end = data.iter().position(|&c| c == 0).unwrap_or(data.len());
        String::from_utf16(&data[..end]).ok()
    }
}
pub fn register_mapi() -> Result<()> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dll = exe.with_file_name("msoe.dll");
    if !dll.is_file() {
        return Err("Build and install msoe.dll beside WinMail.exe first".into());
    }
    let dll32 = exe
        .parent()
        .ok_or("Missing installation directory")?
        .join("x86/msoe.dll");
    for (view, dll) in [(KEY_WOW64_64KEY, dll), (KEY_WOW64_32KEY, dll32)] {
        if !dll.is_file() {
            continue;
        }
        let previous =
            read_view(HKEY_LOCAL_MACHINE, r"Software\Clients\Mail", "", view).unwrap_or_default();
        if previous != APP_NAME {
            set_view(
                HKEY_LOCAL_MACHINE,
                MAIL_KEY,
                "PreviousClient",
                &previous,
                view,
            )?;
        }
        set_view(HKEY_LOCAL_MACHINE, MAIL_KEY, "", APP_NAME, view)?;
        set_view(
            HKEY_LOCAL_MACHINE,
            MAIL_KEY,
            "DLLPath",
            &dll.to_string_lossy(),
            view,
        )?;
        set_view(
            HKEY_LOCAL_MACHINE,
            MAIL_KEY,
            "Executable",
            &exe.to_string_lossy(),
            view,
        )?;
        set_view(
            HKEY_LOCAL_MACHINE,
            r"Software\Clients\Mail",
            "",
            APP_NAME,
            view,
        )?;
    }
    Ok(())
}
pub fn unregister_mapi() -> Result<()> {
    for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
        if read_view(HKEY_LOCAL_MACHINE, r"Software\Clients\Mail", "", view).as_deref()
            == Some(APP_NAME)
        {
            let previous =
                read_view(HKEY_LOCAL_MACHINE, MAIL_KEY, "PreviousClient", view).unwrap_or_default();
            set_view(
                HKEY_LOCAL_MACHINE,
                r"Software\Clients\Mail",
                "",
                &previous,
                view,
            )?;
        }
        unsafe {
            let mut parent = null_mut();
            let error = RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                wide(r"Software\Clients\Mail").as_ptr(),
                0,
                KEY_ALL_ACCESS | view,
                &mut parent,
            );
            if error == 2 {
                continue;
            }
            if error != 0 {
                return Err(format!(
                    "MAPI registry access failed ({error}); run elevated"
                ));
            }
            let error = RegDeleteTreeW(parent, wide(APP_NAME).as_ptr());
            RegCloseKey(parent);
            if error != 0 && error != 2 {
                return Err(format!("MAPI registry removal failed ({error})"));
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_registration_writes_capabilities_in_an_isolated_registry_tree() {
        unsafe {
            let path = format!(
                r"Software\WinMailRustRegistrationTest-{}",
                std::process::id()
            );
            let mut root = null_mut();
            assert_eq!(
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    wide(&path).as_ptr(),
                    0,
                    null(),
                    0,
                    KEY_ALL_ACCESS,
                    null(),
                    &mut root,
                    null_mut()
                ),
                0
            );
            for (key, name, value) in entries(Path::new(r"C:\Mail\WinMail.exe")).unwrap() {
                set(root, &key, &name, &value).unwrap();
            }
            assert_eq!(
                read_view(
                    root,
                    r"Software\WindowsMailRust\Capabilities\URLAssociations",
                    "mailto",
                    0
                )
                .as_deref(),
                Some("WindowsMailRust.Mailto")
            );
            assert_eq!(
                read_view(
                    root,
                    r"Software\Classes\WindowsMailRust.EML\shell\open\command",
                    "",
                    0
                )
                .as_deref(),
                Some(r#""C:\Mail\WinMail.exe" "%1""#)
            );
            RegCloseKey(root);
            assert_eq!(RegDeleteTreeW(HKEY_CURRENT_USER, wide(&path).as_ptr()), 0);
        }
    }
    #[test]
    fn associations_quote_paths_and_do_not_force_user_choice() {
        let rows = entries(Path::new(r"C:\Program Files\Mail\WinMail.exe")).unwrap();
        assert!(
            rows.iter()
                .any(|(p, _, v)| p.ends_with(r"Mailto\shell\open\command")
                    && v == r#""C:\Program Files\Mail\WinMail.exe" "%1""#)
        );
        assert!(rows.iter().all(|(p, _, _)| !p.contains("UserChoice")));
        assert!(entries(Path::new("bad\"path.exe")).is_err());
    }
}
