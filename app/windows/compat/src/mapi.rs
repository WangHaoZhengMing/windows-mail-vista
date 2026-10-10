//! Simple MAPI adapter for this runtime's local store, not Extended MAPI or BMAPI.
//! ANSI strings use the Windows active code page. A session holds the store lock.
//! External callers must supply valid ABI pointers, arrays and output buffers;
//! input memory is borrowed only for the duration of the call. Returned objects
//! belong to the runtime and must be released with this DLL's MAPIFreeBuffer.
use crate::{
    message::{self, Draft},
    store::{Store, default_root},
};
use std::{
    collections::HashMap,
    ffi::c_void,
    ptr::{null, null_mut},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
use windows_sys::Win32::{
    Globalization::*,
    System::{Mapi::*, Memory::*},
};

type MResult<T> = std::result::Result<T, u32>;
static SESSIONS: OnceLock<Mutex<HashMap<usize, Store>>> = OnceLock::new();
static NEXT: AtomicUsize = AtomicUsize::new(1);
struct Allocation {
    temp: Option<std::path::PathBuf>,
}
static BUFFERS: OnceLock<Mutex<HashMap<usize, Allocation>>> = OnceLock::new();
fn buffers() -> &'static Mutex<HashMap<usize, Allocation>> {
    BUFFERS.get_or_init(Default::default)
}
pub fn busy() -> bool {
    let sessions = SESSIONS
        .get_or_init(Default::default)
        .lock()
        .map(|s| !s.is_empty())
        .unwrap_or(true);
    let buffers = buffers().lock().map(|s| !s.is_empty()).unwrap_or(true);
    sessions || buffers
}
fn guarded(f: impl FnOnce() -> MResult<()>) -> u32 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(())) => SUCCESS_SUCCESS,
        Ok(Err(e)) => e,
        Err(_) => MAPI_E_FAILURE,
    }
}
fn with_store<T>(session: usize, f: impl FnOnce(&mut Store) -> MResult<T>) -> MResult<T> {
    if session == 0 {
        let mut store = Store::open(&default_root()).map_err(|_| MAPI_E_MESSAGE_IN_USE)?;
        f(&mut store)
    } else {
        let mut sessions = SESSIONS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| MAPI_E_FAILURE)?;
        f(sessions.get_mut(&session).ok_or(MAPI_E_INVALID_SESSION)?)
    }
}
/// # Safety
/// Pointer must be null or a readable, NUL-terminated ANSI string.
pub unsafe fn ansi_input(pointer: *const u8) -> crate::Result<String> {
    if pointer.is_null() {
        return Ok(String::new());
    }
    let mut length = 0;
    unsafe {
        while length < crate::MAX_MESSAGE && *pointer.add(length) != 0 {
            length += 1;
        }
    }
    if length == crate::MAX_MESSAGE {
        return Err("Unterminated ANSI string".into());
    }
    if length == 0 {
        return Ok(String::new());
    }
    let size = unsafe { MultiByteToWideChar(CP_ACP, 0, pointer, length as i32, null_mut(), 0) };
    if size == 0 {
        return Err("ANSI conversion failed".into());
    }
    let mut wide = vec![0; size as usize];
    unsafe {
        MultiByteToWideChar(CP_ACP, 0, pointer, length as i32, wide.as_mut_ptr(), size);
    }
    String::from_utf16(&wide).map_err(|e| e.to_string())
}
fn ansi_output(text: &str) -> Vec<u8> {
    let wide: Vec<u16> = text.replace('\0', " ").encode_utf16().collect();
    let size = unsafe {
        WideCharToMultiByte(
            CP_ACP,
            0,
            wide.as_ptr(),
            wide.len() as i32,
            null_mut(),
            0,
            null(),
            null_mut(),
        )
    };
    let mut out = vec![0; size.max(0) as usize + 1];
    if size > 0 {
        unsafe {
            WideCharToMultiByte(
                CP_ACP,
                0,
                wide.as_ptr(),
                wide.len() as i32,
                out.as_mut_ptr(),
                size,
                null(),
                null_mut(),
            );
        }
    }
    out
}
unsafe fn input(pointer: *const u8) -> MResult<String> {
    unsafe { ansi_input(pointer) }.map_err(|_| MAPI_E_INVALID_MESSAGE)
}
fn resolve_id(store: &Store, id: &str) -> MResult<String> {
    if id.is_empty() {
        return Err(MAPI_E_INVALID_MESSAGE);
    }
    let found: Vec<_> = store
        .entries()
        .iter()
        .filter(|e| e.id == id || (id.len() == 31 && e.id.starts_with(id)))
        .collect();
    if found.len() != 1 {
        return Err(MAPI_E_INVALID_MESSAGE);
    }
    Ok(found[0].id.clone())
}
unsafe fn put_id(pointer: *mut u8, id: &str) -> MResult<()> {
    if pointer.is_null() {
        return Err(MAPI_E_INVALID_MESSAGE);
    }
    // 31 ASCII characters plus NUL fit both short and long MAPI ID buffers.
    unsafe {
        std::ptr::copy_nonoverlapping(id.as_ptr(), pointer, 31);
        *pointer.add(31) = 0;
    }
    Ok(())
}
unsafe fn draft_from(pointer: *const MapiMessage) -> MResult<Draft> {
    if pointer.is_null() {
        return Err(MAPI_E_INVALID_MESSAGE);
    }
    let mail = unsafe { &*pointer };
    if mail.nRecipCount > 1000 || mail.nFileCount > 100 {
        return Err(MAPI_E_TOO_MANY_RECIPIENTS);
    }
    if (mail.nRecipCount > 0 && mail.lpRecips.is_null())
        || (mail.nFileCount > 0 && mail.lpFiles.is_null())
    {
        return Err(MAPI_E_INVALID_MESSAGE);
    }
    let mut draft = Draft {
        subject: unsafe { input(mail.lpszSubject)? },
        body: unsafe { input(mail.lpszNoteText)? },
        ..Draft::default()
    };
    for i in 0..mail.nRecipCount as usize {
        let recipient = unsafe { &*mail.lpRecips.add(i) };
        let address = unsafe { input(recipient.lpszAddress)? };
        let address = if address.is_empty() {
            unsafe { input(recipient.lpszName)? }
        } else {
            address
        };
        let address = if address
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("SMTP:"))
        {
            &address[5..]
        } else {
            &address
        };
        message::addresses(address).map_err(|_| MAPI_E_UNKNOWN_RECIPIENT)?;
        let field = match recipient.ulRecipClass {
            MAPI_TO => &mut draft.to,
            MAPI_CC => &mut draft.cc,
            MAPI_BCC => &mut draft.bcc,
            _ => return Err(MAPI_E_BAD_RECIPTYPE),
        };
        if !field.is_empty() {
            field.push(',');
        }
        field.push_str(address);
    }
    for i in 0..mail.nFileCount as usize {
        let file = unsafe { &*mail.lpFiles.add(i) };
        draft
            .attachments
            .push(unsafe { input(file.lpszPathName)? }.into());
    }
    Ok(draft)
}

