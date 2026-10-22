# SAMML - San Andreas Mod Manager Linux

A Linux-only mod manager for GTA: San Andreas (v1.0, Steam). Rust throughout: `samml-core` (logic), `samml-cli`, and a planned Tauri + Leptos GUI. Mods are symlinked from a staging directory into the game directory; nothing is copied at deploy time.

Linux is the only supported platform. Unix-only APIs (e.g. symlinks) are fine.

## Product goal

The UI manages everything. Players never open mod folders, `app_manifest.json` or profile files themselves.

## Workspace layout

```
crates/samml-core/   all logic; no printing, no argument parsing
  lib.rs             module list only
  manifest.rs        serde data types only (no behaviour)
  -- data and files
  config.rs          load the app manifest
  library.rs         the mods in staging: list, add, rename, set rules, delete; the library file
  profiles.rs        profile files: list, create, save, rename, duplicate, delete
  receipt.rs         read/write/remove the deploy receipt
  -- operations
  profile_mods.rs    edit which mods a profile uses, in memory (add/remove/move/toggle)
  usage.rs           library + profiles together: who uses a mod; rename/delete everywhere
  resolve.rs         a profile's active mods joined with their rules, ready to deploy
  deploy.rs          teardown / link / deploy (all-or-nothing) / clean
  launch.rs          launching the game through Steam
  -- helpers
  rules.rs           validate deploy rules
  paths.rs           validate/join user-supplied relative paths
  slug.rs            display name -> filename-safe handle, uniqueness
crates/samml-cli/    thin wrapper: parses args, calls core, prints
src-tauri/           (planned) Tauri commands, one-line calls into core
ui/                  (planned) Leptos frontend compiled to WASM with Trunk
tests/               fixtures shared by tests (app_manifest.json, profiles/, library/)
```

Module dependencies run one way: helpers <- data/files <- operations. `library` and `profile_mods` do not know about each other; `usage` and `resolve` are where they meet.

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
- **Library**: STAGING_DIR is the mod library. Every visible folder directly inside it (flat, no nesting) is a mod, and the folder name is the mod's name. Dropping a folder in makes it available to every profile.
- **Library file** (`STAGING_DIR/.samml_library.json`): the single source of truth for deploy rules, `{ "mods": { "<name>": { "files": [...] } } }`. Only mods needing rules have an entry. Keeping it inside STAGING_DIR means zipping that folder backs up the whole library (a planned export/import feature, which will also bundle PROFILES_DIR).
- **Default deployment**: a mod with no rules is linked whole to `GAME_DIR/modloader/<name>`.
- **File rules** use rsync-style trailing slashes (see README). The UI hides this convention behind plain choices.
- **Profile**: `PROFILES_DIR/<slug>.json`; an ordered list of `{ name, active }` entries that refer to library mods by name. The slug is the stable handle and does not change when the profile is renamed. Profiles hold no paths or rules, so any number can share a mod.
- **Deploy order** is the profile's list order. CLEO and modloader must come before mods that deploy into `modloader/`.
- **Receipt** (`GAME_DIR/.samml_receipt.json`): records every symlink and created directory so `clean` can undo exactly what was done. Stock files that a mod overwrites are moved to `GAME_DIR/.samml_backups/` and restored on teardown.

## Decisions

