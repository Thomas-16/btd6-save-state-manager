# Project notes for Claude

Design and save-format findings live in `SPEC.md`; usage and layout in `README.md`.

## After finishing a change

Once a set of changes is done and `cargo test` plus `pnpm build` pass, offer to build the
release exe and deploy it. Ask first; do not deploy unprompted. The deploy is:

1. `pnpm tauri build` (release exe lands at `src-tauri/target/release/btd6-save-state-manager.exe`).
2. Make sure the app is not running, then copy that exe over
   `S:\Programs\btd6-save-state-manager\btd6-save-state-manager.exe`.
3. Leave `S:\Programs\btd6-save-state-manager\saves\` and `settings.json` untouched; they are
   the user's snapshots and settings, stored next to the exe by design.

## Conventions

- `python` on PATH is the Microsoft Store stub; use `node` for scripting.
- `tools/inspect_save.js` is CommonJS; copy it to a `.cjs` file to run it (package.json is
  `type: module`).
- Never write decoded save data except through the selective restore in `restore.rs`.
