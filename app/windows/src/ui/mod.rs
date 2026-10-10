//! Native UI grouped by window responsibilities; business logic lives in `mail_app`.
mod about;
mod accounts;
mod compose;
mod main;
mod reader;
mod settings;
mod setup;
mod shared;

pub use main::{active, run, shutdown};

#[cfg(test)]
pub(crate) static WINDOW_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
