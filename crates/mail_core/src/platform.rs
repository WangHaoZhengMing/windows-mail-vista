//! Select operating-system services without exposing native APIs to mail code.
//! The implementations belong to each platform's package under `app/`.

#[cfg(windows)]
pub(crate) use mail_windows_platform::{
    default_data_dir, open_store_lock, protect_secret, unprotect_secret,
};

#[cfg(target_os = "macos")]
pub(crate) use mail_macos_platform::{
    default_data_dir, open_store_lock, protect_secret, unprotect_secret,
};

#[cfg(target_os = "linux")]
pub(crate) use mail_linux_platform::{
    default_data_dir, open_store_lock, protect_secret, unprotect_secret,
};

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
compile_error!("mail_core supports Windows, macOS, and Linux platform selection only");