/// # Safety
/// `out` must point to writable session storage; profile/password are valid ANSI strings.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPILogon(
    _ui: usize,
    profile: *const u8,
    password: *const u8,
    flags: u32,
    _reserved: u32,
    out: *mut usize,
) -> u32 {
    guarded(|| {
        if out.is_null() {
            return Err(MAPI_E_INVALID_SESSION);
        }
        unsafe {
            *out = 0;
        }
        if flags & MAPI_EXTENDED != 0 {
            return Err(MAPI_E_NOT_SUPPORTED);
        }
        if !unsafe { input(profile)? }.is_empty() || !unsafe { input(password)? }.is_empty() {
            return Err(MAPI_E_NOT_SUPPORTED);
        }
        let store = Store::open(&default_root()).map_err(|_| MAPI_E_MESSAGE_IN_USE)?;
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        SESSIONS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| MAPI_E_FAILURE)?
            .insert(id, store);
        unsafe {
            *out = id;
        }
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn MAPILogoff(session: usize, _ui: usize, _flags: u32, _reserved: u32) -> u32 {
    guarded(|| {
        SESSIONS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| MAPI_E_FAILURE)?
            .remove(&session)
            .ok_or(MAPI_E_INVALID_SESSION)?;
        Ok(())
    })
}
/// # Safety
/// Strings must be valid ANSI strings; `out` must have room for at least 32 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPIFindNext(
    session: usize,
    _ui: usize,
    kind: *const u8,
    seed: *const u8,
    flags: u32,
    _reserved: u32,
    out: *mut u8,
) -> u32 {
    guarded(|| {
        if out.is_null() {
            return Err(MAPI_E_INVALID_MESSAGE);
        }
        unsafe {
            *out = 0;
        }
        if !unsafe { input(kind)? }.is_empty() {
            return Err(MAPI_E_TYPE_NOT_SUPPORTED);
        }
        let seed = unsafe { input(seed)? };
        with_store(session, |store| {
            let mut start = 0;
            if !seed.is_empty() {
                let full = resolve_id(store, &seed)?;
                start = store
                    .entries()
                    .iter()
                    .position(|e| e.id == full)
                    .ok_or(MAPI_E_INVALID_MESSAGE)?
                    + 1;
            }
            let entry = store.entries()[start..]
                .iter()
                .find(|e| e.folder == "Inbox" && (flags & MAPI_UNREAD_ONLY == 0 || !e.read))
                .ok_or(MAPI_E_NO_MESSAGES)?;
            unsafe { put_id(out, &entry.id) }
        })
    })
}
/// # Safety
/// `id` must be a valid ANSI message ID.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPIDeleteMail(
    session: usize,
    _ui: usize,
    id: *const u8,
    _flags: u32,
    _reserved: u32,
) -> u32 {
    guarded(|| {
        let id = unsafe { input(id)? };
        with_store(session, |store| {
            let id = resolve_id(store, &id)?;
            store.move_to(&id, "Trash").map_err(|_| MAPI_E_FAILURE)
        })
    })
}
/// # Safety
/// `message` and its arrays/strings must be valid; `out` needs at least 32 writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPISaveMail(
    session: usize,
    _ui: usize,
    message: *mut MapiMessage,
    flags: u32,
    _reserved: u32,
    out: *mut u8,
) -> u32 {
    guarded(|| {
        if out.is_null() {
            return Err(MAPI_E_INVALID_MESSAGE);
        }
        if flags & MAPI_FORCE_UNICODE != 0 {
            return Err(MAPI_E_UNICODE_NOT_SUPPORTED);
        }
        let draft = unsafe { draft_from(message)? };
        with_store(session, |store| {
            let sender = store
                .account()
                .map(|a| a.email.clone())
                .unwrap_or_else(|| "draft@winmail-rust.local".into());
            let id = store
                .save_draft(&sender, &draft)
                .map_err(|_| MAPI_E_FAILURE)?;
            unsafe { put_id(out, &id) }
        })
    })
}
/// # Safety
/// `message` and its arrays/strings must remain readable during this call.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPISendMail(
    session: usize,
    _ui: usize,
    message: *mut MapiMessage,
    flags: u32,
    _reserved: u32,
) -> u32 {
    guarded(|| {
        // UI send dialogs require their own contract; don't silently send when asked for one.
        if flags & MAPI_DIALOG != 0 {
            return Err(MAPI_E_NOT_SUPPORTED);
        }
        if flags & MAPI_FORCE_UNICODE != 0 {
            return Err(MAPI_E_UNICODE_NOT_SUPPORTED);
        }
        let draft = unsafe { draft_from(message)? };
        with_store(session, |store| {
            let mut account = store.account().cloned().ok_or(MAPI_E_LOGON_FAILURE)?;
            if matches!(account.auth, mail_core::account::Authentication::OAuth2(_)) {
                mail_core::oauth::access_token(&mut account).map_err(|_| MAPI_E_LOGON_FAILURE)?;
                let account_id = store
                    .default_account_id()
                    .ok_or(MAPI_E_LOGON_FAILURE)?
                    .clone();
                store
                    .update_account(&account_id, account.clone())
                    .map_err(|_| MAPI_E_FAILURE)?;
            }
            let raw = draft
                .build(&account.email)
                .map_err(|_| MAPI_E_INVALID_MESSAGE)?;
            let id = store
                .import(&raw, "Outbox", None)
                .map_err(|_| MAPI_E_FAILURE)?;
            crate::network::smtp_send(&account, &draft, &raw)
                .map_err(|_| MAPI_E_NETWORK_FAILURE)?;
            store.move_to(&id, "Sent").map_err(|_| MAPI_E_FAILURE)
        })
    })
}

