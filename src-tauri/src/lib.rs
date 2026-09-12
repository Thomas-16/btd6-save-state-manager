//! Tauri setup and the commands exposed to the frontend (SPEC.md section 6).

pub mod paths;
pub mod saveformat;
pub mod settings;
pub mod snapshot;

use saveformat::ProfileInfo;
use serde::{Deserialize, Serialize};
use settings::{Settings, WindowRect};
use snapshot::{Meta, RestoreReport};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::{Manager, PhysicalPosition, PhysicalSize, State, WindowEvent};

/// Cache of the decoded live profile keyed by (path, mtime, len) so the 2 s poll stays cheap.
#[derive(Default)]
struct LiveCache {
    key: Option<(PathBuf, Option<SystemTime>, u64)>,
    value: Option<Result<ProfileInfo, String>>,
}

#[derive(Default)]
struct AppState {
    settings: Mutex<Settings>,
    live: Mutex<LiveCache>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub running: bool,
    pub process_name: String,
    pub profile_path: String,
    pub profile_exists: bool,
    pub map_save_path: String,
    pub map_save_exists: bool,
    pub saves_dir: String,
    pub live: Option<ProfileInfo>,
    pub live_error: Option<String>,
    pub default_profile_path: String,
    pub default_map_save_path: String,
}

fn live_info(state: &AppState, profile_path: &Path) -> Option<Result<ProfileInfo, String>> {
    let md = std::fs::metadata(profile_path).ok()?;
    let key = (profile_path.to_path_buf(), md.modified().ok(), md.len());
    let mut cache = state.live.lock().unwrap();
    if cache.key.as_ref() == Some(&key) {
        return cache.value.clone();
    }
    let value = std::fs::read(profile_path)
        .map_err(|e| e.to_string())
        .and_then(|b| saveformat::read_profile_info(&b).map_err(|e| e.to_string()));
    cache.key = Some(key);
    cache.value = Some(value.clone());
    Some(value)
}

#[tauri::command]
fn get_status(state: State<'_, AppState>) -> Status {
    let settings = state.settings.lock().unwrap().clone();
    let profile_path = settings.profile_path();
    let map_save_path = settings.map_save_path();
    let (live, live_error) = match live_info(&state, &profile_path) {
        Some(Ok(info)) => (Some(info), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    Status {
        running: paths::game_running(),
        process_name: paths::GAME_PROCESS_NAME.to_owned(),
        profile_exists: profile_path.is_file(),
        profile_path: profile_path.display().to_string(),
        map_save_exists: map_save_path.is_file(),
        map_save_path: map_save_path.display().to_string(),
        saves_dir: paths::saves_dir().display().to_string(),
        live,
        live_error,
        default_profile_path: paths::default_profile_path().display().to_string(),
        default_map_save_path: paths::default_map_save_path().display().to_string(),
    }
}

#[tauri::command]
fn list_snapshots() -> Vec<Meta> {
    snapshot::list(&paths::saves_dir())
}

#[tauri::command]
fn create_snapshot(state: State<'_, AppState>, name: String, note: String) -> Result<Meta, String> {
    let settings = state.settings.lock().unwrap().clone();
    snapshot::create(
        &paths::saves_dir(),
        &settings.profile_path(),
        &settings.map_save_path(),
        &name,
        &note,
    )
}

#[tauri::command]
fn restore_snapshot(state: State<'_, AppState>, id: String) -> Result<RestoreReport, String> {
    if paths::game_running() {
        return Err("BTD6 is running. Close it first.".to_owned());
    }
    let settings = state.settings.lock().unwrap().clone();
    snapshot::restore(
        &paths::saves_dir(),
        &settings.profile_path(),
        &settings.map_save_path(),
        &id,
    )
}

#[tauri::command]
fn rename_snapshot(id: String, name: String) -> Result<Meta, String> {
    snapshot::rename(&paths::saves_dir(), &id, &name)
}

#[tauri::command]
fn set_note(id: String, note: String) -> Result<Meta, String> {
    snapshot::set_note(&paths::saves_dir(), &id, &note)
}

#[tauri::command]
fn delete_snapshot(id: String) -> Result<(), String> {
    snapshot::delete(&paths::saves_dir(), &id)
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

/// Empty strings reset a path to its resolved default.
#[tauri::command]
fn set_paths(
    state: State<'_, AppState>,
    profile_path: String,
    map_save_path: String,
) -> Result<Settings, String> {
    let mut settings = state.settings.lock().unwrap();
    let clean = |s: String| {
        let t = s.trim().to_owned();
        (!t.is_empty()).then_some(t)
    };
    settings.profile_path = clean(profile_path);
    settings.map_save_path = clean(map_save_path);
    settings.save().map_err(|e| format!("saving settings.json: {e}"))?;
    Ok(settings.clone())
}

fn current_rect(window: &tauri::WebviewWindow) -> Option<WindowRect> {
    if window.is_maximized().unwrap_or(false) || window.is_minimized().unwrap_or(false) {
        return None;
    }
    // set_position takes the outer (frame) position but set_size takes the inner size,
    // so record exactly what will be fed back in.
    let pos = window.outer_position().ok()?;
    let size = window.inner_size().ok()?;
    if size.width < 100 || size.height < 100 {
        return None;
    }
    Some(WindowRect { x: pos.x, y: pos.y, width: size.width, height: size.height })
}

fn rect_on_some_monitor(window: &tauri::WebviewWindow, r: &WindowRect) -> bool {
    let Ok(monitors) = window.available_monitors() else { return true };
    monitors.iter().any(|m| {
        let p = m.position();
        let s = m.size();
        let (cx, cy) = (r.x + (r.width as i32) / 2, r.y + 40);
        cx >= p.x && cx < p.x + s.width as i32 && cy >= p.y && cy < p.y + s.height as i32
    })
}

/// Records the window rect in memory; writes settings.json only when `write` is set so a
/// drag does not hammer the disk.
fn persist_window(app: &tauri::AppHandle, write: bool) {
    let Some(window) = app.get_webview_window("main") else { return };
    let state = app.state::<AppState>();
    let mut settings = state.settings.lock().unwrap();
    if let Some(rect) = current_rect(&window) {
        settings.window = Some(rect);
    }
    if write {
        let _ = settings.save();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { settings: Mutex::new(Settings::load()), live: Mutex::default() })
        .setup(|app| {
            let window = app.get_webview_window("main").expect("main window");
            let saved = app.state::<AppState>().settings.lock().unwrap().window;
            if let Some(r) = saved {
                if rect_on_some_monitor(&window, &r) {
                    let _ = window.set_position(PhysicalPosition::new(r.x, r.y));
                    let _ = window.set_size(PhysicalSize::new(r.width, r.height));
                }
            }
            let _ = window.show();
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed => {
                persist_window(window.app_handle(), true)
            }
            WindowEvent::Focused(false) => persist_window(window.app_handle(), true),
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                persist_window(window.app_handle(), false)
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            list_snapshots,
            create_snapshot,
            restore_snapshot,
            rename_snapshot,
            set_note,
            delete_snapshot,
            get_settings,
            set_paths,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
