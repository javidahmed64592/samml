# SAMML - San Andreas Mod Manager Linux

A Linux-only mod manager for GTA: San Andreas (v1.0, Steam). Rust throughout: `samml-core` (logic), `samml-cli`, and a planned Tauri + Leptos GUI.
Mods are symlinked from a staging directory into the game directory; nothing is copied at deploy time.

Linux is the only supported platform.
Unix-only APIs (e.g. symlinks) are fine.

## Product goal

The UI manages everything.
Players never open mod folders, the config file or profile files themselves.

## Workspace layout

```
crates/samml-core/   all logic; no printing, no argument parsing
  lib.rs             module list only
  manager.rs         the facade: every player action as one method; what the CLI and the UI call
  manifest.rs        serde data types only (no behaviour)
  -- data and files
  config.rs          the app config: load, check, save, first-run status and suggestions
  platform.rs        XDG locations (config file, default SAMML folder) and finding the game in Steam libraries
  library.rs         the mods in staging and the library file: ids, list, add, rename, set rules, delete
  profiles.rs        profile files: list, create, save, rename, duplicate, delete
  receipt.rs         read/write/remove the deploy receipt
  -- operations
  import.rs          copy files/folders from anywhere into a mod's folder (preview, conflicts, progress, rollback)
  browse.rs          list what is inside a mod's folder, one level at a time (rule picker)
  profile_mods.rs    edit which mods a profile uses, in memory (add/remove/move/toggle)
  usage.rs           library + profiles together: who uses a mod; delete everywhere
  resolve.rs         a profile's entries joined with the library: describe (names, problems) and resolve for deploy
  links.rs           which symlinks a mod needs (rule resolution); shared by plan and deploy
  plan.rs            what deploying a profile would do: links, conflicts, problems, deployed state (read-only)
  deploy.rs          teardown / link / deploy (all-or-nothing) / clean
  launch.rs          launching the game through Steam
  -- helpers
  rules.rs           validate deploy rules
  paths.rs           validate/join user-supplied relative paths
  slug.rs            display name -> filename-safe handle, uniqueness
crates/samml-cli/    thin wrapper: parses args, calls `Manager`, prints (`init` uses `config`/`platform` directly, since no config exists yet)
src-tauri/           (planned) Tauri commands, one-line calls into `Manager`
ui/                  (planned) Leptos frontend compiled to WASM with Trunk
tests/               fixtures shared by tests (config.json, profiles/, library/)
```

Module dependencies run one way: helpers <- data/files <- operations <- `manager`. `library` and `profile_mods` do not know about each other; `usage` and `resolve` are where they meet.

### Conventions

