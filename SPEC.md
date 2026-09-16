# BTD6 Save State Manager — Spec

A small Windows desktop app for making named snapshots of Bloons TD 6 save data and
restoring them later. Built for one use case: retrying map saves and boss event runs.

Status: v1 implemented (2026-09-12). Selective restore (step 9) implemented 2026-09-15; its
in-game round trip is pending.

---

## 1. Goal and non-goals

**Goal.** From the BTD6 main menu, click one button, type a name, and get a snapshot.
Later, with BTD6 closed, pick a snapshot and restore it. That is the whole loop.

**Non-goals.**
- No free-form editing of save contents. The only write of decoded data is the selective
  restore (section 3.3), which copies whole values from a snapshot into the live profile.
- No Epic Games install support. Steam only.
- No launching BTD6 from the app.
- No tray icon, always-on-top, or global hotkeys. It is a normal small window.
- No cloud or NK-account handling.

---

## 2. What we learned about BTD6 saves (investigation, 2026-09-12)

Decrypted the live files read-only using the format documented by community tools
(BTD6-Save-Converter, monke). Findings:

### 2.1 The two files that matter

| File | Location | Contents |
|---|---|---|
| `Profile.Save` | `C:\Program Files (x86)\Steam\userdata\1276140654\960090\local\link\PRODUCTION\current\` | Whole player profile, encrypted. Includes `savedMaps`, a dictionary of complete in-progress game states. |
| `MapSaveData` | `%USERPROFILE%\AppData\LocalLow\Ninja Kiwi\BloonsTD6\` | Live autosave of the currently running game, unencrypted zlib. Rewritten every round. Set to `null` (60 bytes) when you exit to the menu. On 2026-09-15 the file was absent altogether (the game had deleted it), so treat it as optional. |

Both directories also hold `.bak` (previous version) and `.tmp` (write-in-progress) siblings
that the game manages itself. We leave those alone.

### 2.2 Profile.Save is the source of truth for in-progress games

`savedMaps` inside the profile held five full game states at the time of inspection:

| key | map | mode | round | notes |
|---|---|---|---|---|
| `BB` | FrozenOver | Standard, gameType 4 | 59 | Lych boss event, `playerChallengeId: Lych61_mtw1v6de`, metaData has `BossEventIsElite`, `BossEventTier` |
| `BBCONT` | FrozenOver | same | 40 | boss "continue" checkpoint, same challenge id |
| `Cubism` | Cubism | Standard, gameType 0 | 297 | freeplay, matchWon true |
| `Peninsula` | Peninsula | Clicks, gameType 0 | 100 | |
| `OD` | LotusIsland | Standard, gameType 3 | 20 | odyssey |

Each entry is the same shape as the standalone `MapSaveData` file: `placedTowers`,
`players` (cash, hero), `health`, `round`, `bloons`, `metaData`, and so on.

### 2.3 How the two files hand off

Observed timeline from file timestamps and decrypted contents:

1. 11:24:13 `Profile.bak` written. Lych run at round 50.
2. 11:25:50 and 11:26:16 `MapSaveData.bak` / `.tmp` written. Lych run at rounds 58 and 59.
3. 11:26:53 exit to menu. `MapSaveData` rewritten as `null`. `Profile.Save` written with the Lych run at round 59.

So: while playing, `MapSaveData` is the per-round autosave. On exit to menu the game folds
that state into `Profile.Save.savedMaps` and clears `MapSaveData`.

**Conclusion.** Copying `Profile.Save` alone restores map saves and boss runs, provided the
snapshot is taken from the main menu after exiting the game. This matches the older video and
the existing community tools. We copy `MapSaveData` as well so that a crashed mid-run
autosave can never be offered by the game on top of a restored profile. It costs 60 bytes when idle.

### 2.4 Other facts that shape the design

- Profile `ownerID` starts with `no_link`, so this profile is not linked to a Ninja Kiwi
  account. No NK-side sync to fight with.
- Steam Cloud for BTD6 is already disabled by the user. Documented as a precondition, not handled.
- `savedByGameVersion` is `56.3`. Community tools warn that restoring a profile from a different
  game version can drop trophy store items. The app records the version per snapshot and warns
  on mismatch, but does not block.
- The profile header carries a save counter (5275 at inspection). Restoring an older file with a
  lower counter is what every community tool does; no evidence the game rejects it.
- The BTD6 install directory was not found on `C:`. The process name `BloonsTD6.exe` is the
  standard one and is confirmed on first run.

### 2.5 File format (needed for read-only inspection)

Both files share one container. Offsets are little-endian.

```
0x00  u32  version            = 2
0x04  u32  headerLen          = bytes that follow this field, before payload (71 for Profile, 37 for MapSaveData)
0x08  u32  saveCount          increments on every write
0x0C  [16] guid
0x1C  [8]  unknown
0x24  u64  timestamp          .NET DateTime binary
0x2C  str  7-bit-length-prefixed string  ("\"<32 hex>\"" in Profile, empty in MapSaveData)
----  payload starts at 8 + headerLen
```

Payload, Profile.Save (encrypted):

```
u64  passwordIndex = 2
[24] salt
[..] AES-128-CBC, PKCS7   ->  zlib stream  ->  UTF-8 JSON (with BOM)
key/iv = PBKDF2-HMAC-SHA1(password "11", salt, 10 iterations, 32 bytes); iv = [0..16], key = [16..32]
```

Payload, MapSaveData (not encrypted):

```
[..] zlib stream (starts 78 5E) -> UTF-8 JSON, or the literal `null`
```

Verified against the live files: both decrypt and parse cleanly.

Encoding (used by the selective restore) is the same pipeline backwards: BOM + compact JSON
-> zlib level 2 (the game's `78 5E` header) -> fresh random 24-byte salt -> AES-128-CBC/PKCS7
-> `u64 2` + salt + ciphertext, appended to the live file's header bytes copied verbatim.
The 32-hex header string is **not** a checksum of the payload: `Profile.Save` and
`Profile.bak` carry the same string with different payloads, and it changed between days. The
guid and the 8 unknown bytes were constant across every file inspected.

The decoded JSON is one object with ~249 top-level keys and no whitespace. Numbers are written
by C# in round-trip format (up to 17 significant digits, thousands of `x.0` literals), so the
app parses with serde_json `arbitrary_precision` and re-serialises the untouched parts
byte-for-byte, except that C#'s `1E+08` comes back as `1e+08` (same value; the live-fixture
test checks this is the only difference).

### 2.6 What the profile holds (inspection 2026-09-15)

Compared the 2026-09-12 snapshot with the live profile after two days of play.

- `savedMaps` is a dictionary keyed by `savedMapsId`: regular map saves by map name (`Cubism`,
  `Infernal`), the boss run as `BB` plus a continue checkpoint `BBCONT` (same
  `playerChallengeId`, lower round, its own `savedMapsId` is still `BB`), the odyssey as `OD`.
  Each entry carries its own `gameVersion`; a 56.1 entry sits happily in a 56.3 profile.
- Boss event progress is separate: `bossesEventData` is an array of
  `{eventId, normal: {tierBeaten, highestCompletedRound, ...}, elite: {...}, ...}` where
  `eventId` equals the run's `playerChallengeId` (`Lych61_mtw1v6de`).
- Odyssey progress: `odysseySaveData.activeOdysey` (`hasStarted`, `currentOdyseyMapSaveIndex`,
  `odysseyMapDataHistory`) plus `difficultySaveData`.
- Currencies and counters: `monkeyMoney`, `xp`, `rank`, `trophies`, `lifetimeTrophies`,
  `trophiesSpent`, `knowledgePoints`, `veteranXp`, `veteranRank`.
- Everything else falls into recognisable groups: `mapInfo` (per-map medal bitfields),
  `towerXp` + unlock lists, `powersData`/`instaTowers`/`trophyStorePurchasedItems`,
  `achievements*`, daily-challenge and event lists, settings/hotkeys/cosmetics, and
  `analyticsKonFuze` (69 KB of stats). The full key -> category table lives in
  `src-tauri/src/restore.rs`; a unit test checks every key seen in the real profile is covered.

---

## 3. User-facing behaviour

### 3.1 Window

- Normal small window, roughly 380 x 520, resizable. Position and size are remembered across launches.
- Header row: app name, a status dot, and a **Save current** button.
- Status dot polls every 2 s. Green: BTD6 not running, all actions enabled. Red: BTD6 running,
  **Load** is disabled with tooltip "Close BTD6 first". **Save** stays enabled while the game
  runs but shows a hint: "Best taken from the main menu."
- Below: the snapshot list, newest first. Each row shows:
  - name (editable inline)
  - timestamp (relative, e.g. "12 min ago", full date on hover)
  - auto-info line derived from the snapshot, e.g. `Lych Elite T1 · Frozen Over · R59` and `Rank 143 · 8,042 MM`
  - note (editable inline, optional)
  - actions: **Load**, **Rename**, **Delete**
- Empty state: one line explaining what a snapshot captures and where files come from.

### 3.2 Save current

1. Name dialog with an optional note. Default name is the auto-info of the current live profile,
   e.g. `Lych Elite · Frozen Over · R59`, so pressing Enter is usually enough.
2. Copies `Profile.Save` and `MapSaveData` into a new snapshot folder.
3. Decrypts the copied `Profile.Save` read-only to validate the copy and fill auto-info.
   If decryption fails the snapshot is kept but flagged "unreadable, may be corrupt".

### 3.3 Load (selective restore)

1. Disabled while BTD6 is running.
2. The app decrypts the snapshot's profile and the live profile and shows what differs:
   - **In-progress games**: one checkbox per saved game. A boss run's `BB` + `BBCONT` is one
     item; `OD` is one item. Status per item: *new* (only in the snapshot), *replaces Rn* (in
     both, different), *no change*, *kept* (only in the live profile, never removed). Boss and
     odyssey items get a sub-row for their event progress (the matching `bossesEventData`
     entry, or `odysseySaveData`). Games and their event progress are ticked by default.
   - **Also take from snapshot**: one checkbox per category, unticked, with a one-line summary
     of which keys differ: boss & odyssey records, map completion & medals, tower XP/unlocks/
     knowledge, powers/insta-monkeys/trophy store, achievements, daily challenges/events/
     seasons, settings/hotkeys/cosmetics, and "everything else" (stats, editors, any key a new
     game version adds).
   - **Always kept**: monkey money is shown with both values and cannot be selected. The same
     lock applies to XP, rank, trophies, knowledge points and identity keys (`ownerID`,
     wallets, `savedByGameVersion`, `timeStamp`, ...); those appear only when they differ.
   - **Advanced**: "Overwrite the entire file instead" is the old byte-for-byte copy. It is the
     only option when either profile cannot be decoded, and carries the cross-version warning.
3. Before writing anything, the live files are snapshotted automatically as
   "Auto backup before <name>" (flagged `auto` in `meta.json`). If that fails, nothing is loaded.
4. Merge mode starts from the live JSON, replaces the selected keys/entries with the snapshot's,
   re-copies every locked key from the live profile, re-encrypts with the live file's header,
   decodes the result as a self-check, then writes atomically. The snapshot's `MapSaveData` is
   written only when it is a mid-run autosave of a game that was just restored (matched by
   `savedMapsId` and `gameId`); an idle `null` autosave or one of an unselected run leaves the
   live file alone. Raw mode writes both files as before.
   `Profile.bak` and `.tmp` are never touched.
5. Reports success in the row ("Loaded 3 s ago").

### 3.4 Rename, edit note, delete

- Rename and note edits are inline, saved on blur or Enter.
- Delete asks for confirmation and removes the snapshot folder.

---

## 4. Storage layout

Snapshots live next to the executable. Portable, no AppData.

```
<exe dir>/
  btd6-save-state-manager.exe
  saves/
    2026-09-12_112657_lych-elite-frozen-over-r59/
      meta.json
      Profile.Save
      MapSaveData
  settings.json        window position/size, resolved BTD6 paths
