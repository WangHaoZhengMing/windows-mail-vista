//! Reserved for the future native macOS user interface.
//!
//! No macOS window backend is implemented yet.

pub fn run(_command: &str, _show: i32) -> crate::Result<()> {
    Err("The macOS user interface is not implemented yet".into())
}

pub fn active() -> bool {
    false
}

pub fn shutdown() {}