- Slugs and any other UI-supplied names are validated before they touch a path.
- Core never prints and never reads `std::env::args`; it returns data or `anyhow::Result`.
- One concern per module. A module's top doc comment says what it owns and which modules it depends on. New logic goes in the module for that concern; do not grow `lib.rs`.
- `manifest.rs` holds data types only. Types that cross the Tauri boundary derive `Serialize` and `Deserialize`.
- Core functions take explicit paths or `&AppConfig`; no hidden global state.
- Every behaviour change has a test. Filesystem tests use `tempfile`; fixtures live in `tests/`.
- Checks before every PR: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`.
- The config lives at `~/.config/samml/config.json` (see Decisions); `config.example.json` is the committed template. Nothing user-specific is kept in the repo, so there is nothing to gitignore.

## Concepts

- **GAME_DIR / SAMML_DIR**: set in the config (`AppConfig`).
- **SAMML_DIR** holds everything the player owns: `staging/` (the mod library) and `profiles/`. `AppConfig::staging_dir()` and `profiles_dir()` derive them, so there is one setting to configure and one folder to zip for export/import. **STAGING_DIR** and **PROFILES_DIR** in these docs mean those two subfolders.
- **Library**: STAGING_DIR is the mod library. Every visible folder directly inside it (flat, no nesting) is a mod, and the folder name is the mod's name. Dropping a folder in makes it available to every profile; it gets an id the next time the library is listed (`library::list_mods` / `sync_library`).
- **Library file** (`STAGING_DIR/.samml_library.json`): the single source of truth for mods: `{ "mods": { "<id>": { "name", "files"? } } }`. Keeping it inside STAGING_DIR means zipping that folder backs up the whole library (a planned export/import feature, which will also bundle PROFILES_DIR).
- **Mod id**: a slug of the name plus four random hex digits (`widescreen_fix-9906`), assigned once and never changed. It is what profiles refer to.
- **Default deployment**: a mod with no rules is linked whole to `GAME_DIR/modloader/<name>`.
- **File rules** use rsync-style trailing slashes (see README). The UI hides this convention behind plain choices.
- **Profile**: `PROFILES_DIR/<slug>.json`; an ordered list of `{ id, active }` entries that refer to library mods by id. The profile slug is the stable handle and does not change when the profile is renamed. Profiles hold no names, paths or rules, so any number can share a mod.
- **Deploy order** is the profile's list order. CLEO and modloader must come before mods that deploy into `modloader/`.
- **Receipt** (`GAME_DIR/.samml_receipt.json`): records every symlink and created directory so `clean` can undo exactly what was done. Stock files that a mod overwrites are moved to `GAME_DIR/.samml_backups/` and restored on teardown.

## Decisions

- **Mods are independent of profiles.** Profiles only choose from the library. Editing a mod (rename, rules, delete) is done once, in the library.
- **Profiles refer to mods by id**, so renaming a mod changes only the library entry and its folder: one folder rename plus one atomic file write (rolled back if the write fails). No profile is rewritten and no profile can be left half-updated.
- **Name = folder = in-game name.** A mod's name is its staging folder and its `modloader/<name>` folder. Names must be valid folder names; invalid names are rejected with the reason (never silently cleaned) and are unique ignoring case.
- **Renaming a mod renames its folder** (staging now, game on the next deploy). Resetting that mod's modloader settings is acceptable. If a profile is deployed when a mod is renamed, the old folder remains until redeploy, so the UI should tell the player to redeploy (the deploy plan detects this as `OutOfDate`).
- **Folders changed outside the app**: a folder renamed by hand becomes a new mod with a new id; the old entry shows as a problem (folder missing) and profiles using it show the mod as missing. A later "relink" action could reconnect them.
- **Removing a mod from a profile** leaves the files in the library.
- **Deleting a mod from the library** is separate and confirmed. The UI lists the profiles using it (`usage::profiles_using_mod`); deleting removes it from all of them (`usage::delete_mod_everywhere`).
- **Problems are reported, not hidden**: a profile entry whose id is not in the library, or whose folder is missing or unusable, is shown with the reason (`resolve::describe_profile`), and deploying fails naming every such active mod.
- **Paths**: all user-entered paths are validated: relative only, no `..`, no escaping STAGING_DIR or GAME_DIR. Deploy re-validates, so hand-edited files get the same checks.
- **Rules**: an empty `files` list means the same as no list (deploy to `modloader`). A target of `./` means "inside the game folder".
- **Importing** copies from native paths on the Rust side (Tauri dialog and drag-and-drop give paths), so file bytes never pass through the webview. `import_into_mod` takes a `Placement`: `Items` puts each chosen file/folder inside the mod under its own name; `Contents` merges one chosen folder's contents into the mod root (so a downloaded mod folder becomes the mod). `preview_import` reports file count, size and which existing files would be replaced, without changing anything; with `ConflictPolicy::Fail` any conflict aborts the import, `Overwrite` replaces them. Symlinks in the source are refused, and a source can't be, contain, or sit inside the mod's own folder. If a copy fails, everything the import created is removed (replaced files stay replaced). A progress callback runs after each file.
- **Browsing** a mod (`browse::list_mod_files`) lists one folder level at a time, folders first, and refuses `..`, absolute paths and symlinks leading outside the mod.
- **Deploy plan** (`plan::plan_deploy`) is read-only and is what the UI shows before deploying.
It lists every profile entry's links (paths relative to the game and staging directories), including inactive mods so the "deploys to" column can show them.
Inactive mods take no part in conflicts, ordering checks or the deployed state.
Links come from `links::mod_links`, the same function `deploy` uses, so what is shown is what gets deployed.
A mod that cannot be planned (missing folder, bad rule, missing source, `modloader/` not created earlier) gets a `problem` and no links, and `can_deploy` is false while any active mod has one.
Conflicts (two active links to the same game path, last mod wins) are warnings and do not block.
Placing files inside another mod's whole-folder link (e.g. `modloader/Ragdoll/...` inside the `modloader` link) is intended and is not a conflict.
`replaces_game_file` marks links that will back up and replace a file that is not ours.
- **Deployed state** compares the receipt and the links on disk with the plan: `NotDeployed`, `OtherProfile`, `UpToDate`, or `OutOfDate` with the `missing` (to add), `stale` (to remove) and `changed` (wrong target) paths.
Renaming a mod, adding or removing mods, changing rules or toggling a mod all show up as `OutOfDate`, which is how the UI knows to prompt for a redeploy.
- **Config location**: `$XDG_CONFIG_HOME/samml/config.json` (default `~/.config/samml/config.json`).
The default SAMML folder is `$XDG_DATA_HOME/samml` (default `~/.local/share/samml`), holding `staging/` and `profiles/`.
XDG variables are used only when absolute.
The CLI uses the standard location, or an explicit path given as its first argument; there is no other lookup.
- **Game detection** (`platform::find_game_dirs`) checks the native, `~/.steam` and Flatpak Steam roots plus every library listed in `libraryfolders.vdf` for `steamapps/common/Grand Theft Auto San Andreas` containing `gta-sa.exe` (any case).
Installs reached two ways (symlinks) are reported once.
- **Config validation** (`config::check_config`) returns a list of problems per field, in words the player can act on.
The game folder must exist and contain `gta-sa.exe`; both paths must be absolute; the SAMML folder is created on demand (with `staging/` and `profiles/`), but if it exists it must be a folder; it must not be inside the game folder or contain it.
- **Saving config** (`config::save_config`) refuses a config with problems and writes atomically, so a saved config is always one the app can start with.
`config_status` tells the UI whether to show setup: `Missing` (first run), `Broken` (unreadable), `NeedsAttention` (parses but paths need fixing, e.g. the game moved; the existing values are kept for the form) or `Ready`.
`suggest_config` gives the setup screen its starting values.
- **Manager** (`manager::Manager`) is the single entry point for player actions, so a Tauri command is a one-line call and the CLI only parses arguments and prints.
Setup happens before a manager exists (`config::config_status`, `suggest_config`, `save_config`); then `Manager::open(config_path)` refuses an unusable config and creates the SAMML folder.
Methods are grouped as profiles, library, and deploying; they compose the modules and add no rules of their own.
Compound actions live here: `create_mod` adds the mod and imports its files, and removes the mod again if the import fails; profile edits load, edit and save a profile; `add_mod_to_profile` only accepts mods that are in the library.
- **Locking**: everything that changes the library, a profile or the game directory runs under one lock in `Manager`, so two UI actions at once (e.g. adding two mods to the same profile) cannot overwrite each other.
Reads are not locked; `import_into_mod` is not locked either, so a long copy never freezes the app.
`Manager` is `Send + Sync`, so Tauri can hold it as shared state.
- **Deleting or renaming never touches the game directory.**
Deleting the deployed profile leaves it deployed (`Manager::deployed` reports its slug with no name) until `clean` or the next deploy.
Deleting or renaming a deployed mod leaves dangling or old links that the plan reports as `OutOfDate`; redeploying (which tears down first) clears them.
- **Archives**: not in scope for now. Zip import comes after the core UI; `.7z` and `.rar` later, if at all.

## Requirements

**Setup**
- R1. First run: pick or confirm the game directory; validate it; default the SAMML directory; store config in the platform config dir.

**Profiles**
- R2. List, create (empty), rename, duplicate and delete profiles.

**Library**
- R3. Show every mod in the library with its name, rules and any problem (e.g. an unusable folder name), and which profiles use it.
- R4. Add a mod by choosing files or folders (dialog or drag-and-drop); copy them into a new folder in staging.
- R5. Choose where each mod's files go, or leave blank to deploy to `modloader`.
- R6. Rename a mod, edit its rules, and add more files to its folder.
- R7. Delete a mod from the library (after confirmation, removing it from every profile).

**Mods in a profile**
- R8. Show the profile's mods in order with active state, where each is deployed, and whether it has a problem (e.g. missing).
- R9. Add mods from the library, remove them (files stay), toggle active, and reorder.

**Deploy**
- R10. Deploy, clean and launch from the UI. A failed deploy must never leave the game directory in a state `clean` cannot undo.
- R11. Show whether the game directory currently matches the selected profile.

## Known issues / risks

- Windows-origin mods can assume case-insensitive paths; Linux is case-sensitive.

## Plan

### Phase A - core (tests, no UI)
1. ~~Deploy rollback on failure; receipt records the deployed profile slug.~~ Done.
2. ~~Profile persistence: atomic `save_profile`; rename, duplicate, delete.~~ Done (`profiles.rs`).
3. ~~Name and path validation; editing a profile's mods.~~ Done (`paths.rs`, `rules.rs`, `profile_mods.rs`).
4. ~~Mod library: mods independent of profiles, stable ids, library file, delete everywhere.~~ Done (`library.rs`, `usage.rs`, `resolve.rs`).
5. ~~Importing files into a mod folder (copy from native paths) and listing a mod's files for the rule picker.~~ Done (`import.rs`, `browse.rs`).
6. ~~`plan_deploy`: each mod's resolved link paths, plus conflicts; deployed-state check against the receipt (including a rename since the last deploy).~~ Done (`plan.rs`, `links.rs`).
7. ~~Config: load, save, validate, platform config dir.~~ Done (`config.rs`, `platform.rs`).
8. ~~`Manager` facade struct over the above so each Tauri command is a one-liner.~~ Done (`manager.rs`; the CLI now uses it).

### Phase B - scaffold
`src-tauri/` and `ui/` (Leptos + Trunk) as workspace members; CI gains `webkit2gtk` and the `wasm32-unknown-unknown` target. First screen: the profile list.

### Phase C - UI (one screen per PR)
1. Setup, then profile list with create / rename / delete.
2. Library screen: all mods, rename, delete (with the profiles using it), rules.
3. Profile detail: pick mods from the library, active toggles, reorder, "deploys to" column, missing-mod warnings.
4. Add / edit mod dialog: pickers, drag-and-drop, destination chooser ("Modloader (default)", "Game folder", "Custom...").
5. Deploy bar: deploy, clean, launch, deployed-state indicator, conflict warnings.

### Phase D
Export/import of the whole setup as one zip (the SAMML folder), zip mod import, wiring `import_into_mod`'s progress callback to UI events, polish.
