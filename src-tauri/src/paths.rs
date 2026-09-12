//! Locating the two BTD6 files, the app's own storage, and the running-game check.

use std::path::{Path, PathBuf};
use sysinfo::{ProcessRefreshKind, RefreshKind, System, UpdateKind};

/// Steam user id + BTD6 app id, constants for v1 (SPEC.md section 5).
pub const STEAM_USER_ID: &str = "1276140654";
pub const BTD6_APP_ID: &str = "960090";
pub const GAME_PROCESS_NAME: &str = "BloonsTD6.exe";

/// Directory that holds `saves/` and `settings.json`: next to the exe in release builds,
/// the repo root in `tauri dev` (where the exe sits under `src-tauri/target/`).
pub fn base_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        if let Some(repo) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
            return repo.to_path_buf();
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn saves_dir() -> PathBuf {
    base_dir().join("saves")
}

pub fn settings_path() -> PathBuf {
    base_dir().join("settings.json")
}

/// `HKCU\Software\Valve\Steam\SteamPath`, falling back to the default install location.
pub fn steam_root() -> PathBuf {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Valve\\Steam");
        if let Ok(key) = key {
            if let Ok(path) = key.get_value::<String, _>("SteamPath") {
                let p = PathBuf::from(path.replace('/', "\\"));
                if p.is_dir() {
                    return p;
                }
            }
        }
    }
    PathBuf::from("C:\\Program Files (x86)\\Steam")
}

pub fn default_profile_path() -> PathBuf {
    steam_root()
        .join("userdata")
        .join(STEAM_USER_ID)
        .join(BTD6_APP_ID)
        .join("local")
        .join("link")
        .join("PRODUCTION")
        .join("current")
        .join("Profile.Save")
}

pub fn default_map_save_path() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\"));
    home.join("AppData")
        .join("LocalLow")
        .join("Ninja Kiwi")
        .join("BloonsTD6")
        .join("MapSaveData")
}

/// True when a process named `BloonsTD6.exe` exists.
pub fn game_running() -> bool {
    let refresh = RefreshKind::nothing()
        .with_processes(ProcessRefreshKind::nothing().with_exe(UpdateKind::Never));
    let sys = System::new_with_specifics(refresh);
    sys.processes().values().any(|p| {
        p.name()
            .to_string_lossy()
            .eq_ignore_ascii_case(GAME_PROCESS_NAME)
    })
}
