//! Snapshot folders under `saves/`: create, list, restore, rename, note, delete.
//!
//! One folder per snapshot with its own `meta.json`; there is no global index. All writes to
//! the live game files go through [`atomic_write`], which writes a temp file in the target
//! directory and renames it over the original.

use crate::saveformat::{self, ProfileInfo, SavedMapInfo};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const PROFILE_FILE: &str = "Profile.Save";
pub const MAP_SAVE_FILE: &str = "MapSaveData";
const META_FILE: &str = "meta.json";
const TMP_SUFFIX: &str = ".bssm-tmp";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Meta {
    pub id: String,
    pub name: String,
    pub note: String,
    pub created_at: DateTime<Utc>,
    pub game_version: Option<String>,
    pub rank: Option<i64>,
    pub monkey_money: Option<i64>,
    pub saved_maps: Vec<SavedMapInfo>,
    pub last_played_key: Option<String>,
    pub summary: Option<String>,
    /// `MapSaveData` existed and was copied into the snapshot.
    pub map_save_data_present: bool,
    /// The copied `MapSaveData` holds a real game state rather than `null`.
    pub map_save_has_game: bool,
    /// The copied `Profile.Save` decrypted and parsed.
    pub readable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_restored_at: Option<DateTime<Utc>>,
}

impl Default for Meta {
    fn default() -> Self {
        Meta {
            id: String::new(),
            name: String::new(),
            note: String::new(),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            game_version: None,
            rank: None,
            monkey_money: None,
            saved_maps: Vec::new(),
            last_played_key: None,
            summary: None,
            map_save_data_present: false,
            map_save_has_game: false,
            readable: false,
            error: None,
            last_restored_at: None,
        }
    }
}

impl Meta {
    fn apply_profile_info(&mut self, info: ProfileInfo) {
        self.game_version = info.game_version;
        self.rank = info.rank;
        self.monkey_money = info.monkey_money;
        self.saved_maps = info.saved_maps;
        self.last_played_key = info.last_played_key;
        self.summary = info.summary;
        self.readable = true;
        self.error = None;
    }
}

/// Writes `bytes` to `target` via a temp file in the same directory plus rename.
/// On Windows `fs::rename` replaces an existing destination file.
pub fn atomic_write(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = target.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "target has no parent directory")
    })?;
    fs::create_dir_all(dir)?;
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let tmp = dir.join(format!("{file_name}{TMP_SUFFIX}"));
    {
        let mut f = fs::File::create(&tmp)?;
        use std::io::Write;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    match fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn write_meta(dir: &Path, meta: &Meta) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(meta).map_err(|e| e.to_string())?;
    atomic_write(&dir.join(META_FILE), &json).map_err(|e| format!("writing meta.json: {e}"))
}

fn read_meta(dir: &Path) -> Option<Meta> {
    let bytes = fs::read(dir.join(META_FILE)).ok()?;
    let mut meta: Meta = serde_json::from_slice(&bytes).ok()?;
    // The folder name is authoritative for the id.
    meta.id = dir.file_name()?.to_string_lossy().into_owned();
    Some(meta)
}

/// Folder-safe slug: lowercase ASCII alphanumerics, runs of anything else become `-`.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    if out.is_empty() {
        "snapshot".to_owned()
    } else {
        out
    }
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 120
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn snapshot_dir(saves: &Path, id: &str) -> Result<PathBuf, String> {
    if !is_safe_id(id) {
        return Err(format!("invalid snapshot id '{id}'"));
    }
    let dir = saves.join(id);
    if !dir.is_dir() {
        return Err(format!("snapshot '{id}' does not exist"));
    }
    Ok(dir)
}

/// All snapshots, newest first. Folders without a readable `meta.json` are skipped.
pub fn list(saves: &Path) -> Vec<Meta> {
    let mut out: Vec<Meta> = match fs::read_dir(saves) {
        Ok(rd) => rd
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| read_meta(&e.path()))
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| b.id.cmp(&a.id)));
    out
}

