//! `settings.json` next to the executable: window geometry and user-overridden file paths.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub window: Option<WindowRect>,
    /// User override for `Profile.Save`; `None` means "use the resolved default".
    pub profile_path: Option<String>,
    /// User override for `MapSaveData`.
    pub map_save_path: Option<String>,
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read(paths::settings_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = paths::settings_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap())?;
        std::fs::rename(&tmp, &path)
    }

    pub fn profile_path(&self) -> PathBuf {
        self.profile_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(paths::default_profile_path)
    }

    pub fn map_save_path(&self) -> PathBuf {
        self.map_save_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(paths::default_map_save_path)
    }
}