- **Mods are independent of profiles.** Profiles only choose from the library. Editing a mod (rename, rules, delete) is done once, in the library.
- **Name = folder = in-game name.** A mod's name is its staging folder and its `modloader/<name>` folder. Names must be valid folder names; invalid names are rejected with the reason (never silently cleaned) and are unique ignoring case. There is no separate id or slug for mods.
- **Profiles refer to mods by name**, so a rename rewrites every profile that lists the mod (`usage::rename_mod_everywhere`). If a profile can't be written, the error says which, and the stale entry shows as a missing mod.
- **Renaming a mod renames its folder** (staging and, on the next deploy, game). Resetting that mod's modloader settings is acceptable. If a profile is deployed when a mod is renamed, the old folder remains until redeploy, so the UI should tell the player to redeploy (the deploy plan in step 5 should detect this).
- **Removing a mod from a profile** leaves the files in the library.
- **Deleting a mod from the library** is separate and confirmed. The UI lists the profiles using it (`usage::profiles_using_mod`); deleting removes it from all of them (`usage::delete_mod_everywhere`).
- **Missing mods**: a profile may name a mod that is no longer in staging. It is shown as missing, and deploying fails naming all missing active mods.
- **Paths**: all user-entered paths are validated: relative only, no `..`, no escaping STAGING_DIR or GAME_DIR. Deploy re-validates, so hand-edited files get the same checks.
- **Rules**: an empty `files` list means the same as no list (deploy to `modloader`). A target of `./` means "inside the game folder".
- **Importing** copies from native paths on the Rust side (Tauri dialog and drag-and-drop give paths), so file bytes never pass through the webview.
- **Archives**: not in scope for now. Zip import comes after the core UI; `.7z` and `.rar` later, if at all.

## Requirements

**Setup**
- R1. First run: pick or confirm the game directory; validate it; default the staging and profiles directories; store config in the platform config dir.

**Profiles**
- R2. List, create (empty), rename, duplicate and delete profiles.

**Library**
- R3. Show every mod in the library with its name, rules and any problem (e.g. an unusable folder name), and which profiles use it.
- R4. Add a mod by choosing files or folders (dialog or drag-and-drop); copy them into a new folder in staging.
- R5. Choose where each mod's files go, or leave blank to deploy to `modloader`.
- R6. Rename a mod (updating every profile), edit its rules, and add more files to its folder.
- R7. Delete a mod from the library (after confirmation, removing it from every profile).

**Mods in a profile**
- R8. Show the profile's mods in order with active state, where each is deployed, and whether it is missing.
- R9. Add mods from the library, remove them (files stay), toggle active, and reorder.

**Deploy**
- R10. Deploy, clean and launch from the UI. A failed deploy must never leave the game directory in a state `clean` cannot undo.
- R11. Show whether the game directory currently matches the selected profile.

## Known issues / risks

- Two mods targeting the same path: the last one silently wins. The deploy plan should report conflicts.
- Windows-origin mods can assume case-insensitive paths; Linux is case-sensitive.

## Plan

### Phase A - core (tests, no UI)
1. ~~Deploy rollback on failure; receipt records the deployed profile slug.~~ Done.
2. ~~Profile persistence: atomic `save_profile`; rename, duplicate, delete.~~ Done (`profiles.rs`).
3. ~~Name and path validation; editing a profile's mods.~~ Done (`paths.rs`, `rules.rs`, `profile_mods.rs`).
4. ~~Mod library: mods independent of profiles, library file, rename/delete everywhere.~~ Done (`library.rs`, `usage.rs`, `resolve.rs`).
5. Importing files into a mod folder (copy from native paths) and listing a mod's files for the rule picker.
6. `plan_deploy`: each mod's resolved link paths, plus conflicts; deployed-state check against the receipt (including a rename since the last deploy).
7. Config: load, save, validate, platform config dir.
8. `Manager` facade struct over the above so each Tauri command is a one-liner.

### Phase B - scaffold
`src-tauri/` and `ui/` (Leptos + Trunk) as workspace members; CI gains `webkit2gtk` and the `wasm32-unknown-unknown` target. First screen: the profile list.

### Phase C - UI (one screen per PR)
1. Setup, then profile list with create / rename / delete.
2. Library screen: all mods, rename, delete (with the profiles using it), rules.
3. Profile detail: pick mods from the library, active toggles, reorder, "deploys to" column, missing-mod warnings.
4. Add / edit mod dialog: pickers, drag-and-drop, destination chooser ("Modloader (default)", "Game folder", "Custom...").
5. Deploy bar: deploy, clean, launch, deployed-state indicator, conflict warnings.

### Phase D
Export/import of the whole setup as one zip (staging + profiles), zip mod import, progress events for large copies, polish.
