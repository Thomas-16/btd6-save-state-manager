//! Selective restore: which top-level profile keys belong to which category, a preview of
//! what differs between a snapshot and the live profile, and the merge that takes only the
//! selected parts from the snapshot and re-encrypts the result (SPEC.md section 3.3).
//!
//! Everything that crosses Tauri IPC here is plain strings/numbers/bools; `serde_json::Value`
//! never leaves this module (with `arbitrary_precision` it does not serialize portably).

use crate::saveformat::{self, describe, saved_map_info, SavedMapInfo};
use crate::snapshot::{self, RestoreReport};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub const KEY_SAVED_MAPS: &str = "savedMaps";
pub const KEY_BOSS_EVENTS: &str = "bossesEventData";
pub const KEY_ODYSSEY: &str = "odysseySaveData";
/// Suffix of a boss run's continue checkpoint (`BB` + `BBCONT`).
const CONTINUE_SUFFIX: &str = "CONT";

/// Always kept from the live profile: currencies, progression counters and identity.
pub const LOCKED_KEYS: &[&str] = &[
    "monkeyMoney", "xp", "rank", "veteranXp", "veteranRank", "levelCapWas", "trophies",
    "lifetimeTrophies", "lifetimeTeamTrophies", "trophiesSpent", "knowledgePoints",
    "unclaimedTrophies", "ownerID", "trophiesWalletId", "teamTrophiesWalletId",
    "savedByGameVersion", "savedBySkuId", "timeStamp", "lastSavedUTCTime", "purchase",
    "noLinkIdVersion", "isPendingNoLinkIdMigration", "pendingSteamAttribution",
];

pub struct CategoryDef {
    pub id: &'static str,
    pub label: &'static str,
    pub keys: &'static [&'static str],
}

pub const OTHER_ID: &str = "other";
pub const OTHER_LABEL: &str = "Everything else (stats, editors, unknown keys)";

/// Whole-key categories, in display order. `other` is computed at runtime.
pub const CATEGORIES: &[CategoryDef] = &[
    CategoryDef {
        id: "boss_records",
        label: "Boss & odyssey records",
        keys: &[
            "bossesEventData", "bossMedals", "bossBadgeData", "bossLeaderboardMedals",
            "bossLeaderboardEliteMedals", "completedBossEvents", "uniqueBossesDefeated",
            "bossChallengeScores", "bossChallengeScores2p", "bossChallengeScores3p",
            "bossChallengeScores4p", "isBossRankedSelected", "isBossEliteSelected",
            "selectedChallengeBoss", "selectedChallengeBossMap", "selectedChallengeBossRanked",
            "selectedChallengeBossScoring", "seenBossChallenge", "completedOdysseys",
            "totalCompletedOdysseys",
        ],
    },
    CategoryDef {
        id: "maps",
        label: "Map completion & medals",
        keys: &[
            "mapInfo", "highestSeenRound", "highestSeenRoundCurrentVersion", "completedGame",
            "raceMedalData", "totalRacesEntered", "raceBestTimeForAchievements",
            "ctLeaderboardBadges", "ctGlobalLeaderboardBadges", "ctSeenFirstTimePopup",
            "ctCapturedTileType",
        ],
    },
    CategoryDef {
        id: "towers",
        label: "Tower XP, unlocks & knowledge",
        keys: &[
            "towerXp", "acquiredUpgrades", "viewedUpgrades", "paragonUpgradesPurchased",
            "unlockedTowers", "unlockedHeroes", "unlockedTowerSkins", "towerUnlockProgresses",
            "selectedTowerForUnlockProgression", "acquiredKnowledge", "paidForKnowledge",
            "knowledgeDisabled", "newKnowledgePoints", "heavyInvestment",
            "seenUnlockedNotification", "seenUnlockedHeroes", "seenNewTowerNotifications",
            "seenNewHeroNotification", "seenNewTowerSkinNotification",
        ],
    },
    CategoryDef {
        id: "items",
        label: "Powers, insta-monkeys & trophy store",
        keys: &[
            "powers", "powersData", "instaTowers", "powersProSaveData", "seenPowers",
            "trophyStorePurchasedItems", "favoriteTrophyItems", "trophyStoreSeen",
        ],
    },
    CategoryDef {
        id: "achievements",
        label: "Achievements",
        keys: &[
            "achievementsProgress", "achievementsClaimed", "achievementsSeen",
            "achievementsPostedToXBoxLive", "achievementsPostedToGooglePlay", "giftedAchievements",
        ],
    },
    CategoryDef {
        id: "events",
        label: "Daily challenges, events & seasons",
        keys: &[
            "dailyRewardIndex", "nextDailyRewardDateTime", "totalDailyChallengesCompleted",
            "consecutiveDailyChallengesCompleted", "uniqueCompletedDailyChallengeIds",
            "playedDailyChallengeIds", "lostDailyChallengeIds", "wonDailyChallengeIds",
            "numWonUniqueDailyChallenges", "challengesPlayed", "challengesShared",
            "collectionEventData", "collectionEventMapBonusData", "collectionEventCratesOpened",
            "collectionEventCratesTypesOpened", "monkeyTeamsData", "monkeyTeamsWins",
            "goldenBloonData", "goldenBloonsPopped", "questsEventData", "questsSaveData",
            "claimedSocialSeasonRewards", "seenSocialSeasonEventIds",
            "socialSeasonIdsPendingEndPopup", "profilePerkData", "rewardedSsBadges", "seasonStart",
            "midSeason", "seasonChampion", "socialSeasonBadgesSequential", "championSocialSeasons",
            "perksToggledOn", "currentSweepstakesStats", "legendsData", "seenLegends",
            "seenLegendUpdate", "shownLegendPopups", "giftsReceived", "receivedGiftEventIds",
            "pendingGifts", "claimedGifts", "seenEvents", "seenStorePipIds", "seenMessageEventIds",
            "deletedMessageEventIds", "privateMessages", "lastReceivedInboxMessage",
        ],
    },
    CategoryDef {
        id: "settings",
        label: "Settings, hotkeys & cosmetics",
        keys: &[
            "inGameSettings", "hotkeysData", "hotkeysData2", "audioSettings",
            "accessibilitySettings", "colorBlindModeOn", "ctScorePanelCollapsed",
            "selectedTowerSkinData", "profileAvatar", "profileAvatarFrame", "profileBanner",
            "namedMonkeyNames", "savedStats", "savedNamedMonkeyStats", "seenProfileStats",
            "primaryHero", "secondaryHero", "languageCode", "notificationsSettings",
            "notificationPopupInfo", "timesShownNotificationPopup", "neverShowNotificationPopup",
            "useJukeBox", "savedPlayList", "coopQuickMatchSetting", "coopMatchSetToPrivate",
            "friendScreenShowCode", "coopLobbyScreenShowCode", "toggleFriendRequests",
            "toggleTeamRequests", "isOnlineStatusEnabled", "hideAccoladeNotifs",
            "hasPublicAccolades", "selectedContentTab", "tutorialTypeSelected",
            "HasCompletedTutorial", "dataConsent", "analyticsConsent", "hasSeenNewDoubleCash",
            "seenBigBloons", "unlockedBigBloons", "bigBloonsActive", "seenSmallBloons",
            "unlockedSmallBloons", "smallBloonsActive", "seenBigTowers", "unlockedBigTowers",
            "bigTowersActive", "seenSmallTowers", "unlockedSmallTowers", "smallTowersActive",
            "smallBossesActive", "seenSmallBosses", "unlockedSmallBosses", "seenFastTrack",
            "unlockedFastTrack", "fastTrackActive", "rateMeSkuVersionNumber",
            "countGamesSinceSkuRateMeChange", "completedGamesSinceSkuRateMeChange", "seenMiniRace",
            "seenMapEditorInfoPopup", "seenMapBrowser", "seenGameBrowser",
            "seenChallengeModifiedPopup", "cancelledFacebookFriendsPopup", "seenVeteranRankInfo",
            "lastSeenMainMenuPopupTime", "seenDialogues", "seenUpcomingCtEventId",
            "hasSentDiscordMail", "iapLoginReminderCount",
        ],
    },
];

