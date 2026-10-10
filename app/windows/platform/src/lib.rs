//! Windows API adapters shared by the application's windows.
//!
//! Control helpers return borrowed handles: Windows destroys child controls with
//! their parent window. Owned fonts and icons are managed by `dpi::Resources`.

#![cfg(windows)]

#[cfg(feature = "ui")]
pub mod classic;
#[cfg(feature = "ui")]
pub mod controls;
mod credentials;
#[cfg(feature = "ui")]
pub mod dialogs;
#[cfg(feature = "ui")]
pub mod dpi;
#[cfg(feature = "ui")]
pub mod filter;
#[cfg(feature = "ui")]
pub mod html;
#[cfg(feature = "ui")]
pub mod icons;
#[cfg(feature = "ui")]
pub mod ipc;
#[cfg(feature = "ui")]
pub mod menubar;
mod paths;
#[cfg(feature = "ui")]
pub mod search;
mod storage;
#[cfg(feature = "ui")]
pub mod strings;

pub use credentials::{protect_secret, unprotect_secret};
pub use paths::default_data_dir;
pub use storage::open_store_lock;

#[cfg(feature = "ui")]
pub mod theme;