/// Copies the live files into a new snapshot folder and fills `meta.json`.
pub fn create(
    saves: &Path,
    profile_path: &Path,
    map_save_path: &Path,
    name: &str,
    note: &str,
) -> Result<Meta, String> {
    let name = name.trim();
    let name = if name.is_empty() { "Untitled snapshot" } else { name };
    let profile_bytes = fs::read(profile_path)
        .map_err(|e| format!("cannot read {}: {e}", profile_path.display()))?;
    let map_save_bytes = fs::read(map_save_path).ok();

    let now_utc = Utc::now();
    let stamp = now_utc.with_timezone(&Local).format("%Y-%m-%d_%H%M%S");
    let base_id = format!("{stamp}_{}", slugify(name));
    fs::create_dir_all(saves).map_err(|e| format!("cannot create saves dir: {e}"))?;
    let mut id = base_id.clone();
    let mut n = 2;
    while saves.join(&id).exists() {
        id = format!("{base_id}-{n}");
        n += 1;
    }
    let dir = saves.join(&id);
    fs::create_dir(&dir).map_err(|e| format!("cannot create snapshot folder: {e}"))?;

    let finish = |dir: &Path| -> Result<Meta, String> {
        fs::write(dir.join(PROFILE_FILE), &profile_bytes)
            .map_err(|e| format!("copying Profile.Save: {e}"))?;
        if let Some(b) = &map_save_bytes {
            fs::write(dir.join(MAP_SAVE_FILE), b).map_err(|e| format!("copying MapSaveData: {e}"))?;
        }

        let mut meta = Meta {
            id: id.clone(),
            name: name.to_owned(),
            note: note.trim().to_owned(),
            created_at: now_utc,
            map_save_data_present: map_save_bytes.is_some(),
            ..Meta::default()
        };
        // Validate the copy, not the in-memory bytes, so a bad write is caught here.
        match fs::read(dir.join(PROFILE_FILE))
            .map_err(|e| e.to_string())
            .and_then(|b| saveformat::read_profile_info(&b).map_err(|e| e.to_string()))
        {
            Ok(info) => meta.apply_profile_info(info),
            Err(e) => {
                meta.readable = false;
                meta.error = Some(e);
            }
        }
        if map_save_bytes.is_some() {
            meta.map_save_has_game = fs::read(dir.join(MAP_SAVE_FILE))
                .ok()
                .and_then(|b| saveformat::map_save_has_game(&b).ok())
                .unwrap_or(false);
        }
        write_meta(dir, &meta)?;
        Ok(meta)
    };

    match finish(&dir) {
        Ok(meta) => Ok(meta),
        Err(e) => {
            let _ = fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub meta: Meta,
    pub profile_written: String,
    pub map_save_written: Option<String>,
}

/// Overwrites the live files with the snapshot's copies. The caller checks the game state.
pub fn restore(
    saves: &Path,
    profile_path: &Path,
    map_save_path: &Path,
    id: &str,
) -> Result<RestoreReport, String> {
    let dir = snapshot_dir(saves, id)?;
    let mut meta = read_meta(&dir).ok_or_else(|| "snapshot has no readable meta.json".to_owned())?;
    let profile_bytes =
        fs::read(dir.join(PROFILE_FILE)).map_err(|e| format!("snapshot Profile.Save: {e}"))?;
    // Sanity-check the header before touching anything live.
    saveformat::parse_container(&profile_bytes)
        .map_err(|e| format!("snapshot Profile.Save is not a BTD6 save: {e}"))?;
    let map_save_bytes = fs::read(dir.join(MAP_SAVE_FILE)).ok();

    atomic_write(profile_path, &profile_bytes)
        .map_err(|e| format!("writing {}: {e}", profile_path.display()))?;
    let mut map_save_written = None;
    if let Some(b) = &map_save_bytes {
        atomic_write(map_save_path, b)
            .map_err(|e| format!("Profile.Save restored, but writing {} failed: {e}", map_save_path.display()))?;
        map_save_written = Some(map_save_path.display().to_string());
    }

    meta.last_restored_at = Some(Utc::now());
    write_meta(&dir, &meta)?;
    Ok(RestoreReport {
        meta,
        profile_written: profile_path.display().to_string(),
        map_save_written,
    })
}

pub fn rename(saves: &Path, id: &str, name: &str) -> Result<Meta, String> {
    let dir = snapshot_dir(saves, id)?;
    let mut meta = read_meta(&dir).ok_or("snapshot has no readable meta.json")?;
    let name = name.trim();
    if name.is_empty() {
        return Err("name cannot be empty".to_owned());
    }
    meta.name = name.to_owned();
    write_meta(&dir, &meta)?;
    Ok(meta)
}

pub fn set_note(saves: &Path, id: &str, note: &str) -> Result<Meta, String> {
    let dir = snapshot_dir(saves, id)?;
    let mut meta = read_meta(&dir).ok_or("snapshot has no readable meta.json")?;
    meta.note = note.trim().to_owned();
    write_meta(&dir, &meta)?;
    Ok(meta)
}

pub fn delete(saves: &Path, id: &str) -> Result<(), String> {
    let dir = snapshot_dir(saves, id)?;
    fs::remove_dir_all(&dir).map_err(|e| format!("deleting snapshot: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Lych Elite · Frozen Over · R59"), "lych-elite-frozen-over-r59");
        assert_eq!(slugify("   "), "snapshot");
        assert_eq!(slugify("--a--b--"), "a-b");
        assert!(slugify(&"x".repeat(100)).len() <= 40);
    }

    #[test]
    fn ids_are_validated() {
        assert!(is_safe_id("2026-09-12_112657_lych"));
        assert!(!is_safe_id("../etc"));
        assert!(!is_safe_id(""));
        assert!(!is_safe_id("a/b"));
    }

    #[test]
    fn atomic_write_replaces_existing_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("bssm-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("Profile.Save");
        fs::write(&target, b"old").unwrap();
        atomic_write(&target, b"new").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(!dir.join(format!("Profile.Save{TMP_SUFFIX}")).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn create_restore_rename_note_delete_round_trip() {
        use crate::saveformat::testutil::{encrypted_profile, map_save};
        let root = std::env::temp_dir().join(format!("bssm-rt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let saves = root.join("saves");
        let live_profile = root.join("steam").join("current").join(PROFILE_FILE);
        let live_map = root.join("locallow").join(MAP_SAVE_FILE);
        fs::create_dir_all(live_profile.parent().unwrap()).unwrap();
        fs::create_dir_all(live_map.parent().unwrap()).unwrap();

        let profile_v1 = encrypted_profile(
            r#"{"savedByGameVersion":"56.3","rank":143,"monkeyMoney":8042,"savedMaps":{
              "BB":{"mapName":"FrozenOver","modeName":"Standard","mapDifficulty":"Medium","round":59,"gameType":4,
                    "playerChallengeId":"Lych61_mtw1v6de","gameId":2152,"attemptId":10,
                    "metaData":{"BossEventIsElite":"True","BossEventTier":"1"}}}}"#,
        );
        fs::write(&live_profile, &profile_v1).unwrap();
        fs::write(&live_map, map_save("null")).unwrap();

        // Create: files copied, meta filled from the copy.
        let meta = create(&saves, &live_profile, &live_map, "  Lych run  ", "before r60").unwrap();
        assert_eq!(meta.name, "Lych run");
        assert_eq!(meta.note, "before r60");
        assert!(meta.readable);
        assert!(meta.map_save_data_present);
        assert!(!meta.map_save_has_game);
        assert_eq!(meta.summary.as_deref(), Some("Lych Elite T1 · Frozen Over · R59"));
        assert_eq!(meta.game_version.as_deref(), Some("56.3"));
        assert!(meta.id.ends_with("_lych-run"));
        assert_eq!(fs::read(saves.join(&meta.id).join(PROFILE_FILE)).unwrap(), profile_v1);
        assert_eq!(list(&saves).len(), 1);

        // Same name again gets a distinct folder.
        let meta2 = create(&saves, &live_profile, &live_map, "Lych run", "").unwrap();
        assert_ne!(meta2.id, meta.id);
        assert_eq!(list(&saves)[0].id, meta2.id, "newest first");

        // The live files move on (a mid-run autosave appears), then restore rolls them back.
        let profile_v2 = encrypted_profile(r#"{"savedByGameVersion":"56.4","rank":144,"savedMaps":{}}"#);
        fs::write(&live_profile, &profile_v2).unwrap();
        fs::write(&live_map, map_save(r#"{"round":61}"#)).unwrap();
        let report = restore(&saves, &live_profile, &live_map, &meta.id).unwrap();
        assert_eq!(fs::read(&live_profile).unwrap(), profile_v1);
        assert!(!saveformat::map_save_has_game(&fs::read(&live_map).unwrap()).unwrap());
        assert!(report.map_save_written.is_some());
        assert!(report.meta.last_restored_at.is_some());
        assert!(!live_profile.with_file_name("Profile.Save.bssm-tmp").exists());

        // Rename and note persist through a fresh list().
        rename(&saves, &meta.id, "Renamed").unwrap();
        set_note(&saves, &meta.id, "  new note ").unwrap();
        let again = list(&saves).into_iter().find(|m| m.id == meta.id).unwrap();
        assert_eq!(again.name, "Renamed");
        assert_eq!(again.note, "new note");
        assert!(rename(&saves, &meta.id, "   ").is_err());

        // Unreadable profile is kept but flagged.
        fs::write(&live_profile, b"garbage").unwrap();
        let bad = create(&saves, &live_profile, &live_map, "bad", "").unwrap();
        assert!(!bad.readable);
        assert!(bad.error.is_some());
        assert!(restore(&saves, &live_profile, &live_map, &bad.id).is_err(), "header check blocks");

        delete(&saves, &meta.id).unwrap();
        assert!(delete(&saves, &meta.id).is_err());
        assert!(restore(&saves, &live_profile, &live_map, "../x").is_err());
        assert_eq!(list(&saves).len(), 2);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn meta_round_trips_through_json() {
        let meta = Meta {
            id: "x".into(),
            name: "n".into(),
            created_at: Utc::now(),
            readable: true,
            ..Meta::default()
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: Meta = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, back);
        // Older/partial meta files still load thanks to `default`.
        let partial: Meta = serde_json::from_str(r#"{"id":"a","name":"b"}"#).unwrap();
        assert_eq!(partial.name, "b");
        assert!(!partial.readable);
    }
}