```

`meta.json`:

```json
{
  "id": "2026-09-12_112657_lych-elite-frozen-over-r59",
  "name": "Lych Elite · Frozen Over · R59",
  "note": "before round 60 spike",
  "createdAt": "2026-09-12T18:26:57Z",
  "gameVersion": "56.3",
  "rank": 143,
  "monkeyMoney": 8042,
  "savedMaps": [
    { "key": "BB", "map": "FrozenOver", "mode": "Standard", "gameType": 4, "round": 59,
      "boss": "Lych", "elite": true, "tier": 1, "challengeId": "Lych61_mtw1v6de" }
  ],
  "mapSaveDataPresent": true,
  "readable": true,
  "auto": false
}
```

One folder per snapshot with its own `meta.json`. No global index file, so a bad write can only
damage one snapshot. The list is built by scanning `saves/`.

In development (`tauri dev`) the exe lives under `src-tauri/target/`, so debug builds use
`<repo>/saves/` instead. That folder is gitignored.

---

## 5. Paths and detection

- Steam root: read `HKCU\Software\Valve\Steam\SteamPath`, fall back to `C:\Program Files (x86)\Steam`.
- Profile: `<steam>\userdata\1276140654\960090\local\link\PRODUCTION\current\Profile.Save`.
  The user ID is a constant for v1. If the file is missing at startup, show a settings pane with
  a path picker rather than failing.
- MapSaveData: `%USERPROFILE%\AppData\LocalLow\Ninja Kiwi\BloonsTD6\MapSaveData`.
- Game running: a process named `BloonsTD6.exe` exists.

---

## 6. Tech stack

**Tauri v2** (Rust backend, WebView2 frontend). Toolchain confirmed on this machine:
cargo 1.94, rustc 1.94, node 24, pnpm 9, WebView2 152.

- Frontend: Vite + vanilla TypeScript, one page, no framework. Small enough not to need one.
- Backend crates: `tauri`, `serde`, `serde_json`, `sysinfo` (process check), `aes` + `cbc`,
  `pbkdf2`, `hmac`, `sha1`, `flate2` (inspection), `chrono`, `winreg` (Steam path).
- Tauri commands exposed to the frontend:
  - `get_status()` -> running flag, resolved paths, live profile auto-info
  - `list_snapshots()` -> `Vec<Meta>`
  - `create_snapshot(name, note)` -> `Meta`
  - `preview_restore(id)` -> `RestorePreview` (games, categories, locked values; read-only)
  - `restore_snapshot(id, selection)` -> `RestoreReport` (takes the auto backup, then merge or raw)
  - `rename_snapshot(id, name)`, `set_note(id, note)`, `delete_snapshot(id)`
- Frontend calls `get_status` every 2 s and re-renders the header.

### 6.1 Repo layout

```
SPEC.md
src/                 frontend (index.html, main.ts, style.css)
src-tauri/
  Cargo.toml
  tauri.conf.json
  src/
    main.rs          tauri setup, command registration
    paths.rs         Steam/LocalLow resolution, process check
    snapshot.rs      create/restore/rename/delete, auto backup, atomic writes
    restore.rs       key categories, preview diff, merge, apply (selective restore)
    saveformat.rs    header parse, decrypt/encrypt, inflate/deflate, auto-info extraction
