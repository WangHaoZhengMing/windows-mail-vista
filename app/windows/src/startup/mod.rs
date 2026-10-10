//! Windows application startup, command routing and explicit legacy DLL support.

#[cfg(windows)]
mod platform;
#[cfg(windows)]
pub use platform::native;
pub mod command;
#[cfg(windows)]
pub mod identify;
pub mod pe;

pub const INSTANCE_MUTEX: &str = "Local\\OutlookExpress_InstanceMutex_101897";
pub const HIDDEN_WINDOW: &str = "OutlookExpressHiddenWindow";
pub const START_EXPORT: &[u8] = b"CoStartOutlookExpressW\0";
pub const START_FLAGS: u32 = 0x3f;
pub const RESTART: i32 = 0x714;

/// RVA 0x4384..0x43aa: most core results become process exit code zero.
pub fn normalize_core_result(result: i32) -> i32 {
    if result < 0 && result != 0x8004100au32 as i32 && result != 0x80040113u32 as i32 {
        103
    } else {
        0
    }
}

/// RVA 0x423f..0x4281: distinguish missing core from missing dependencies/resources.
pub fn loader_exit_code(error: u32, core_exists: bool, resources_exist: bool) -> i32 {
    match error {
        126 if core_exists => 105,
        126 => 107,
        1114 if resources_exist => 106,
        1114 => 108,
        _ => 102,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_hresult_exceptions_are_not_startup_errors() {
        assert_eq!(normalize_core_result(0x8004100au32 as i32), 0);
        assert_eq!(normalize_core_result(0x80040113u32 as i32), 0);
        assert_eq!(normalize_core_result(0x80004005u32 as i32), 103);
        assert_eq!(normalize_core_result(0), 0);
        assert_eq!(normalize_core_result(42), 0);
    }

    #[test]
    fn loader_diagnostics_preserve_original_codes() {
        assert_eq!(loader_exit_code(126, false, false), 107);
        assert_eq!(loader_exit_code(126, true, false), 105);
        assert_eq!(loader_exit_code(1114, true, false), 108);
        assert_eq!(loader_exit_code(1114, true, true), 106);
        assert_eq!(loader_exit_code(193, true, true), 102);
    }
}
