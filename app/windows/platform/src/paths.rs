use std::path::PathBuf;

pub fn default_data_dir() -> PathBuf {
    std::env::var_os("WINMAIL_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("WinMailRust")
        })
}