saves/               dev-only snapshot dir, gitignored
```

---

## 7. Auto-info extraction rules

From the decrypted profile JSON:

- `rank`, `monkeyMoney`, `savedByGameVersion` straight from top level.
- For each `savedMaps` entry: `mapName`, `modeName`, `mapDifficulty`, `round`, `gameType`,
  `playerChallengeId`, `savedMapsId`.
- Boss detection: `playerChallengeId` present and `metaData` contains `BossEventIsElite`.
  Boss name is the leading alphabetic run of `playerChallengeId` (`Lych61_...` -> `Lych`).
  Elite from `BossEventIsElite == "True"`, tier from `BossEventTier`.
- Row summary picks the most recently played entry: the one with the highest `gameId`, then
  highest `attemptId`. `gameId` is a counter that grows across games (1254, 1808, 2144, 2152 in
  the inspected profile) and `ts` is always null, so this is the only recency signal available.
  Observed `gameType` values: 0 standard, 3 odyssey, 4 boss event. Whether 4 also covers other
  event types is still unverified (see section 9).
- Human map names: strip camel case (`FrozenOver` -> `Frozen Over`). No lookup table in v1.

---

## 8. Build plan

1. **Scaffold** Tauri v2 app, window sizing, settings persistence. Empty list renders.
2. **Paths + status**: resolve both files, process poll, status dot.
3. **Snapshot create/list/delete** with `meta.json`, no decryption yet.
4. **Save format**: port the header parser and decrypt path from the verified Node prototype (`tools/inspect_save.js`)
   to Rust. Unit-test against a copy of the live `Profile.Save` and `MapSaveData`.
5. **Auto-info** in rows and as the default snapshot name.
6. **Restore** with atomic writes, running-game block, version-mismatch warning.
7. **Rename / note** inline editing.
8. **Release build** and a first real round trip: snapshot, play, restore, confirm in game.
9. **Selective restore**: encoder, key categories, preview dialog, merge + self-check, auto
   backup, raw escape hatch. Done 2026-09-15; needs its own in-game round trip (load only the
   boss run from an older snapshot, confirm it resumes and monkey money is untouched).

---

## 9. Open questions to verify while building

- Exact process name of the Steam build (`BloonsTD6.exe` expected).
- Whether the game shows any recovery prompt when `MapSaveData` is restored to a non-null state.
- Whether `gameType` 4 is boss only or also covers other event types.
- Whether restoring while Steam itself is closed matters (it should not; no cloud).
