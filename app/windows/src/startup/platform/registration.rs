//! RVA 0x338c and 0x3624: register/unregister selected mail/news commands.
//! No registration is performed during normal launch or diagnostics.
use super::native::wide;
use std::ptr::null;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Shell::*;

#[derive(Clone)]
struct Value {
    kind: u32,
    bytes: Vec<u8>,
}
fn read(root: HKEY, path: &str, expand: bool) -> Option<Value> {
    let path = wide(path);
    let mut kind = 0;
    let mut bytes = vec![0u8; 520];
    let mut size = bytes.len() as u32;
    let mut error = if expand {
        unsafe {
            SHRegGetValueW(
                root,
                path.as_ptr(),
                null(),
                0x10000004,
                &mut kind,
                bytes.as_mut_ptr().cast(),
                &mut size,
            )
        }
    } else {
        1
    };
    if error != 0 {
        size = bytes.len() as u32;
        error = unsafe {
            SHGetValueW(
                root,
                path.as_ptr(),
                null(),
                &mut kind,
                bytes.as_mut_ptr().cast(),
                &mut size,
            )
        };
    }
    if error != 0 {
        return None;
    }
    bytes.truncate(size as usize);
    Some(Value { kind, bytes })
}
fn text(value: &Value) -> String {
    let words: Vec<u16> = value
        .bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    String::from_utf16_lossy(&words)
}
fn set(root: HKEY, path: &str, value: &Value) -> bool {
    unsafe {
        SHSetValueW(
            root,
            wide(path).as_ptr(),
            null(),
            value.kind,
            value.bytes.as_ptr().cast(),
            value.bytes.len() as u32,
        ) == 0
    }
}
fn delete(root: HKEY, path: &str) -> bool {
    unsafe { SHDeleteKeyW(root, wide(path).as_ptr()) == 0 }
}

pub fn update(register: bool) {
    update_in(register, HKEY_LOCAL_MACHINE, HKEY_CLASSES_ROOT);
}

fn update_in(register: bool, machine: HKEY, classes: HKEY) {
    if read(machine, r"Software\Clients\Mail", false)
        .is_some_and(|v| text(&v).eq_ignore_ascii_case("Windows Mail"))
    {
        let target = r"mailto\shell\open";
        if register {
            if let Some(v) = read(
                machine,
                r"Software\Clients\Mail\Windows Mail\Protocols\mailto\shell\open\command",
                true,
            ) {
                set(classes, &format!(r"{target}\command"), &v);
            }
        } else {
            delete(classes, target);
        }
    }
    if !read(machine, r"Software\Clients\News", false)
        .is_some_and(|v| text(&v).eq_ignore_ascii_case("Windows Mail"))
    {
        return;
    }
    let previous: Vec<_> = ["news", "nntp", "snews"]
        .iter()
        .map(|s| {
            let path = format!(r"{s}\shell\open\command");
            (path.clone(), read(classes, &path, true))
        })
        .collect();
    for protocol in ["news", "nntp", "snews"] {
        let target = format!(r"{protocol}\shell\open");
        let succeeded = if register {
            let template =
                format!(r"Software\Clients\News\Windows Mail\Protocols\{target}\command");
            read(machine, &template, true)
                .is_some_and(|v| set(classes, &format!(r"{target}\command"), &v))
        } else {
            delete(classes, &target)
        };
        if !succeeded {
            // RVA 0x3a84 onward restores previous command values after failure.
            for (path, value) in &previous {
                if let Some(value) = value {
                    set(classes, path, value);
                }
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null_mut;

    struct Sandbox {
        path: String,
        handle: HKEY,
    }
    impl Sandbox {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = format!(r"Software\WinMailRustTests\{}-{nonce}", std::process::id());
            let mut handle = null_mut();
            assert_eq!(
                unsafe {
                    RegCreateKeyExW(
                        HKEY_CURRENT_USER,
                        wide(&path).as_ptr(),
                        0,
                        null(),
                        0,
                        KEY_ALL_ACCESS,
                        null(),
                        &mut handle,
                        null_mut(),
                    )
                },
                0
            );
            Self { path, handle }
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.handle);
                RegDeleteTreeW(HKEY_CURRENT_USER, wide(&self.path).as_ptr());
            }
        }
    }
    fn string(s: &str) -> Value {
        Value {
            kind: REG_SZ,
            bytes: wide(s).iter().flat_map(|c| c.to_le_bytes()).collect(),
        }
    }

    #[test]
    fn protocol_registration_uses_templates_and_preserves_other_clients() {
        // Pass temporary HKCU handles; never touch HKLM, HKCR or real associations.
        let machine = Sandbox::new();
        let classes = Sandbox::new();
        let command = r#""C:\Program Files\Windows Mail\WinMail.exe" "%1""#;
        set(
            machine.handle,
            r"Software\Clients\Mail",
            &string("Other Client"),
        );
        set(
            machine.handle,
            r"Software\Clients\News",
            &string("Other Client"),
        );
        for (category, protocols) in [
            ("Mail", &["mailto"][..]),
            ("News", &["news", "nntp", "snews"][..]),
        ] {
            for protocol in protocols {
                set(
                    machine.handle,
                    &format!(
                        r"Software\Clients\{category}\Windows Mail\Protocols\{protocol}\shell\open\command"
                    ),
                    &string(command),
                );
                set(
                    classes.handle,
                    &format!(r"{protocol}\shell\open\command"),
                    &string("original"),
                );
            }
        }
        update_in(true, machine.handle, classes.handle);
        assert_eq!(
            text(&read(classes.handle, r"mailto\shell\open\command", false).unwrap()),
            "original"
        );
        set(
            machine.handle,
            r"Software\Clients\Mail",
            &string("Windows Mail"),
        );
        set(
            machine.handle,
            r"Software\Clients\News",
            &string("Windows Mail"),
        );
        update_in(true, machine.handle, classes.handle);
        for protocol in ["mailto", "news", "nntp", "snews"] {
            assert_eq!(
                text(
                    &read(
                        classes.handle,
                        &format!(r"{protocol}\shell\open\command"),
                        false
                    )
                    .unwrap()
                ),
                command
            );
        }
        update_in(false, machine.handle, classes.handle);
        for protocol in ["mailto", "news", "nntp", "snews"] {
            assert!(
                read(
                    classes.handle,
                    &format!(r"{protocol}\shell\open\command"),
                    false
                )
                .is_none()
            );
        }
    }
}
