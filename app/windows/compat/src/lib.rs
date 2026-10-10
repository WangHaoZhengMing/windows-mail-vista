//! Windows Mail compatibility exports over the independent Rust mail crates.
//! This crate preserves the implemented Windows Mail ABI and Simple MAPI contracts.

pub use mail_core::{MAX_MESSAGE, Result, account, message, network, store};

#[cfg(windows)]
pub mod abi;
#[cfg(windows)]
pub mod mapi;
#[cfg(windows)]
pub use mail_windows::ui;
#[cfg(windows)]
pub use mail_windows::{instance_names, instance_names_for};
