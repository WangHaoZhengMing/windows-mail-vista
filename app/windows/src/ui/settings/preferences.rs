//! Persisted desktop layout and theme preferences.
use crate::Result;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    pub folders: bool,
    pub preview: bool,
    pub dark_mode: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            folders: true,
            preview: true,
            dark_mode: false,
        }
    }
}
impl Preferences {
    pub(crate) fn exists() -> bool {
        Self::path().is_file()
    }
    pub(super) fn path() -> std::path::PathBuf {
        crate::store::default_root().join("desktop-settings.json")
    }
    pub(crate) fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    pub(crate) fn save(&self) -> Result<()> {
        let path = Self::path();
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }
}
