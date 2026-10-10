//! Reserved macOS backend. Native UI, Keychain credentials and store locks are
//! not implemented; callers receive explicit unsupported-service errors.

#![cfg(target_os = "macos")]

pub mod html;

use std::{
    fs::File,
    path::{Path, PathBuf},
};

pub fn default_data_dir() -> PathBuf {
    std::env::var_os("WINMAIL_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("Library/Application Support/WinMailRust")
        })
}

pub fn protect_secret(_bytes: &[u8]) -> Result<Vec<u8>, String> {
    Err("macOS Keychain credential storage is not implemented".into())
}

pub fn unprotect_secret(_bytes: &[u8]) -> Result<Vec<u8>, String> {
    Err("macOS Keychain credential storage is not implemented".into())
}

pub fn open_store_lock(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "macOS mail store locking is not implemented",
    ))
}
