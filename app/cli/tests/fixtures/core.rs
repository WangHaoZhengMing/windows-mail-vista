use std::sync::atomic::{AtomicUsize, Ordering};
static CALLS: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoStartOutlookExpressW(
    flags: u32,
    command: *const u16,
    show: i32,
) -> i32 {
    let call = CALLS.fetch_add(1, Ordering::SeqCst);
    let mut len = 0;
    unsafe {
        while *command.add(len) != 0 {
            len += 1;
        }
    }
    let tail = unsafe { std::slice::from_raw_parts(command, len) };
    let log = std::env::var_os("WINMAIL_TEST_LOG").unwrap();
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    writeln!(file, "call={call};flags={flags};show={show};tail={tail:?}").unwrap();
    if call == 0 && std::env::var_os("WINMAIL_TEST_RESTART").is_some() {
        return 0x714;
    }
    std::env::var("WINMAIL_TEST_HRESULT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
