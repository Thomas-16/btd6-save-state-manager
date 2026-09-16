# BTD6 Save State Manager

A small Windows desktop app for making named snapshots of Bloons TD 6 save data and
restoring them later. Built for one use case: retrying map saves and boss event runs.
Design and investigation notes live in [SPEC.md](SPEC.md).

## Use

1. Exit to the BTD6 main menu (or close the game).
2. Click **Save current**, accept the suggested name (for example `Lych Elite T1 · Frozen Over · R59`), press Enter.
3. Play. When you want to retry, close BTD6, pick the snapshot in the left column and click
   **Load this snapshot**. A dialog lists what differs: each in-progress game (boss runs with
   their event progress, odysseys, map saves) is a checkbox, ticked by default; other groups
   (medals, tower XP, powers, achievements, events, settings, everything else) are unticked;
   monkey money, XP, rank and trophies are always kept. Tick what you want and apply.

The window is a two-column split: snapshots on the left, the selected one on the right with
its rank, monkey money, game version, and the in-progress games it holds. The status chip in
the top bar is green when BTD6 is not running and red when it is. **Load** is disabled while
the game runs. Names and notes are edited inline in the detail pane; Ctrl+S takes a snapshot.

Snapshots are folders under `saves/` next to the executable, each holding `Profile.Save`,
`MapSaveData` and a `meta.json`. There is no global index, so you can delete or copy
folders by hand. Window position and path overrides live in `settings.json` next to the
executable.

Preconditions: Steam build of BTD6, Steam Cloud for BTD6 disabled. The Steam user id is
fixed in `src-tauri/src/paths.rs`; if `Profile.Save` is not found at startup the app opens
the settings dialog where you can point it at the right file.

## Develop

Requires Rust (stable, MSVC), Node 24, pnpm 9, and WebView2.

```
pnpm install
pnpm tauri dev      # debug build; snapshots go to <repo>/saves/
pnpm tauri build    # release exe + NSIS installer under src-tauri/target/release/
cd src-tauri && cargo test
```

Layout:

```
src/                 frontend (index.html at root, main.ts, style.css); vanilla TypeScript
src-tauri/src/
  lib.rs             Tauri setup, commands, window-state persistence
  paths.rs           Steam/LocalLow resolution, running-game check
  settings.rs        settings.json
  snapshot.rs        create/restore/rename/note/delete, auto backup, atomic writes
  restore.rs         key categories, preview diff, merge and apply for selective loads
  saveformat.rs      container header parse, decrypt/encrypt, inflate/deflate, auto-info extraction
tools/inspect_save.js  read-only Node prototype that dumps a save as JSON (copy to .cjs to run)
```

The unit tests `live_fixtures_if_present` and `live_fixtures_preview_and_merge` use real
files if you copy `Profile.Save`, `MapSaveData` and an older profile as `Snapshot.Save` into
`src-tauri/fixtures/` (gitignored). They are skipped otherwise.

A load first snapshots the live files automatically ("Auto backup before ..."), then merges
the selected parts of the snapshot into the live profile, re-encrypts it with a fresh salt,
decodes the result as a self-check and writes it via a temp file plus rename. The advanced
"overwrite the entire file" option copies the snapshot's bytes as v1 did.
