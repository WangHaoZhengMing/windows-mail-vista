//! Reserved for the future native Linux user interface.
//!
//! No Linux window backend is implemented yet.

pub fn run(_command: &str, _show: i32) -> crate::Result<()> {
    Err("The Linux user interface is not implemented yet".into())
}

pub fn active() -> bool {
    false
}

pub fn shutdown() {}
