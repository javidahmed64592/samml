# SAMML - San Andreas Mod Manager Linux

A Linux-only mod manager for GTA: San Andreas (v1.0, Steam). Rust throughout: `samml-core` (logic), `samml-cli`, and a planned Tauri + Leptos GUI. Mods are symlinked from a staging directory into the game directory; nothing is copied at deploy time.

Linux is the only supported platform. Unix-only APIs (e.g. symlinks) are fine.

## Product goal

The UI manages everything. Players never open mod folders, `app_manifest.json` or profile files themselves.

## Workspace layout

```
crates/samml-core/   all logic; no printing, no argument parsing
  config.rs          load the app manifest
  slug.rs            display name -> handle, uniqueness
  paths.rs           validate/join user-supplied relative paths
  mods.rs            edit a profile's mod list in memory (add/remove/rename/move/...)
  deploy.rs          teardown / link / deploy (all-or-nothing) / clean
  launch.rs          launching the game through Steam
  lib.rs             module list only
  manifest.rs        serde data types only (no functions with side effects)
  profiles.rs        profile files: list, create, save, rename, duplicate, delete
  receipt.rs         read/write/remove the deploy receipt
crates/samml-cli/    thin wrapper: parses args, calls core, prints
src-tauri/           (planned) Tauri commands, one-line calls into core
ui/                  (planned) Leptos frontend compiled to WASM with Trunk
tests/               fixtures shared by tests (app_manifest.json, profiles/)
```

### Conventions

- Slugs and any other UI-supplied names are validated before they touch a path.
- Core never prints and never reads `std::env::args`; it returns data or `anyhow::Result`.
- One concern per module. A module's top doc comment says what it owns and which modules it depends on. New logic goes in the module for that concern; do not grow `lib.rs`.
- `manifest.rs` holds data types only. Types that cross the Tauri boundary derive `Serialize` and `Deserialize`.
- Core functions take explicit paths or `&AppManifest`; no hidden global state.
- Every behaviour change has a test. Filesystem tests use `tempfile`; fixtures live in `tests/`.
- Checks before every PR: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`.
- `app_manifest.json` is the developer's local config and is gitignored; `app_manifest.example.json` is the committed template.

## Concepts

- **GAME_DIR / STAGING_DIR / PROFILES_DIR**: set in the app manifest.
- **Profile**: `PROFILES_DIR/<slug>.json`; an ordered list of mods. The slug is the stable handle and does not change when the profile is renamed.
- **Mod** (`ModManifest`): display name, `mod_path` (folder under STAGING_DIR), `active`, optional `files` rules.
- **Deploy order** is array order. CLEO and modloader must come before mods that deploy into `modloader/`.
- **Default deployment**: a mod with no `files` is linked to `GAME_DIR/modloader/<name>`.
- **File rules** use rsync-style trailing slashes (see README). The UI hides this convention behind plain choices.
- **Receipt** (`GAME_DIR/.samml_receipt.json`): records every symlink and created directory so `clean` can undo exactly what was done. Stock files that a mod overwrites are moved to `GAME_DIR/.samml_backups/` and restored on teardown.
- Mod folders in staging are shared between profiles.

## Decisions

- **Name = in-game folder**: a mod's display `name` is exactly what modloader lists in game, so it is the `modloader/<name>` folder. Names must be valid folder names; invalid names are rejected with the reason (never silently cleaned), and are unique within a profile, ignoring case. There is no separate id; mods are looked up by name.
- **Renaming a mod renames its folder** on the next deploy. Resetting that mod's modloader settings is acceptable. If the profile is currently deployed, the old folder remains until redeploy, so the UI should tell the player to redeploy (the deploy plan in step 5 should detect this).
- **Staging folder** (`mod_path`): mods added through the app are stored at `mods/<slug>`, chosen once when the mod is added and never changed (other profiles may share it). Players never see it, so it uses a slug.
- **Removing a mod** removes it from the profile only, by default. The files stay in staging.
- **Deleting a mod's files** is a separate, opt-in choice (a checkbox in the UI). The UI shows which other profiles use the mod. If the user deletes the files while other profiles use them, the mod is also removed from those profiles.
- **Paths**: all user-entered paths are validated: relative only, no `..`, no escaping STAGING_DIR or GAME_DIR. Deploy re-validates, so hand-edited profiles get the same checks.
- **Rules**: an empty `files` list means the same as no list (deploy to `modloader`). A target of `./` means "inside the game folder".
- **Importing** copies from native paths on the Rust side (Tauri dialog and drag-and-drop give paths), so file bytes never pass through the webview.
- **Archives**: not in scope for now. Zip import comes after the core UI; `.7z` and `.rar` later, if at all.

## Requirements

**Setup**
- R1. First run: pick or confirm the game directory; validate it; default the staging and profiles directories; store config in the platform config dir.

**Profiles**
- R2. List, create (empty), rename, duplicate and delete profiles.

**Mods in a profile**
- R3. Show each mod's name, active state and where its files are deployed.
- R4. Add a mod by choosing files or folders (dialog or drag-and-drop); copy them into staging.
- R5. Choose where files go, or leave blank to deploy to `modloader`.
- R6. Edit a mod's name and paths, and add more files to its folder.
- R7. Remove a mod (see Decisions), toggle active, and reorder.

**Deploy**
- R8. Deploy, clean and launch from the UI. A failed deploy must never leave the game directory in a state `clean` cannot undo.
- R9. Show whether the game directory currently matches the selected profile.

## Known issues / risks

- Two mods targeting the same path: the last one silently wins. The deploy plan should report conflicts.
- Windows-origin mods can assume case-insensitive paths; Linux is case-sensitive.

## Plan

### Phase A - core (tests, no UI)
1. ~~Deploy rollback on failure; receipt records the deployed profile slug.~~ Done.
2. ~~Profile persistence: atomic `save_profile`; rename, duplicate, delete.~~ Done (`profiles.rs`).
3. ~~Mod operations on a `Profile` (add, remove, rename, move, set active, set files) with name and path validation.~~ Done (`mods.rs`, `paths.rs`, `slug.rs`).
4. Mod storage: import files into a mod folder, list a mod's files, find which profiles use a mod, delete mod files (with removal from other profiles).
5. `plan_deploy`: each mod's resolved link paths, plus conflicts; deployed-state check against the receipt.
6. Config: load, save, validate, platform config dir.
7. `Manager` facade struct over the above so each Tauri command is a one-liner.

### Phase B - scaffold
`src-tauri/` and `ui/` (Leptos + Trunk) as workspace members; CI gains `webkit2gtk` and the `wasm32-unknown-unknown` target. First screen: the profile list.

### Phase C - UI (one screen per PR)
1. Setup, then profile list with create / rename / delete.
2. Profile detail: mod list, active toggles, reorder, "deploys to" column.
3. Add / edit mod dialog: pickers, drag-and-drop, destination chooser ("Modloader (default)", "Game folder", "Custom...").
4. Deploy bar: deploy, clean, launch, deployed-state indicator, conflict warnings.

### Phase D
Zip import, progress events for large copies, polish.