struct Blob {
    bytes: Vec<u8>,
}
impl Blob {
    fn new(size: usize) -> Self {
        Self {
            bytes: vec![0; size],
        }
    }
    fn text(&mut self, text: &str) -> usize {
        let offset = self.bytes.len();
        self.bytes.extend(ansi_output(text));
        offset
    }
    fn allocate(self, temp: Option<std::path::PathBuf>) -> MResult<*mut u8> {
        let mut registry = buffers().lock().map_err(|_| MAPI_E_FAILURE)?;
        let pointer =
            unsafe { HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, self.bytes.len()) }.cast::<u8>();
        if pointer.is_null() {
            return Err(MAPI_E_INSUFFICIENT_MEMORY);
        }
        unsafe {
            std::ptr::copy_nonoverlapping(self.bytes.as_ptr(), pointer, self.bytes.len());
        }
        registry.insert(pointer as usize, Allocation { temp });
        Ok(pointer)
    }
}
/// # Safety
/// `name` is a readable ANSI string and `out` points to writable pointer storage.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPIResolveName(
    session: usize,
    _ui: usize,
    name: *const u8,
    _flags: u32,
    _reserved: u32,
    out: *mut *mut MapiRecipDesc,
) -> u32 {
    guarded(|| {
        if out.is_null() {
            return Err(MAPI_E_INVALID_RECIPS);
        }
        unsafe {
            *out = null_mut();
        }
        if session != 0 {
            with_store(session, |_| Ok(()))?;
        }
        let name = unsafe { input(name)? };
        let stripped = if name
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("SMTP:"))
        {
            &name[5..]
        } else {
            &name
        };
        let addresses = message::addresses(stripped).map_err(|_| MAPI_E_UNKNOWN_RECIPIENT)?;
        if addresses.len() != 1 {
            return Err(MAPI_E_AMBIGUOUS_RECIPIENT);
        }
        let mut blob = Blob::new(std::mem::size_of::<MapiRecipDesc>());
        let display = blob.text(&name);
        let address = blob.text(&format!("SMTP:{}", addresses[0]));
        let pointer = blob.allocate(None)?;
        let recipient = MapiRecipDesc {
            ulRecipClass: MAPI_TO,
            lpszName: unsafe { pointer.add(display) },
            lpszAddress: unsafe { pointer.add(address) },
            ..MapiRecipDesc::default()
        };
        unsafe {
            pointer.cast::<MapiRecipDesc>().write(recipient);
            *out = pointer.cast();
        }
        Ok(())
    })
}
/// # Safety
/// `id` is a readable ANSI string; `out` points to writable pointer storage.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPIReadMail(
    session: usize,
    _ui: usize,
    id: *const u8,
    flags: u32,
    _reserved: u32,
    out: *mut *mut MapiMessage,
) -> u32 {
    guarded(|| {
        if out.is_null() {
            return Err(MAPI_E_INVALID_MESSAGE);
        }
        unsafe {
            *out = null_mut();
        }
        if flags & MAPI_BODY_AS_FILE != 0 {
            return Err(MAPI_E_NOT_SUPPORTED);
        }
        if flags & MAPI_FORCE_UNICODE != 0 {
            return Err(MAPI_E_UNICODE_NOT_SUPPORTED);
        }
        let id = unsafe { input(id)? };
        with_store(session, |store| {
            let id = resolve_id(store, &id)?;
            let raw = store.raw(&id).map_err(|_| MAPI_E_INVALID_MESSAGE)?;
            let parsed = message::parse(&raw).map_err(|_| MAPI_E_INVALID_MESSAGE)?;
            let envelope = flags & MAPI_ENVELOPE_ONLY != 0;
            let count = if envelope || flags & MAPI_SUPPRESS_ATTACH != 0 {
                0
            } else {
                parsed.attachments.len()
            };
            let mut recipients: Vec<_> = message::addresses(&parsed.summary.to)
                .unwrap_or_default()
                .into_iter()
                .map(|r| (MAPI_TO, r))
                .collect();
            recipients.extend(
                message::addresses(&parsed.summary.cc)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|r| (MAPI_CC, r)),
            );
            let recip_offset = std::mem::size_of::<MapiMessage>();
            let file_offset =
                recip_offset + (recipients.len() + 1) * std::mem::size_of::<MapiRecipDesc>();
            let mut blob = Blob::new(file_offset + count * std::mem::size_of::<MapiFileDesc>());
            let subject = blob.text(&parsed.summary.subject);
            let body = blob.text(if envelope { "" } else { &parsed.body });
            let date = blob.text(&parsed.summary.date);
            let from = blob.text(&parsed.summary.from);
            let from_address = blob.text(&format!(
                "SMTP:{}",
                message::addresses(&parsed.summary.from)
                    .ok()
                    .and_then(|v| v.into_iter().next())
                    .unwrap_or_default()
            ));
            let recip_strings: Vec<_> = recipients
                .iter()
                .map(|(class, r)| (*class, blob.text(r), blob.text(&format!("SMTP:{r}"))))
                .collect();
            let mut files = Vec::new();
            let temp = if count > 0 {
                let nonce = NEXT.fetch_add(1, Ordering::SeqCst);
                let directory = std::env::temp_dir()
                    .join(format!("winmail-mapi-{}-{nonce}", std::process::id()));
                std::fs::create_dir(&directory).map_err(|_| MAPI_E_ATTACHMENT_WRITE_FAILURE)?;
                let result = (|| {
                    for (i, attachment) in parsed.attachments.iter().take(count).enumerate() {
                        let path = directory.join(format!("{i}-{}", attachment.name));
                        crate::store::atomic_write(&path, &attachment.bytes)
                            .map_err(|_| MAPI_E_ATTACHMENT_WRITE_FAILURE)?;
                        files.push((
                            blob.text(&path.to_string_lossy()),
                            blob.text(&attachment.name),
                        ));
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    let _ = std::fs::remove_dir_all(&directory);
                    return Err(error);
                }
                Some(directory)
            } else {
                None
            };
            // Marking read is committed before handing ownership to the caller.
            if flags & MAPI_PEEK == 0 && store.mark_read(&id).is_err() {
                if let Some(dir) = &temp {
                    let _ = std::fs::remove_dir_all(dir);
                }
                return Err(MAPI_E_FAILURE);
            }
            let pointer = match blob.allocate(temp.clone()) {
                Ok(p) => p,
                Err(e) => {
                    if let Some(dir) = temp {
                        let _ = std::fs::remove_dir_all(dir);
                    }
                    return Err(e);
                }
            };
            unsafe {
                let recips = pointer.add(recip_offset).cast::<MapiRecipDesc>();
                recips.write(MapiRecipDesc {
                    ulRecipClass: MAPI_ORIG,
                    lpszName: pointer.add(from),
                    lpszAddress: pointer.add(from_address),
                    ..MapiRecipDesc::default()
                });
                for (i, (class, name, address)) in recip_strings.iter().enumerate() {
                    recips.add(i + 1).write(MapiRecipDesc {
                        ulRecipClass: *class,
                        lpszName: pointer.add(*name),
                        lpszAddress: pointer.add(*address),
                        ..MapiRecipDesc::default()
                    });
                }
                let attachments = pointer.add(file_offset).cast::<MapiFileDesc>();
                for (i, (path, name)) in files.iter().enumerate() {
                    attachments.add(i).write(MapiFileDesc {
                        nPosition: u32::MAX,
                        lpszPathName: pointer.add(*path),
                        lpszFileName: pointer.add(*name),
                        ..MapiFileDesc::default()
                    });
                }
                pointer.cast::<MapiMessage>().write(MapiMessage {
                    lpszSubject: pointer.add(subject),
                    lpszNoteText: pointer.add(body),
                    lpszDateReceived: pointer.add(date),
                    flFlags: if store.entries().iter().any(|e| e.id == id && !e.read) {
                        MAPI_UNREAD
                    } else {
                        0
                    },
                    lpOriginator: recips,
                    nRecipCount: recipients.len() as u32,
                    lpRecips: if recipients.is_empty() {
                        null_mut()
                    } else {
                        recips.add(1)
                    },
                    nFileCount: count as u32,
                    lpFiles: if count == 0 { null_mut() } else { attachments },
                    ..MapiMessage::default()
                });
                *out = pointer.cast();
            }
            Ok(())
        })
    })
}
/// # Safety
/// Pointer must be an unreleased allocation returned by this runtime, or null.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MAPIFreeBuffer(pointer: *mut c_void) -> u32 {
    guarded(|| {
        if pointer.is_null() {
            return Ok(());
        }
        let allocation = buffers()
            .lock()
            .map_err(|_| MAPI_E_FAILURE)?
            .remove(&(pointer as usize))
            .ok_or(MAPI_E_INVALID_MESSAGE)?;
        if unsafe { HeapFree(GetProcessHeap(), 0, pointer) } == 0 {
            return Err(MAPI_E_FAILURE);
        }
        if let Some(directory) = allocation.temp {
            let _ = std::fs::remove_dir_all(directory);
        }
        Ok(())
    })
}
