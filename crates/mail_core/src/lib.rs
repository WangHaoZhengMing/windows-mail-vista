//! Mail data and protocols shared by application and compatibility adapters.
//!
//! OS services are selected through a private platform facade; this crate has no window
//! handles, native command IDs or dependency on a particular UI toolkit.

pub mod account;
pub mod message;
pub mod network;
pub mod oauth;
mod platform;
pub mod store;

pub type Result<T> = std::result::Result<T, String>;
pub const MAX_MESSAGE: usize = 50 * 1024 * 1024;
