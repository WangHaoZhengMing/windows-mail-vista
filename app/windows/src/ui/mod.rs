//! Native window adapters. Business state and mail operations live in `app`.
mod account_window;
mod command_ids;
mod compose_window;
mod controls;
mod dispatch;
mod folder_tree;
mod layout;
mod main_window;
mod menus;
mod reader_window;
mod toolbar_layout;

pub use main_window::{active, run, shutdown};

/// Compose windows post commands to their owning main window.
const DISPATCH: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 10;
const READER_DISPATCH: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 12;
const HTML_STATUS: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 11;

#[cfg(test)]
pub(crate) static WINDOW_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