pub fn is_locked(key: &str) -> bool {
    LOCKED_KEYS.contains(&key)
}

/// The table category a key belongs to, if any (locked keys, `savedMaps` and `odysseySaveData`
/// return `None`; unmapped keys are `other`).
pub fn category_of(key: &str) -> Option<&'static str> {
    if is_locked(key) || key == KEY_SAVED_MAPS || key == KEY_ODYSSEY {
        return None;
    }
    CATEGORIES
        .iter()
        .find(|c| c.keys.contains(&key))
        .map(|c| c.id)
        .or(Some(OTHER_ID))
}

// ---- Preview types (IPC-safe) ------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemStatus {
    /// Only in the snapshot: restoring adds it.
    Add,
    /// In both and different: restoring replaces the live one.
    Replace,
    /// In both and identical.
    Same,
    /// Only in the live profile: always kept.
    Kept,
}

/// Progress record tied to a game item: the matching `bossesEventData` entry or `odysseySaveData`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventProgress {
    pub kind: String,
    pub snapshot: Option<String>,
    pub live: Option<String>,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameItem {
    /// `game:<group key>`, e.g. `game:BB`.
    pub id: String,
    /// `savedMaps` keys covered by this item, in either profile (`["BB", "BBCONT"]`).
    pub keys: Vec<String>,
    /// `boss`, `odyssey` or `map`.
    pub kind: String,
    pub status: ItemStatus,
    /// `Lych Elite T1 · Frozen Over`.
    pub label: String,
    pub snapshot: Option<SavedMapInfo>,
    pub live: Option<SavedMapInfo>,
    pub snapshot_continue_round: Option<i64>,
    pub live_continue_round: Option<i64>,
    /// Both present but not the same run (different gameId or challenge id).
    pub different_game: bool,
    pub event: Option<EventProgress>,
    pub default_checked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryDiff {
    pub id: String,
    pub label: String,
    pub changed: bool,
    pub changed_keys: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedDiff {
    pub key: String,
    pub label: String,
    pub snapshot: String,
    pub live: String,
    pub differs: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreview {
    pub snapshot_id: String,
    pub snapshot_name: String,
    /// False when either profile could not be decoded; only raw mode is possible then.
    pub merge_available: bool,
    pub merge_error: Option<String>,
    pub games: Vec<GameItem>,
    pub categories: Vec<CategoryDiff>,
    pub locked: Vec<LockedDiff>,
    /// Nothing selectable differs.
    pub identical: bool,
    pub snapshot_game_version: Option<String>,
    pub live_game_version: Option<String>,
    pub map_save_present: bool,
    pub map_save_has_game: bool,
    /// Id of the game item the snapshot's mid-run `MapSaveData` autosave belongs to. The
    /// autosave is written only when that item is restored.
    pub map_save_item: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RestoreSelection {
    /// Whole-file copy, ignoring everything else.
    pub raw: bool,
    /// Game item ids to take from the snapshot.
    pub games: Vec<String>,
    /// Game item ids whose event progress to take.
    pub events: Vec<String>,
    /// Category ids to take from the snapshot.
    pub categories: Vec<String>,
}

impl RestoreSelection {
    pub fn is_empty(&self) -> bool {
        self.games.is_empty() && self.events.is_empty() && self.categories.is_empty()
    }
}

// ---- Diff ----------------------------------------------------------------------------------

fn saved_maps(v: &Value) -> Option<&Map<String, Value>> {
    v.get(KEY_SAVED_MAPS).and_then(Value::as_object)
}

/// `BBCONT` -> `BB`; anything else unchanged.
fn group_key(key: &str) -> &str {
    key.strip_suffix(CONTINUE_SUFFIX)
        .filter(|s| !s.is_empty())
        .unwrap_or(key)
}

/// `mapInfo` -> `map info`.
pub fn humanize_key(key: &str) -> String {
    saveformat::humanize_map_name(key).to_lowercase()
}

fn locked_label(key: &str) -> String {
    match key {
        "monkeyMoney" => "Monkey money".to_owned(),
        "xp" => "XP".to_owned(),
        "rank" => "Rank".to_owned(),
        "veteranXp" => "Veteran XP".to_owned(),
        "veteranRank" => "Veteran rank".to_owned(),
        "trophies" => "Trophies".to_owned(),
        "lifetimeTrophies" => "Lifetime trophies".to_owned(),
        "trophiesSpent" => "Trophies spent".to_owned(),
        "knowledgePoints" => "Knowledge points".to_owned(),
        "savedByGameVersion" => "Game version".to_owned(),
        "timeStamp" => "Profile timestamp".to_owned(),
        _ => {
            let mut s = humanize_key(key);
            if let Some(first) = s.get(..1) {
                let up = first.to_uppercase();
                s.replace_range(..1, &up);
            }
            s
        }
    }
}

fn display_value(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "—".to_owned(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(m)) => format!("{{{} fields}}", m.len()),
        Some(Value::Array(a)) => format!("[{} items]", a.len()),
        Some(other) => other.to_string(),
    }
}

fn as_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .or_else(|| v.as_str()?.parse().ok())
}

fn find_boss_event<'a>(profile: &'a Value, event_id: &str) -> Option<&'a Value> {
    profile
        .get(KEY_BOSS_EVENTS)?
        .as_array()?
        .iter()
        .find(|e| e.get("eventId").and_then(Value::as_str) == Some(event_id))
}

fn boss_event_summary(entry: &Value, elite: bool) -> String {
    let side = entry.get(if elite { "elite" } else { "normal" });
    let tier = side
        .and_then(|s| s.get("tierBeaten"))
        .and_then(as_i64)
        .unwrap_or(0);
    let best = side
        .and_then(|s| s.get("highestCompletedRound"))
        .and_then(as_i64)
        .unwrap_or(0);
    format!(
        "{} · tier {tier} beaten · best round {best}",
        if elite { "Elite" } else { "Normal" }
    )
}

