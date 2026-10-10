//! Windows single-instance object names shared by the launcher and mail UI.

use sha2::{Digest, Sha256};

pub fn instance_names_for(key: &str) -> (String, String) {
    let digest = format!("{:x}", Sha256::digest(key.as_bytes()));
    (
        format!("Local\\WinMailRust.Instance.{}", &digest[..24]),
        format!("WinMailRust.Window.{}", &digest[..24]),
    )
}
