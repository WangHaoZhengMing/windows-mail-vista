//! Windows application: native UI and startup, composed from shared mail logic.
//! The optional compatibility DLL adapts external callers to this application.

#[cfg(windows)]
use mail_app as app;
#[cfg(windows)]
use mail_core::{account, message, store};
#[cfg(windows)]
use mail_windows_platform as platform;

#[cfg(windows)]
pub type Result<T> = mail_core::Result<T>;

#[cfg(windows)]
pub mod startup;
#[cfg(windows)]
pub mod ui;

#[cfg(windows)]
pub mod integration;
#[cfg(windows)]
pub mod mapi_bridge;

/// Instance and window-class names shared by the launcher and UI.
#[cfg(windows)]
pub fn instance_names_for(key: &str) -> (String, String) {
    platform::ipc::instance_names_for(key)
}

#[cfg(windows)]
pub fn instance_names() -> (String, String) {
    let key = std::env::var("WINMAIL_INSTANCE_ID").unwrap_or_else(|_| {
        mail_core::store::default_root()
            .to_string_lossy()
            .to_lowercase()
    });
    instance_names_for(&key)
}
