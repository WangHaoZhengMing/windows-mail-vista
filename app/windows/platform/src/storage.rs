//! The lock file is held without sharing; Windows releases it on process exit.

use std::{fs::File, os::windows::fs::OpenOptionsExt, path::Path};

pub fn open_store_lock(path: &Path) -> std::io::Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
}