fn odyssey_summary(data: &Value) -> String {
    let active = data.get("activeOdysey");
    let started = active
        .and_then(|a| a.get("hasStarted"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !started {
        return "Not started".to_owned();
    }
    let idx = active
        .and_then(|a| a.get("currentOdyseyMapSaveIndex"))
        .and_then(as_i64)
        .unwrap_or(0);
    let total = active
        .and_then(|a| a.get("odysseyMapDataHistory"))
        .and_then(Value::as_object)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    if total > 0 {
        format!("In progress · island {} of {total}", idx + 1)
    } else {
        format!("In progress · island {}", idx + 1)
    }
}

fn kind_of(info: &SavedMapInfo) -> &'static str {
    if info.boss.is_some() {
        "boss"
    } else if info.game_type == 3 {
        "odyssey"
    } else {
        "map"
    }
}

fn event_progress(info: &SavedMapInfo, snap: &Value, live: &Value) -> Option<EventProgress> {
    match kind_of(info) {
        "odyssey" => {
            let s = snap.get(KEY_ODYSSEY);
            let l = live.get(KEY_ODYSSEY);
            if s.is_none() && l.is_none() {
                return None;
            }
            Some(EventProgress {
                kind: "odyssey".to_owned(),
                snapshot: s.map(odyssey_summary),
                live: l.map(odyssey_summary),
                changed: s != l,
            })
        }
        "boss" => {
            let id = info.challenge_id.as_deref()?;
            let s = find_boss_event(snap, id);
            let l = find_boss_event(live, id);
            if s.is_none() && l.is_none() {
                return None;
            }
            Some(EventProgress {
                kind: "boss".to_owned(),
                snapshot: s.map(|e| boss_event_summary(e, info.elite)),
                live: l.map(|e| boss_event_summary(e, info.elite)),
                changed: s != l,
            })
        }
        _ => None,
    }
}

/// One item per saved-game group (`BB` + `BBCONT` together), snapshot changes first.
pub fn game_items(snap: &Value, live: &Value) -> Vec<GameItem> {
    let empty = Map::new();
    let sm = saved_maps(snap).unwrap_or(&empty);
    let lm = saved_maps(live).unwrap_or(&empty);

    // Group keys in first-seen order: live first, then snapshot-only.
    let mut groups: Vec<String> = Vec::new();
    for k in lm.keys().chain(sm.keys()) {
        let g = group_key(k).to_owned();
        if !groups.contains(&g) {
            groups.push(g);
        }
    }

    let mut items = Vec::new();
    for g in groups {
        let cont = format!("{g}{CONTINUE_SUFFIX}");
        let mut keys = Vec::new();
        for k in [&g, &cont] {
            if sm.contains_key(k) || lm.contains_key(k) {
                keys.push(k.clone());
            }
        }
        let primary = |m: &Map<String, Value>| -> Option<SavedMapInfo> {
            [&g, &cont]
                .into_iter()
                .find_map(|k| m.get(k).map(|v| saved_map_info(k, v)))
        };
        let s_info = primary(sm);
        let l_info = primary(lm);
        let same = keys.iter().all(|k| sm.get(k) == lm.get(k));
        let status = match (s_info.is_some(), l_info.is_some()) {
            (true, false) => ItemStatus::Add,
            (true, true) if same => ItemStatus::Same,
            (true, true) => ItemStatus::Replace,
            (false, true) => ItemStatus::Kept,
            (false, false) => continue,
        };
        let different_game = match (&s_info, &l_info) {
            (Some(a), Some(b)) => a.game_id != b.game_id || a.challenge_id != b.challenge_id,
            _ => false,
        };
        let cont_round =
            |m: &Map<String, Value>| m.get(&cont).and_then(|v| v.get("round")).and_then(as_i64);
        let describe_info = s_info
            .as_ref()
            .or(l_info.as_ref())
            .expect("one side present");
        let event = event_progress(describe_info, snap, live);
        items.push(GameItem {
            id: format!("game:{g}"),
            keys,
            kind: kind_of(describe_info).to_owned(),
            status,
            label: describe(describe_info),
            snapshot_continue_round: cont_round(sm),
            live_continue_round: cont_round(lm),
            different_game,
            event,
            default_checked: matches!(status, ItemStatus::Add | ItemStatus::Replace),
            snapshot: s_info,
            live: l_info,
        });
    }
    let rank = |s: ItemStatus| match s {
        ItemStatus::Add | ItemStatus::Replace => 0,
        ItemStatus::Same => 1,
        ItemStatus::Kept => 2,
    };
    items.sort_by_key(|i| rank(i.status));
    items
}

/// Keys of the runtime `other` category: everything not mapped anywhere else.
fn other_keys(snap: &Value, live: &Value) -> Vec<String> {
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for p in [snap, live] {
        if let Some(m) = p.as_object() {
            for k in m.keys() {
                if category_of(k) == Some(OTHER_ID) {
                    keys.insert(k.clone());
                }
            }
        }
    }
    keys.into_iter().collect()
}

fn category_keys(id: &str, snap: &Value, live: &Value, has_odyssey_item: bool) -> Option<Vec<String>> {
    if id == OTHER_ID {
        return Some(other_keys(snap, live));
    }
    let def = CATEGORIES.iter().find(|c| c.id == id)?;
    let mut keys: Vec<String> = def.keys.iter().map(|k| (*k).to_owned()).collect();
    // With no odyssey game item, odyssey progress has nowhere else to go.
    if id == "boss_records" && !has_odyssey_item {
        keys.push(KEY_ODYSSEY.to_owned());
    }
    Some(keys)
}

fn category_summary(changed: &[String]) -> String {
    if changed.is_empty() {
        return "no differences".to_owned();
    }
    let shown: Vec<String> = changed.iter().take(4).map(|k| humanize_key(k)).collect();
    let mut s = format!(
        "{} {} differ{}: {}",
        changed.len(),
        if changed.len() == 1 { "key" } else { "keys" },
        if changed.len() == 1 { "s" } else { "" },
        shown.join(", ")
    );
    if changed.len() > 4 {
        s.push_str(&format!(" +{} more", changed.len() - 4));
    }
    s
}

/// Compares a decoded snapshot profile with the decoded live profile.
pub fn diff(snapshot_id: &str, snapshot_name: &str, snap: &Value, live: &Value) -> RestorePreview {
    let games = game_items(snap, live);
    let has_odyssey_item = games.iter().any(|g| g.kind == "odyssey");

    let mut categories = Vec::new();
    for id in CATEGORIES.iter().map(|c| c.id).chain(std::iter::once(OTHER_ID)) {
        let keys = category_keys(id, snap, live, has_odyssey_item).unwrap_or_default();
        let changed_keys: Vec<String> = keys
            .into_iter()
            .filter(|k| snap.get(k).is_some() && snap.get(k) != live.get(k))
            .collect();
        let label = CATEGORIES
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.label)
            .unwrap_or(OTHER_LABEL);
        categories.push(CategoryDiff {
            id: id.to_owned(),
            label: label.to_owned(),
            changed: !changed_keys.is_empty(),
            summary: category_summary(&changed_keys),
            changed_keys,
        });
    }

    let locked = LOCKED_KEYS
        .iter()
        .filter(|k| snap.get(**k).is_some() || live.get(**k).is_some())
        .map(|k| LockedDiff {
            key: (*k).to_owned(),
            label: locked_label(k),
            snapshot: display_value(snap.get(*k)),
            live: display_value(live.get(*k)),
            differs: snap.get(*k) != live.get(*k),
        })
        .collect();

    let version = |p: &Value| {
        p.get("savedByGameVersion")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let identical = games
        .iter()
        .all(|g| !g.default_checked && !g.event.as_ref().map_or(false, |e| e.changed))
        && categories.iter().all(|c| !c.changed);

    RestorePreview {
        snapshot_id: snapshot_id.to_owned(),
        snapshot_name: snapshot_name.to_owned(),
        merge_available: true,
        merge_error: None,
        games,
        categories,
        locked,
        identical,
        snapshot_game_version: version(snap),
        live_game_version: version(live),
        map_save_present: false,
        map_save_has_game: false,
        map_save_item: None,
    }
}

/// The game item a `MapSaveData` autosave belongs to: same group key as its `savedMapsId`
/// and the same `gameId` as the snapshot's entry. `None` for the idle `null` file or an
/// autosave of a run the snapshot does not hold.
pub fn map_save_item(map_save_bytes: &[u8], games: &[GameItem]) -> Option<String> {
    let v = saveformat::decode_value(map_save_bytes).ok()?;
    let key = v.get("savedMapsId").and_then(Value::as_str).filter(|s| !s.is_empty())?;
    let info = saved_map_info(key, &v);
    let id = format!("game:{}", group_key(key));
    games
        .iter()
        .find(|g| g.id == id && g.snapshot.as_ref().map_or(false, |s| s.game_id == info.game_id))
        .map(|g| g.id.clone())
}

// ---- Merge ---------------------------------------------------------------------------------

pub struct MergeOutcome {
    pub json: Value,
    /// Human labels of what was taken.
    pub applied: Vec<String>,
    /// Ids of the game items that actually changed the live `savedMaps`.
    pub restored_games: Vec<String>,
}

/// Live profile with the selected parts replaced from the snapshot. Never deletes a top-level
/// key and never touches a locked key.
pub fn build_merged(
    live: &Value,
    snap: &Value,
    preview: &RestorePreview,
    sel: &RestoreSelection,
) -> Result<MergeOutcome, String> {
    if sel.is_empty() {
        return Err("Nothing selected".to_owned());
    }
    let mut merged = live.clone();
    let out = merged
        .as_object_mut()
        .ok_or_else(|| "live profile is not a JSON object".to_owned())?;
    let snap_obj = snap
        .as_object()
        .ok_or_else(|| "snapshot profile is not a JSON object".to_owned())?;
    let mut applied = Vec::new();

    // Categories first: whole-key copies.
    let has_odyssey_item = preview.games.iter().any(|g| g.kind == "odyssey");
    for id in &sel.categories {
        let cat = preview
            .categories
            .iter()
            .find(|c| &c.id == id)
            .ok_or_else(|| format!("unknown category '{id}'"))?;
        let keys = category_keys(id, snap, live, has_odyssey_item).unwrap_or_default();
        for k in keys {
            if is_locked(&k) {
                continue;
            }
            if let Some(v) = snap_obj.get(&k) {
                out.insert(k, v.clone());
            }
        }
        if cat.changed {
            applied.push(cat.label.clone());
        }
    }

    // Game items: make the live slots equal the snapshot slots for the item's keys.
    let mut restored_games = Vec::new();
    let snap_maps = saved_maps(snap).cloned().unwrap_or_default();
    for id in &sel.games {
        let item = preview
            .games
            .iter()
            .find(|g| &g.id == id)
            .ok_or_else(|| format!("unknown game item '{id}'"))?;
        if !matches!(item.status, ItemStatus::Add | ItemStatus::Replace) {
            continue;
        }
        let slot = out
            .entry(KEY_SAVED_MAPS.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        if !slot.is_object() {
            *slot = Value::Object(Map::new());
        }
        let live_maps = slot.as_object_mut().expect("just ensured");
        for k in &item.keys {
            match snap_maps.get(k) {
                Some(v) => {
                    live_maps.insert(k.clone(), v.clone());
                }
                None => {
                    live_maps.shift_remove(k);
                }
            }
        }
        restored_games.push(item.id.clone());
        applied.push(item.label.clone());
    }

    // Event progress tied to game items.
    for id in &sel.events {
        let item = preview
            .games
            .iter()
            .find(|g| &g.id == id)
            .ok_or_else(|| format!("unknown game item '{id}'"))?;
        let Some(event) = &item.event else { continue };
        if !event.changed {
            continue;
        }
        if event.kind == "odyssey" {
            if let Some(v) = snap_obj.get(KEY_ODYSSEY) {
                out.insert(KEY_ODYSSEY.to_owned(), v.clone());
                applied.push("Odyssey progress".to_owned());
            }
            continue;
        }
        // The preview looked the event up by whichever side holds the game (a live-only run
        // can still have a differing event record in the snapshot), so resolve it the same way.
        let Some(event_id) = item
            .snapshot
            .as_ref()
            .or(item.live.as_ref())
            .and_then(|i| i.challenge_id.clone())
        else {
            continue;
        };
        let Some(entry) = find_boss_event(snap, &event_id).cloned() else { continue };
        let slot = out
            .entry(KEY_BOSS_EVENTS.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
        if !slot.is_array() {
            *slot = Value::Array(Vec::new());
        }
        let arr = slot.as_array_mut().expect("just ensured");
        match arr
            .iter_mut()
            .find(|e| e.get("eventId").and_then(Value::as_str) == Some(event_id.as_str()))
        {
            Some(existing) => *existing = entry,
            None => arr.push(entry),
        }
        applied.push(format!("{} event progress", item.label));
    }

    // Locked keys always come from the live profile, whatever the selection said.
    for k in LOCKED_KEYS {
        match live.get(*k) {
            Some(v) => {
                out.insert((*k).to_owned(), v.clone());
            }
            None => {
                out.shift_remove(*k);
            }
        }
    }
    for k in LOCKED_KEYS {
        if out.get(*k) != live.get(*k) {
            return Err(format!("internal error: locked key '{k}' was modified"));
        }
    }

    Ok(MergeOutcome { json: merged, applied, restored_games })
}

// ---- File-level entry points ---------------------------------------------------------------

fn unavailable(id: &str, meta: &snapshot::Meta, error: String) -> RestorePreview {
    RestorePreview {
        snapshot_id: id.to_owned(),
        snapshot_name: meta.name.clone(),
        merge_available: false,
        merge_error: Some(error),
        games: Vec::new(),
        categories: Vec::new(),
        locked: Vec::new(),
        identical: false,
        snapshot_game_version: meta.game_version.clone(),
        live_game_version: None,
        map_save_present: meta.map_save_data_present,
        map_save_has_game: meta.map_save_has_game,
        map_save_item: None,
    }
}

/// The snapshot's `MapSaveData` bytes, if the folder has one.
fn snapshot_map_save(saves: &Path, id: &str) -> Result<Option<Vec<u8>>, String> {
    let dir = snapshot::snapshot_dir(saves, id)?;
    Ok(fs::read(dir.join(snapshot::MAP_SAVE_FILE)).ok())
}

struct Decoded {
    meta: snapshot::Meta,
    snap: Value,
    live_bytes: Vec<u8>,
    live: Value,
}

/// Outer error: the snapshot itself is broken. Inner error: one side cannot be decoded, so
/// only a raw restore is possible.
fn decode_both(
    saves: &Path,
    profile_path: &Path,
    id: &str,
) -> Result<Result<Decoded, (snapshot::Meta, String)>, String> {
    let dir = snapshot::snapshot_dir(saves, id)?;
    let meta = snapshot::read_meta(&dir).ok_or("snapshot has no readable meta.json")?;
    let snap_bytes = fs::read(dir.join(snapshot::PROFILE_FILE))
        .map_err(|e| format!("snapshot Profile.Save: {e}"))?;
    let snap = match saveformat::decode_value(&snap_bytes) {
        Ok(v) => v,
        Err(e) => return Ok(Err((meta, format!("snapshot could not be decoded: {e}")))),
    };
    let live_bytes = match fs::read(profile_path) {
        Ok(b) => b,
        Err(e) => return Ok(Err((meta, format!("live profile could not be read: {e}")))),
    };
    let live = match saveformat::decode_value(&live_bytes) {
        Ok(v) => v,
        Err(e) => return Ok(Err((meta, format!("live profile could not be decoded: {e}")))),
    };
    Ok(Ok(Decoded { meta, snap, live_bytes, live }))
}

/// What loading `id` would change, for the selection dialog.
pub fn preview(saves: &Path, profile_path: &Path, id: &str) -> Result<RestorePreview, String> {
    match decode_both(saves, profile_path, id)? {
        Err((meta, error)) => Ok(unavailable(id, &meta, error)),
        Ok(d) => {
            let mut p = diff(id, &d.meta.name, &d.snap, &d.live);
            p.map_save_present = d.meta.map_save_data_present;
            p.map_save_has_game = d.meta.map_save_has_game;
            p.map_save_item = snapshot_map_save(saves, id)?
                .and_then(|b| map_save_item(&b, &p.games));
            Ok(p)
        }
    }
}

/// Applies a selection: raw copy, or merge + re-encrypt + atomic write. The caller checks the
/// game state and takes the backup.
pub fn apply(
    saves: &Path,
    profile_path: &Path,
    map_save_path: &Path,
    id: &str,
    sel: &RestoreSelection,
) -> Result<RestoreReport, String> {
    if sel.raw {
        return snapshot::restore(saves, profile_path, map_save_path, id);
    }
    if sel.is_empty() {
        return Err("Nothing selected".to_owned());
    }
    let d = match decode_both(saves, profile_path, id)? {
        Ok(d) => d,
        Err((_, e)) => return Err(format!("{e}; only a raw overwrite is possible")),
    };
    // Recompute the preview from the files on disk; the dialog's copy may be stale.
    let preview = diff(id, &d.meta.name, &d.snap, &d.live);
    let outcome = build_merged(&d.live, &d.snap, &preview, sel)?;
    if outcome.applied.is_empty() {
        return Err("Nothing selected differs from the current profile".to_owned());
    }

    let json = serde_json::to_string(&outcome.json).map_err(|e| e.to_string())?;
    let header = saveformat::parse_container(&d.live_bytes)
        .map_err(|e| format!("live Profile.Save header: {e}"))?
        .header_bytes;
    let bytes = saveformat::encode_profile(header, &json).map_err(|e| e.to_string())?;
    // Self-check before touching the live file.
    let back = saveformat::decode_json(&bytes).map_err(|e| format!("self-check decode: {e}"))?;
    if back != json {
        return Err("self-check failed: re-encoded profile does not decode to the merged JSON".to_owned());
    }
    saveformat::extract_profile_info(&json).map_err(|e| format!("self-check parse: {e}"))?;

    snapshot::atomic_write(profile_path, &bytes)
        .map_err(|e| format!("writing {}: {e}", profile_path.display()))?;

    // The snapshot's mid-run autosave is written only when it belongs to a run that was just
    // restored; an idle `null` autosave or one of an unselected run leaves the live file alone.
    let dir = snapshot::snapshot_dir(saves, id)?;
    let mut map_save_written = None;
    if let Some(b) = snapshot_map_save(saves, id)? {
        let owner = map_save_item(&b, &preview.games);
        if owner.map_or(false, |o| outcome.restored_games.contains(&o)) {
            snapshot::atomic_write(map_save_path, &b).map_err(|e| {
                format!("Profile.Save restored, but writing {} failed: {e}", map_save_path.display())
            })?;
            map_save_written = Some(map_save_path.display().to_string());
        }
    }

    let mut meta = d.meta;
    meta.last_restored_at = Some(Utc::now());
    snapshot::write_meta(&dir, &meta)?;
    Ok(RestoreReport {
        meta,
        profile_written: profile_path.display().to_string(),
        map_save_written,
        mode: "merge".to_owned(),
        applied: outcome.applied,
        backup: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saveformat::testutil::{encrypted_profile, map_save};
    use serde_json::json;

    /// Every top-level key seen in a real 56.3 profile.
    const REAL_KEYS: &[&str] = &[
        "savedByGameVersion", "savedBySkuId", "towerXp", "acquiredUpgrades", "viewedUpgrades", "paragonUpgradesPurchased",
        "acquiredKnowledge", "paidForKnowledge", "knowledgeDisabled", "newKnowledgePoints", "unlockedTowers", "unlockedHeroes",
        "unlockedTowerSkins", "seenUnlockedNotification", "seenUnlockedHeroes", "seenNewTowerNotifications", "seenNewHeroNotification", "seenNewTowerSkinNotification",
        "mapInfo", "UnlockAllModes", "seenEvents", "seenPowers", "rateMeSkuVersionNumber", "countGamesSinceSkuRateMeChange",
        "completedGamesSinceSkuRateMeChange", "completedGame", "selectedTowerSkinData", "powers", "powersData", "instaTowers",
        "savedMaps", "isPendingNoLinkIdMigration", "noLinkIdVersion", "ownerID", "trophiesWalletId", "teamTrophiesWalletId",
        "unclaimedTrophies", "timeStamp", "monkeyMoney", "xp", "rank", "veteranXp",
        "veteranRank", "seenVeteranRankInfo", "levelCapWas", "trophies", "lifetimeTrophies", "lifetimeTeamTrophies",
        "knowledgePoints", "primaryHero", "secondaryHero", "achievementsProgress", "achievementsClaimed", "achievementsSeen",
        "achievementsPostedToXBoxLive", "achievementsPostedToGooglePlay", "analyticsKonFuze", "highestSeenRound", "highestSeenRoundCurrentVersion", "purchase",
        "lastReceivedInboxMessage", "dailyRewardIndex", "lastSavedUTCTime", "nextDailyRewardDateTime", "totalDailyChallengesCompleted", "consecutiveDailyChallengesCompleted",
        "uniqueCompletedDailyChallengeIds", "raceMedalData", "bossBadgeData", "bossMedals", "bossLeaderboardMedals", "bossLeaderboardEliteMedals",
        "ctLeaderboardBadges", "ctGlobalLeaderboardBadges", "seenMiniRace", "totalRacesEntered", "raceBestTimeForAchievements", "challengeEditorModel",
        "mapEditorData", "hasUnlockedMapEditor", "unlockedPropPacks", "seenMapEditorInfoPopup", "seenMapBrowser", "seenGameBrowser",
        "autosetSpecialConditions", "submittedChallengeEditorID", "submittedOdysseyEditorID", "seenChallengeModifiedPopup", "inGameSettings", "languageCode",
        "challengesPlayed", "challengesShared", "winsWithCustomHeroSkin", "billGreates", "aCrateTime", "collectionEventMapBonusData",
        "odysseySaveData", "odysseyEditorSaveData", "odysseysEditorData", "embarkedOdysseyEditorDcm", "completedOdysseys", "totalCompletedOdysseys",
        "cancelledFacebookFriendsPopup", "coopQuickMatchSetting", "coopMatchSetToPrivate", "currentCoopGameDetails", "hotkeysData", "hotkeysData2",
        "hasSeenNewDoubleCash", "seenBigBloons", "unlockedBigBloons", "bigBloonsActive", "seenSmallBloons", "unlockedSmallBloons",
        "smallBloonsActive", "seenBigTowers", "unlockedBigTowers", "bigTowersActive", "seenSmallTowers", "unlockedSmallTowers",
        "smallTowersActive", "smallBossesActive", "seenSmallBosses", "unlockedSmallBosses", "seenFastTrack", "unlockedFastTrack",
        "fastTrackActive", "patWinsOn10Release", "oompaLoompad", "collectionEventData", "savedPlayList", "useJukeBox",
        "trophyStorePurchasedItems", "namedMonkeyNames", "favoriteTrophyItems", "savedStats", "profileAvatar", "profileAvatarFrame",
        "profileBanner", "seenProfileStats", "savedNamedMonkeyStats", "trophyStoreSeen", "noStoneUnturned", "moProblems",
        "fullSpeed", "transformicTonicUsesOn20Release", "currentTowerGiftProgress", "trophiesSpent", "hostedCoopGames", "collectionEventCratesOpened",
        "collectionEventCratesTypesOpened", "continuesUsed", "blockedHostnames", "selectedContentTab", "goldenBloonData", "goldenBloonsPopped",
        "monkeyTeamsWins", "monkeyTeamsData", "giftedAchievements", "unverifiedRacePassClaims", "isBossRankedSelected", "isBossEliteSelected",
        "bossesEventData", "questsEventData", "playedDailyChallengeIds", "lostDailyChallengeIds", "wonDailyChallengeIds", "numWonUniqueDailyChallenges",
        "friendScreenShowCode", "coopLobbyScreenShowCode", "toggleFriendRequests", "numAwaitingFriendRequests", "guildLastCheckedMessage", "guildInviteChecked",
        "teamInvitationsSent", "isInTeam", "colorBlindModeOn", "ctScorePanelCollapsed", "dataConsent", "analyticsConsent",
        "ctSeenFirstTimePopup", "ctCapturedTileType", "toggleTeamRequests", "previousGuildId", "nexusCreatorId", "nexus",
        "isOnlineStatusEnabled", "questsSaveData", "isPendingNamedMonkeyValidation", "accessibilitySettings", "geraldoShopItemsUsed", "audioSettings",
        "seenUpcomingCtEventId", "bossChallengeScores", "bossChallengeScores2p", "bossChallengeScores3p", "bossChallengeScores4p", "selectedChallengeBoss",
        "selectedChallengeBossMap", "selectedChallengeBossRanked", "selectedChallengeBossScoring", "seenBossChallenge", "uniqueBossesDefeated", "heavyInvestment",
        "hideAccoladeNotifs", "hasPublicAccolades", "towerUnlockProgresses", "selectedTowerForUnlockProgression", "gameEditorData", "legendsData",
        "seenLegends", "seenLegendUpdate", "seenStorePipIds", "iapLoginReminderCount", "completedBossEvents", "currentSweepstakesStats",
        "seenDialogues", "socialSeasonBadgesSequential", "championSocialSeasons", "giftsReceived", "receivedGiftEventIds", "pendingGifts",
        "claimedGifts", "seenMessageEventIds", "deletedMessageEventIds", "tutorialTypeSelected", "powersProSaveData", "privateMessages",
        "pendingSteamAttribution", "shownLegendPopups", "perksToggledOn", "claimedSocialSeasonRewards", "seenSocialSeasonEventIds", "socialSeasonIdsPendingEndPopup",
        "profilePerkData", "rewardedSsBadges", "seasonStart", "midSeason", "seasonChampion", "lastOwnTeamIdSeen",
        "lastOwnTeamUiOpenTime", "notificationsSettings", "notificationPopupInfo", "timesShownNotificationPopup", "neverShowNotificationPopup", "lastSeenMainMenuPopupTime",
        "hasSentDiscordMail", "modeTimers", "HasCompletedTutorial"
    ];

    fn boss(round: i64, challenge: &str, game_id: i64, elite: bool) -> Value {
        json!({
            "mapName": "FrozenOver", "modeName": "Standard", "mapDifficulty": "Medium",
            "round": round, "gameType": 4, "playerChallengeId": challenge, "gameId": game_id,
            "attemptId": 1, "gameVersion": "56.3",
            "metaData": {"BossEventIsElite": if elite { "True" } else { "False" }, "BossEventTier": "1"}
        })
    }

    fn boss_event(id: &str, tier: i64, best: i64) -> Value {
        json!({"eventId": id, "normal": {"highestCompletedRound": 0, "tierBeaten": 0},
               "elite": {"highestCompletedRound": best, "tierBeaten": tier}, "hasClaimedRewards": false})
    }

    fn snap_profile() -> Value {
        json!({
            "savedByGameVersion": "56.3", "monkeyMoney": 8042, "xp": 1.5, "rank": 143, "trophies": 445,
            "towerXp": {"DartMonkey": 100.0}, "mapInfo": {"maps": {"Cubism": 1}},
            "savedMaps": {
                "Cubism": {"mapName": "Cubism", "modeName": "Standard", "mapDifficulty": "Easy", "round": 297, "gameType": 0, "gameId": 1254, "attemptId": 13},
                "BB": boss(59, "Lych61_mtw1v6de", 2152, true),
                "BBCONT": boss(40, "Lych61_mtw1v6de", 2152, true),
                "OD": {"mapName": "LotusIsland", "modeName": "Standard", "mapDifficulty": "Medium", "round": 20, "gameType": 3, "gameId": 2144, "attemptId": 0}
            },
            "bossesEventData": [boss_event("Lych61_mtw1v6de", 1, 58)],
            "odysseySaveData": {"activeOdysey": {"hasStarted": true, "currentOdyseyMapSaveIndex": 2, "odysseyMapDataHistory": {"0": {}, "1": {}, "2": {}, "3": {}, "4": {}}}},
            "analyticsKonFuze": {"pops": 1},
            "hotkeysData2": {"a": 1}
        })
    }

    fn live_profile() -> Value {
        json!({
            "savedByGameVersion": "56.4", "monkeyMoney": 5622, "xp": 2.5, "rank": 144, "trophies": 465,
            "towerXp": {"DartMonkey": 200.0}, "mapInfo": {"maps": {"Cubism": 1}},
            "savedMaps": {
                "Cubism": {"mapName": "Cubism", "modeName": "Standard", "mapDifficulty": "Easy", "round": 297, "gameType": 0, "gameId": 1254, "attemptId": 13},
                "Infernal": {"mapName": "Infernal", "modeName": "Impoppable", "mapDifficulty": "Hard", "round": 95, "gameType": 0, "gameId": 2160, "attemptId": 6},
                "BB": boss(30, "Bloonarius69_x", 2170, true),
                "BBCONT": boss(20, "Bloonarius69_x", 2170, true)
            },
            "bossesEventData": [boss_event("Bloonarius69_x", 2, 70)],
            "odysseySaveData": {"activeOdysey": {"hasStarted": false, "currentOdyseyMapSaveIndex": 0, "odysseyMapDataHistory": {}}},
            "analyticsKonFuze": {"pops": 2},
            "hotkeysData2": {"a": 1},
            "brandNewKey": true
        })
    }

    #[test]
    fn every_real_key_is_covered_exactly_once() {
        for k in REAL_KEYS {
            let in_table = CATEGORIES.iter().filter(|c| c.keys.contains(k)).count();
            let special = is_locked(k) || *k == KEY_SAVED_MAPS || *k == KEY_ODYSSEY;
            assert!(in_table <= 1, "{k} appears in {in_table} categories");
            if special {
                assert_eq!(in_table, 0, "{k} is special and also in a category");
                assert_eq!(category_of(k), None);
            } else {
                assert!(category_of(k).is_some(), "{k}");
            }
        }
        assert_eq!(category_of("analyticsKonFuze"), Some(OTHER_ID));
        assert_eq!(category_of("someFutureKey"), Some(OTHER_ID));
        assert_eq!(category_of("mapInfo"), Some("maps"));
        assert_eq!(category_of("monkeyMoney"), None);
        // No key appears twice inside the table either.
        let mut seen = std::collections::HashSet::new();
        for c in CATEGORIES {
            for k in c.keys {
                assert!(seen.insert(*k), "{k} listed twice");
                assert!(!is_locked(k), "{k} is locked and categorised");
            }
        }
    }

    #[test]
    fn game_items_statuses_and_grouping() {
        let items = game_items(&snap_profile(), &live_profile());
        let by_id = |id: &str| items.iter().find(|i| i.id == id).unwrap();

        let bb = by_id("game:BB");
        assert_eq!(bb.status, ItemStatus::Replace);
        assert_eq!(bb.keys, vec!["BB", "BBCONT"]);
        assert!(bb.different_game);
        assert_eq!(bb.kind, "boss");
        assert_eq!(bb.label, "Lych Elite T1 · Frozen Over");
        assert_eq!(bb.snapshot_continue_round, Some(40));
        assert_eq!(bb.live_continue_round, Some(20));
        let ev = bb.event.as_ref().unwrap();
        assert_eq!(ev.kind, "boss");
        assert_eq!(ev.snapshot.as_deref(), Some("Elite · tier 1 beaten · best round 58"));
        assert_eq!(ev.live, None);
        assert!(ev.changed);
        assert!(bb.default_checked);

        assert_eq!(by_id("game:Cubism").status, ItemStatus::Same);
        assert!(!by_id("game:Cubism").default_checked);
        assert_eq!(by_id("game:Infernal").status, ItemStatus::Kept);
        let od = by_id("game:OD");
        assert_eq!(od.status, ItemStatus::Add);
        assert_eq!(od.kind, "odyssey");
        let ev = od.event.as_ref().unwrap();
        assert_eq!(ev.kind, "odyssey");
        assert_eq!(ev.snapshot.as_deref(), Some("In progress · island 3 of 5"));
        assert_eq!(ev.live.as_deref(), Some("Not started"));
        assert!(ev.changed);

        // Changes first, then same, then kept.
        let order: Vec<ItemStatus> = items.iter().map(|i| i.status).collect();
        assert_eq!(order, vec![ItemStatus::Replace, ItemStatus::Add, ItemStatus::Same, ItemStatus::Kept]);

        // A snapshot holding only the checkpoint still yields an item.
        let mut s = snap_profile();
        s["savedMaps"].as_object_mut().unwrap().shift_remove("BB");
        let items = game_items(&s, &json!({"savedMaps": {}}));
        let bb = items.iter().find(|i| i.id == "game:BB").unwrap();
        assert_eq!(bb.keys, vec!["BBCONT"]);
        assert_eq!(bb.status, ItemStatus::Add);
    }

    #[test]
    fn diff_categories_and_locked() {
        let p = diff("id", "name", &snap_profile(), &live_profile());
        let cat = |id: &str| p.categories.iter().find(|c| c.id == id).unwrap();
        assert!(cat("towers").changed);
        assert_eq!(cat("towers").changed_keys, vec!["towerXp"]);
        assert_eq!(cat("towers").summary, "1 key differs: tower xp");
        assert!(!cat("maps").changed);
        assert_eq!(cat("maps").summary, "no differences");
        assert!(!cat("settings").changed);
        assert!(cat("other").changed);
        assert_eq!(cat("other").changed_keys, vec!["analyticsKonFuze"]);
        // Boss records differ (the live event list lacks the Lych entry) but odyssey progress is
        // owned by the OD item, not the category.
        assert_eq!(cat("boss_records").changed_keys, vec!["bossesEventData"]);

        let mm = p.locked.iter().find(|l| l.key == "monkeyMoney").unwrap();
        assert_eq!((mm.snapshot.as_str(), mm.live.as_str(), mm.differs), ("8042", "5622", true));
        assert_eq!(mm.label, "Monkey money");
        assert!(!p.identical);
        assert_eq!(p.snapshot_game_version.as_deref(), Some("56.3"));
        assert_eq!(p.live_game_version.as_deref(), Some("56.4"));

        let same = diff("id", "name", &live_profile(), &live_profile());
        assert!(same.identical);
        assert!(same.categories.iter().all(|c| !c.changed));
        assert!(same.games.iter().all(|g| g.status == ItemStatus::Same));
    }

    #[test]
    fn merge_game_replaces_slot_and_event_entry() {
        let (snap, live) = (snap_profile(), live_profile());
        let p = diff("id", "name", &snap, &live);
        let sel = RestoreSelection {
            games: vec!["game:BB".into(), "game:Cubism".into(), "game:Infernal".into()],
            events: vec!["game:BB".into()],
            ..Default::default()
        };
        let out = build_merged(&live, &snap, &p, &sel).unwrap();
        let m = &out.json;
        assert_eq!(m["savedMaps"]["BB"], snap["savedMaps"]["BB"]);
        assert_eq!(m["savedMaps"]["BBCONT"], snap["savedMaps"]["BBCONT"]);
        assert_eq!(m["savedMaps"]["Infernal"], live["savedMaps"]["Infernal"], "kept");
        assert_eq!(m["savedMaps"]["Cubism"], live["savedMaps"]["Cubism"]);
        let events = m["bossesEventData"].as_array().unwrap();
        assert_eq!(events.len(), 2, "Bloonarius kept, Lych appended");
        assert_eq!(events[0]["eventId"], "Bloonarius69_x");
        assert_eq!(events[1], snap["bossesEventData"][0]);
        assert_eq!(m["monkeyMoney"], 5622);
        assert_eq!(m["towerXp"], live["towerXp"]);
        assert_eq!(m["odysseySaveData"], live["odysseySaveData"]);
        assert_eq!(out.restored_games, vec!["game:BB"], "same/kept items are no-ops");
        assert_eq!(out.applied, vec!["Lych Elite T1 · Frozen Over", "Lych Elite T1 · Frozen Over event progress"]);
        // Key order follows the live file.
        let keys: Vec<&String> = m.as_object().unwrap().keys().collect();
        let live_keys: Vec<&String> = live.as_object().unwrap().keys().collect();
        assert_eq!(keys, live_keys);

        // Live BBCONT from another run is removed when the snapshot has only BB.
        let mut s2 = snap.clone();
        s2["savedMaps"].as_object_mut().unwrap().shift_remove("BBCONT");
        let p2 = diff("id", "name", &s2, &live);
        let out = build_merged(&live, &s2, &p2, &RestoreSelection { games: vec!["game:BB".into()], ..Default::default() }).unwrap();
        assert!(out.json["savedMaps"].get("BBCONT").is_none());

        // A live-only boss run whose event record the snapshot also holds (and differs): the
        // event is previewed from the live challenge id and must be applied from it too.
        let mut s3 = snap.clone();
        for k in ["BB", "BBCONT"] {
            s3["savedMaps"].as_object_mut().unwrap().shift_remove(k);
        }
        s3["bossesEventData"] = json!([boss_event("Bloonarius69_x", 1, 40)]);
        let p3 = diff("id", "name", &s3, &live);
        let bb = p3.games.iter().find(|g| g.id == "game:BB").unwrap();
        assert_eq!(bb.status, ItemStatus::Kept);
        let ev = bb.event.as_ref().unwrap();
        assert!(ev.changed);
        assert_eq!(ev.snapshot.as_deref(), Some("Elite · tier 1 beaten · best round 40"));
        let out = build_merged(&live, &s3, &p3, &RestoreSelection { events: vec!["game:BB".into()], ..Default::default() }).unwrap();
        assert_eq!(out.json["bossesEventData"], s3["bossesEventData"]);
        assert_eq!(out.json["savedMaps"]["BB"], live["savedMaps"]["BB"], "the run itself is kept");
        assert_eq!(out.applied, vec!["Lych Elite T1 · Frozen Over event progress".replace("Lych Elite T1", "Bloonarius Elite T1")]);
        assert!(out.restored_games.is_empty());

        // Odyssey item: savedMaps.OD added and odysseySaveData replaced.
        let out = build_merged(&live, &snap, &p, &RestoreSelection { games: vec!["game:OD".into()], events: vec!["game:OD".into()], ..Default::default() }).unwrap();
        assert_eq!(out.json["savedMaps"]["OD"], snap["savedMaps"]["OD"]);
        assert_eq!(out.json["odysseySaveData"], snap["odysseySaveData"]);
        assert_eq!(out.json["savedMaps"]["BB"], live["savedMaps"]["BB"]);
    }

    #[test]
    fn merge_categories_copy_without_deleting_and_locked_stay() {
        let (snap, live) = (snap_profile(), live_profile());
        let p = diff("id", "name", &snap, &live);
        let all_cats: Vec<String> = p.categories.iter().map(|c| c.id.clone()).collect();
        let all_games: Vec<String> = p.games.iter().map(|g| g.id.clone()).collect();
        let sel = RestoreSelection { games: all_games.clone(), events: all_games, categories: all_cats, raw: false };
        let out = build_merged(&live, &snap, &p, &sel).unwrap();
        let m = &out.json;
        for k in LOCKED_KEYS {
            assert_eq!(m.get(*k), live.get(*k), "{k}");
        }
        assert_eq!(m["monkeyMoney"], 5622);
        assert_eq!(m["savedByGameVersion"], "56.4");
        assert_eq!(m["towerXp"], snap["towerXp"]);
        assert_eq!(m["analyticsKonFuze"], snap["analyticsKonFuze"]);
        assert_eq!(m["brandNewKey"], true, "live-only key survives");
        assert!(out.applied.contains(&"Tower XP, unlocks & knowledge".to_owned()));
        assert!(!out.applied.contains(&"Map completion & medals".to_owned()), "unchanged category not reported");

        assert!(build_merged(&live, &snap, &p, &RestoreSelection::default()).is_err());
        assert!(build_merged(&live, &snap, &p, &RestoreSelection { games: vec!["game:Nope".into()], ..Default::default() }).is_err());
        assert!(build_merged(&live, &snap, &p, &RestoreSelection { categories: vec!["nope".into()], ..Default::default() }).is_err());
    }

    #[test]
    fn apply_writes_a_valid_merged_profile() {
        let root = std::env::temp_dir().join(format!("bssm-merge-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let saves = root.join("saves");
        let live_profile_path = root.join("steam").join("Profile.Save");
        let live_map = root.join("locallow").join("MapSaveData");
        fs::create_dir_all(live_profile_path.parent().unwrap()).unwrap();
        fs::create_dir_all(live_map.parent().unwrap()).unwrap();

        let v1 = encrypted_profile(&serde_json::to_string(&snap_profile()).unwrap());
        fs::write(&live_profile_path, &v1).unwrap();
        // The snapshot carries a mid-run autosave of the Lych run (round 60).
        let mut run = boss(60, "Lych61_mtw1v6de", 2152, true);
        run["savedMapsId"] = json!("BB");
        let autosave = map_save(&serde_json::to_string(&run).unwrap());
        fs::write(&live_map, &autosave).unwrap();
        let meta = snapshot::create(&saves, &live_profile_path, &live_map, "Lych run", "").unwrap();
        assert!(meta.map_save_has_game);

        // The live profile moves on; the live autosave now belongs to something else.
        let v2 = encrypted_profile(&serde_json::to_string(&live_profile()).unwrap());
        fs::write(&live_profile_path, &v2).unwrap();
        let live_autosave = map_save(r#"{"savedMapsId":"Infernal","gameId":2160,"round":96}"#);
        fs::write(&live_map, &live_autosave).unwrap();

        let p = preview(&saves, &live_profile_path, &meta.id).unwrap();
        assert!(p.merge_available);
        assert!(p.map_save_present);
        assert_eq!(p.map_save_item.as_deref(), Some("game:BB"));
        assert_eq!(p.games.iter().find(|g| g.id == "game:BB").unwrap().status, ItemStatus::Replace);

        // Restoring a different game leaves the unrelated live autosave alone.
        let sel = RestoreSelection { games: vec!["game:OD".into()], ..Default::default() };
        let report = apply(&saves, &live_profile_path, &live_map, &meta.id, &sel).unwrap();
        assert!(report.map_save_written.is_none());
        assert_eq!(fs::read(&live_map).unwrap(), live_autosave);
        fs::write(&live_profile_path, &v2).unwrap();
        fs::remove_file(&live_map).unwrap();

        // Category only: no MapSaveData written.
        let sel = RestoreSelection { categories: vec!["towers".into()], ..Default::default() };
        let report = apply(&saves, &live_profile_path, &live_map, &meta.id, &sel).unwrap();
        assert_eq!(report.mode, "merge");
        assert!(report.map_save_written.is_none());
        assert!(!live_map.exists());
        let now = saveformat::decode_value(&fs::read(&live_profile_path).unwrap()).unwrap();
        assert_eq!(now["towerXp"]["DartMonkey"].to_string(), "100.0");
        assert_eq!(now["monkeyMoney"], 5622);
        assert_eq!(now["savedMaps"]["BB"]["round"], 30);
        // Header copied from the live file, not the snapshot.
        let hdr = |b: &[u8]| saveformat::parse_container(b).unwrap().header_bytes.to_vec();
        assert_eq!(hdr(&fs::read(&live_profile_path).unwrap()), hdr(&v2));

        // Game item: savedMaps + event + MapSaveData.
        let sel = RestoreSelection { games: vec!["game:BB".into()], events: vec!["game:BB".into()], ..Default::default() };
        let report = apply(&saves, &live_profile_path, &live_map, &meta.id, &sel).unwrap();
        assert!(report.map_save_written.is_some());
        assert_eq!(fs::read(&live_map).unwrap(), autosave, "the run's own autosave comes along");
        assert!(report.meta.last_restored_at.is_some());
        assert_eq!(report.applied.len(), 2);
        let now = saveformat::decode_value(&fs::read(&live_profile_path).unwrap()).unwrap();
        assert_eq!(now["savedMaps"]["BB"]["round"], 59);
        assert_eq!(now["savedMaps"]["Infernal"]["round"], 95);
        assert_eq!(now["monkeyMoney"], 5622);
        assert_eq!(now["savedByGameVersion"], "56.4");
        assert_eq!(now["bossesEventData"].as_array().unwrap().len(), 2);
        let info = saveformat::read_profile_info(&fs::read(&live_profile_path).unwrap()).unwrap();
        assert_eq!(info.summary.as_deref(), Some("Infernal · Impoppable · R95"));
        assert!(!live_profile_path.with_file_name("Profile.Save.bssm-tmp").exists());

        // Selecting only unchanged things is rejected; empty and raw behave as before.
        let sel = RestoreSelection { games: vec!["game:Cubism".into()], ..Default::default() };
        assert!(apply(&saves, &live_profile_path, &live_map, &meta.id, &sel).is_err());
        assert!(apply(&saves, &live_profile_path, &live_map, &meta.id, &RestoreSelection::default()).is_err());
        let report = apply(&saves, &live_profile_path, &live_map, &meta.id, &RestoreSelection { raw: true, ..Default::default() }).unwrap();
        assert_eq!(report.mode, "raw");
        assert_eq!(fs::read(&live_profile_path).unwrap(), v1);

        // An idle `null` autosave in the snapshot never touches the live one.
        fs::write(&live_profile_path, &v2).unwrap();
        fs::write(&live_map, &live_autosave).unwrap();
        fs::write(saves.join(&meta.id).join(snapshot::MAP_SAVE_FILE), map_save("null")).unwrap();
        let p = preview(&saves, &live_profile_path, &meta.id).unwrap();
        assert_eq!(p.map_save_item, None);
        let sel = RestoreSelection { games: vec!["game:BB".into()], ..Default::default() };
        let report = apply(&saves, &live_profile_path, &live_map, &meta.id, &sel).unwrap();
        assert!(report.map_save_written.is_none());
        assert_eq!(fs::read(&live_map).unwrap(), live_autosave);

        // Unreadable snapshot: preview says raw only, merge apply refuses.
        fs::write(saves.join(&meta.id).join(snapshot::PROFILE_FILE), b"garbage").unwrap();
        let p = preview(&saves, &live_profile_path, &meta.id).unwrap();
        assert!(!p.merge_available);
        assert!(p.merge_error.is_some());
        let sel = RestoreSelection { categories: vec!["towers".into()], ..Default::default() };
        assert!(apply(&saves, &live_profile_path, &live_map, &meta.id, &sel).is_err());

        fs::remove_dir_all(&root).unwrap();
    }
}

#[cfg(test)]
mod live_fixture_tests {
    use super::*;
    use std::path::PathBuf;

    /// Optional: with the real live `Profile.Save` and an older `Snapshot.Save` in
    /// `src-tauri/fixtures/`, print the preview and run a full merge. Skipped when absent.
    #[test]
    fn live_fixtures_preview_and_merge() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let (Ok(live_bytes), Ok(snap_bytes)) =
            (fs::read(dir.join("Profile.Save")), fs::read(dir.join("Snapshot.Save")))
        else {
            return;
        };
        let live = saveformat::decode_value(&live_bytes).unwrap();
        let snap = saveformat::decode_value(&snap_bytes).unwrap();
        let p = diff("snap", "snap", &snap, &live);
        for g in &p.games {
            println!("game {:?} {:?} keys={:?} label={} ev={:?}", g.id, g.status, g.keys, g.label, g.event);
        }
        for c in &p.categories {
            println!("cat {} changed={} {}", c.id, c.changed, c.summary);
        }
        for l in &p.locked {
            println!("locked {} {} -> {} differs={}", l.key, l.snapshot, l.live, l.differs);
        }
        let same = diff("live", "live", &live, &live);
        assert!(same.identical);

        // Every changed game + event and every category: merged output still decodes,
        // locked keys untouched, and the re-encoded file parses.
        let sel = RestoreSelection {
            games: p.games.iter().filter(|g| g.default_checked).map(|g| g.id.clone()).collect(),
            events: p.games.iter().filter(|g| g.default_checked).map(|g| g.id.clone()).collect(),
            categories: Vec::new(),
            raw: false,
        };
        let out = build_merged(&live, &snap, &p, &sel).unwrap();
        for k in LOCKED_KEYS {
            assert_eq!(out.json.get(*k), live.get(*k), "{k}");
        }
        let json = serde_json::to_string(&out.json).unwrap();
        let header = saveformat::parse_container(&live_bytes).unwrap().header_bytes;
        let bytes = saveformat::encode_profile(header, &json).unwrap();
        assert_eq!(saveformat::decode_json(&bytes).unwrap(), json);
        let info = saveformat::read_profile_info(&bytes).unwrap();
        println!("merged summary: {:?} maps={:?}", info.summary, info.saved_maps.iter().map(|m| m.key.clone()).collect::<Vec<_>>());
        println!("applied: {:?}", out.applied);
    }
}
