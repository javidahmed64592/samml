//! The mod library: the mods in `STAGING_DIR`.
//!
//! Every folder directly inside STAGING_DIR is a mod, and the folder name is
//! the mod's name (also its `modloader/<name>` folder, so it is what shows
//! in game). Dropping a folder into staging makes it available.
//!
//! Each mod has a stable id, which is what profiles refer to, so renaming a
//! mod only ever changes this library. Ids, names and deploy rules live in
//! one file, `STAGING_DIR/.samml_library.json`, so zipping the staging
//! directory backs up the mods and their rules together. Folders dropped in
//! by hand get an id the next time the library is listed (`list_mods`).
//!
//! This module only knows about staging. Which profiles use a mod is
//! `usage`'s concern; adding a mod to a profile is `profile_mods`.

use crate::manifest::{FileRule, Library, LibraryMod, ModSettings};
use crate::{paths, rules, slug};
use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const LIBRARY_FILE: &str = ".samml_library.json";

// --- The library file ----------------------------------------------------

fn library_path(staging_dir: &Path) -> PathBuf {
    staging_dir.join(LIBRARY_FILE)
}

/// A missing file is an empty library.
pub fn load_library(staging_dir: &Path) -> Result<Library> {
    let path = library_path(staging_dir);
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("parsing library file {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Library::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Atomic (temp file, then rename), so a crash never leaves a half-written
/// library.
fn save_library(staging_dir: &Path, library: &Library) -> Result<()> {
    fs::create_dir_all(staging_dir)
        .with_context(|| format!("creating {}", staging_dir.display()))?;
    let path = library_path(staging_dir);
    let tmp = staging_dir.join(format!("{LIBRARY_FILE}.tmp"));
    fs::write(&tmp, serde_json::to_string_pretty(library)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("saving {}", path.display()))
}

// --- Ids and names -------------------------------------------------------

/// A new id for a mod called `name`: a slug of the name plus a short random
/// suffix (`widescreen_fix-3fa9`), not used by any mod in the library. It
/// never changes, even if the mod is renamed.
fn new_id(library: &Library, name: &str) -> String {
    let base = slug::slugify_or(name, "mod");
    loop {
        let candidate = format!("{base}-{}", slug::random_suffix());
        if !library.mods.contains_key(&candidate) {
            return candidate;
        }
    }
}

/// Names of the visible folders in staging (hidden ones, with a leading
/// `.`, are never mods).
fn folder_names(staging_dir: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    if !staging_dir.is_dir() {
        return Ok(names);
    }
    for entry in fs::read_dir(staging_dir)
        .with_context(|| format!("reading staging directory {}", staging_dir.display()))?
    {
        let entry = entry?;
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') {
            names.push(name);
        }
    }
    Ok(names)
}

/// The mod's folder in staging. Errors if `name` could point anywhere other
/// than a direct child of `staging_dir`.
pub fn mod_dir(staging_dir: &Path, name: &str) -> Result<PathBuf> {
    paths::validate_component(name)
        .map_err(|reason| anyhow!("'{name}' is not a valid mod name: {reason}"))?;
    Ok(staging_dir.join(name))
}

/// The library entry for `id`.
pub fn entry<'a>(library: &'a Library, id: &str) -> Result<&'a ModSettings> {
    library
        .mods
        .get(id)
        .with_context(|| format!("there is no mod with id '{id}' in the library"))
}

/// `name` must be a valid folder name and must not clash (ignoring case)
/// with another mod's folder or name. `except` is the mod being renamed,
/// which may keep or re-case its own name.
fn validate_new_name(
    staging_dir: &Path,
    library: &Library,
    name: &str,
    except: Option<&str>,
) -> Result<()> {
    paths::validate_component(name)
        .map_err(|reason| anyhow!("'{name}' is not a valid mod name: {reason}"))?;
    let own = except.and_then(|id| library.mods.get(id)).map(|m| &m.name);
    let taken: HashSet<String> = folder_names(staging_dir)?
        .into_iter()
        .chain(library.mods.values().map(|m| m.name.clone()))
        .filter(|existing| Some(existing) != own)
        .map(|existing| existing.to_lowercase())
        .collect();
    if taken.contains(&name.to_lowercase()) {
        bail!("a mod named '{name}' already exists");
    }
    Ok(())
}

// --- Reading -------------------------------------------------------------

/// Gives an id to every visible folder that has none, saving the library
/// only if something was added. Returns the up-to-date library.
pub fn sync_library(staging_dir: &Path) -> Result<Library> {
    let mut library = load_library(staging_dir)?;
    let known: HashSet<String> = library.mods.values().map(|m| m.name.clone()).collect();
    let mut changed = false;
    for folder in folder_names(staging_dir)? {
        if !known.contains(&folder) {
            let id = new_id(&library, &folder);
            library.mods.insert(
                id,
                ModSettings {
                    name: folder,
                    files: None,
                },
            );
            changed = true;
        }
    }
    if changed {
        save_library(staging_dir, &library)?;
    }
    Ok(library)
}

/// Every mod in the library, sorted by name, with its deploy rules and any
/// problem that stops it being used. Registers new folders first (see
/// `sync_library`), so this is the call the UI makes when it opens.
pub fn list_mods(staging_dir: &Path) -> Result<Vec<LibraryMod>> {
    let library = sync_library(staging_dir)?;
    let folders: HashSet<String> = folder_names(staging_dir)?.into_iter().collect();
    let mut mods: Vec<LibraryMod> = library
        .mods
        .into_iter()
        .map(|(id, settings)| {
            let problem = if !folders.contains(&settings.name) {
                Some(format!(
                    "its folder '{}' is missing from {}",
                    settings.name,
                    staging_dir.display()
                ))
            } else {
                paths::validate_component(&settings.name)
                    .err()
                    .map(|reason| {
                        format!("'{}' cannot be used as a mod name: {reason}", settings.name)
                    })
            };
            LibraryMod {
                id,
                name: settings.name,
                files: settings.files,
                problem,
            }
        })
        .collect();
    mods.sort_by_cached_key(|m| (m.name.to_lowercase(), m.id.clone()));
    Ok(mods)
}

// --- Editing -------------------------------------------------------------

/// Creates an empty mod folder named `name`, ready for files to be copied
/// in, and returns the new mod's id and folder. `files` are its deploy rules
/// (`None` or empty deploys it to `modloader/<name>`).
pub fn add_mod(
    staging_dir: &Path,
    name: &str,
    files: Option<Vec<FileRule>>,
) -> Result<(String, PathBuf)> {
    let mut library = load_library(staging_dir)?;
    validate_new_name(staging_dir, &library, name, None)?;
    let files = rules::normalize_rules(files)?;
    let dir = mod_dir(staging_dir, name)?;
    fs::create_dir_all(staging_dir)
        .with_context(|| format!("creating {}", staging_dir.display()))?;
    fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let id = new_id(&library, name);
    library.mods.insert(
        id.clone(),
        ModSettings {
            name: name.to_string(),
            files,
        },
    );
    if let Err(err) = save_library(staging_dir, &library) {
        let _ = fs::remove_dir(&dir);
        return Err(err);
    }
    Ok((id, dir))
}

/// Renames the mod: its folder in staging and its name in the library. Its
/// id does not change, so no profile needs updating; the game folder
/// follows on the next deploy.
pub fn rename_mod(staging_dir: &Path, id: &str, new_name: &str) -> Result<()> {
    let mut library = load_library(staging_dir)?;
    let old_name = entry(&library, id)?.name.clone();
    let old_dir = mod_dir(staging_dir, &old_name)?;
    if !old_dir.is_dir() {
        bail!(
            "the folder '{old_name}' is missing from {}",
            staging_dir.display()
        );
    }
    validate_new_name(staging_dir, &library, new_name, Some(id))?;
    let new_dir = mod_dir(staging_dir, new_name)?;

    fs::rename(&old_dir, &new_dir)
        .with_context(|| format!("renaming {} to {}", old_dir.display(), new_dir.display()))?;

    if let Some(settings) = library.mods.get_mut(id) {
        settings.name = new_name.to_string();
    }
    if let Err(err) = save_library(staging_dir, &library) {
        let _ = fs::rename(&new_dir, &old_dir);
        return Err(err);
    }
    Ok(())
}

/// Sets where the mod's files are deployed. `None` (or an empty list)
/// deploys the whole mod folder to `modloader/<name>`.
pub fn set_mod_files(staging_dir: &Path, id: &str, files: Option<Vec<FileRule>>) -> Result<()> {
    let mut library = load_library(staging_dir)?;
    entry(&library, id)?;
    let files = rules::normalize_rules(files)?;
    if let Some(settings) = library.mods.get_mut(id) {
        settings.files = files;
    }
    save_library(staging_dir, &library)
}

/// Deletes the mod's folder (if it still exists) and its library entry.
/// Profiles that use the mod are not touched here; see
/// `usage::delete_mod_everywhere`.
pub fn delete_mod(staging_dir: &Path, id: &str) -> Result<()> {
    let mut library = load_library(staging_dir)?;
    let name = entry(&library, id)?.name.clone();
    let dir = mod_dir(staging_dir, &name)?;
    if dir.is_dir() {
        fs::remove_dir_all(&dir).with_context(|| format!("deleting {}", dir.display()))?;
    }
    library.mods.remove(id);
    save_library(staging_dir, &library)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn rule(source: &str, target: &str) -> FileRule {
        FileRule {
            source: source.into(),
            target: target.into(),
        }
    }

    fn names(staging: &Path) -> Vec<String> {
        list_mods(staging)
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect()
    }

    fn repo_fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/library")
    }

    // --- ids ---

    #[test]
    fn ids_are_a_slug_of_the_name_plus_a_suffix() {
        let library = Library::default();
        let id = new_id(&library, "Widescreen Fix");
        let (slug_part, suffix) = id.rsplit_once('-').unwrap();
        assert_eq!(slug_part, "widescreen_fix");
        assert_eq!(suffix.len(), 4);
        assert!(new_id(&library, "!!!").starts_with("mod_"));
    }

    #[test]
    fn ids_never_collide_with_existing_ones() {
        let mut library = Library::default();
        for _ in 0..200 {
            let id = new_id(&library, "Same Name");
            assert!(!library.mods.contains_key(&id));
            library.mods.insert(
                id,
                ModSettings {
                    name: "x".into(),
                    files: None,
                },
            );
        }
    }

    // --- reading ---

    #[test]
    fn loads_the_fixture_library() {
        let library = load_library(&repo_fixture()).unwrap();
        assert_eq!(library.mods.len(), 4);
        assert_eq!(
            library.mods["modloader-91c3"].files.as_ref().unwrap().len(),
            2
        );
    }

    #[test]
    fn a_missing_library_file_is_an_empty_library() {
        let dir = TempDir::new().unwrap();
        assert!(load_library(dir.path()).unwrap().mods.is_empty());
        assert!(
            load_library(&dir.path().join("nope"))
                .unwrap()
                .mods
                .is_empty()
        );
    }

    #[test]
    fn a_broken_library_file_is_a_clear_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(LIBRARY_FILE), "{ not json").unwrap();
        let err = load_library(dir.path()).unwrap_err();
        assert!(err.to_string().contains("parsing library file"));
    }

    #[test]
    fn listing_registers_dropped_in_folders_and_keeps_their_ids() {
        let dir = TempDir::new().unwrap();
        for n in ["zebra", "Apple", ".hidden"] {
            fs::create_dir(dir.path().join(n)).unwrap();
        }
        fs::write(dir.path().join("loose_file.txt"), "x").unwrap();

        let first = list_mods(dir.path()).unwrap();
        let second = list_mods(dir.path()).unwrap();

        let listed: Vec<_> = first.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            listed,
            ["Apple", "zebra"],
            "sorted, hidden and files skipped"
        );
        assert_eq!(first, second, "ids are stable once assigned");
        assert!(first[0].id.starts_with("apple-"));
    }

    #[test]
    fn listing_a_missing_staging_dir_is_empty_and_creates_nothing() {
        let dir = TempDir::new().unwrap();
        let staging = dir.path().join("nope");
        assert!(list_mods(&staging).unwrap().is_empty());
        assert!(!staging.exists());
    }

    #[test]
    fn listing_joins_entries_with_their_rules() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("Plain")).unwrap();
        add_mod(dir.path(), "Ruled", Some(vec![rule("a", "b")])).unwrap();

        let mods = list_mods(dir.path()).unwrap();

        assert_eq!(mods[0].name, "Plain");
        assert!(mods[0].files.is_none() && mods[0].problem.is_none());
        assert_eq!(mods[1].name, "Ruled");
        assert_eq!(mods[1].files.as_ref().unwrap()[0].target, "b");
    }

    #[test]
    fn folders_with_unusable_names_are_listed_with_the_reason() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("trailing ")).unwrap();

        let mods = list_mods(dir.path()).unwrap();

        let problem = mods[0].problem.as_ref().unwrap();
        assert!(problem.contains("start or end with a space"), "{problem}");
    }

    #[test]
    fn entries_whose_folder_is_gone_are_listed_as_problems() {
        let dir = TempDir::new().unwrap();
        let (id, path) = add_mod(dir.path(), "Gone", None).unwrap();
        fs::remove_dir(path).unwrap();

        let mods = list_mods(dir.path()).unwrap();

        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].id, id);
        assert!(mods[0].problem.as_ref().unwrap().contains("is missing"));
    }

    #[test]
    fn a_folder_renamed_outside_the_app_becomes_a_new_mod() {
        let dir = TempDir::new().unwrap();
        let (old_id, path) = add_mod(dir.path(), "Old", None).unwrap();
        fs::rename(path, dir.path().join("Renamed By Hand")).unwrap();

        let mods = list_mods(dir.path()).unwrap();

        let by_name = |n: &str| mods.iter().find(|m| m.name == n).unwrap();
        assert_eq!(by_name("Old").id, old_id);
        assert!(by_name("Old").problem.is_some(), "old entry is orphaned");
        assert!(by_name("Renamed By Hand").problem.is_none());
        assert_ne!(by_name("Renamed By Hand").id, old_id);
    }

    // --- add ---

    #[test]
    fn add_mod_creates_the_folder_and_records_rules() {
        let dir = TempDir::new().unwrap();
        let staging = dir.path().join("staging");

        let (id, path) = add_mod(&staging, "Cool Cars", Some(vec![rule("x/", "y/")])).unwrap();

        assert_eq!(path, staging.join("Cool Cars"));
        assert!(path.is_dir());
        let library = load_library(&staging).unwrap();
        assert_eq!(library.mods[&id].name, "Cool Cars");
        assert_eq!(library.mods[&id].files.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn add_mod_treats_empty_rules_as_the_default() {
        let dir = TempDir::new().unwrap();
        let (id, _) = add_mod(dir.path(), "Plain", Some(vec![])).unwrap();
        assert!(load_library(dir.path()).unwrap().mods[&id].files.is_none());
    }

    #[test]
    fn add_mod_rejects_invalid_names_with_the_reason() {
        let dir = TempDir::new().unwrap();
        let cases = [
            ("", "cannot be empty"),
            (" Mod", "start or end with a space"),
            ("a/b", "cannot contain '/'"),
            ("a\\b", "cannot contain '\\'"),
            (".hidden", "start with a '.'"),
            ("bad\tname", "control characters"),
        ];
        for (name, reason) in cases {
            let msg = add_mod(dir.path(), name, None).unwrap_err().to_string();
            assert!(msg.contains("not a valid mod name"), "{name:?}: {msg}");
            assert!(msg.contains(reason), "{name:?}: {msg}");
        }
        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn add_mod_rejects_existing_names_ignoring_case() {
        let dir = TempDir::new().unwrap();
        add_mod(dir.path(), "Mod", None).unwrap();
        fs::create_dir(dir.path().join("Unregistered")).unwrap();

        for name in ["MOD", "unregistered"] {
            let err = add_mod(dir.path(), name, None).unwrap_err();
            assert!(err.to_string().contains("already exists"), "{name}");
        }
        assert_eq!(names(dir.path()), ["Mod", "Unregistered"]);
    }

    #[test]
    fn add_mod_rejects_bad_rules_without_creating_anything() {
        let dir = TempDir::new().unwrap();
        assert!(add_mod(dir.path(), "Bad", Some(vec![rule("../x", "y")])).is_err());
        assert!(names(dir.path()).is_empty());
    }

    // --- rename ---

    #[test]
    fn rename_moves_the_folder_but_keeps_the_id_and_rules() {
        let dir = TempDir::new().unwrap();
        let (id, path) = add_mod(dir.path(), "Old", Some(vec![rule("a", "b")])).unwrap();
        fs::write(path.join("file.txt"), "keep").unwrap();

        rename_mod(dir.path(), &id, "New Name").unwrap();

        let mods = list_mods(dir.path()).unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(
            (mods[0].id.as_str(), mods[0].name.as_str()),
            (id.as_str(), "New Name")
        );
        assert!(mods[0].files.is_some());
        assert_eq!(
            fs::read_to_string(dir.path().join("New Name/file.txt")).unwrap(),
            "keep"
        );
        assert!(!dir.path().join("Old").exists());
    }

    #[test]
    fn rename_validates_the_new_name_and_changes_nothing_on_error() {
        let dir = TempDir::new().unwrap();
        let (id, _) = add_mod(dir.path(), "Old", None).unwrap();
        add_mod(dir.path(), "Other", None).unwrap();

        let err = rename_mod(dir.path(), &id, "a/b").unwrap_err();
        assert!(err.to_string().contains("cannot contain '/'"));
        assert!(rename_mod(dir.path(), &id, "other").is_err(), "name taken");
        assert!(
            rename_mod(dir.path(), "ghost-0000", "Fine").is_err(),
            "unknown id"
        );
        assert_eq!(names(dir.path()), ["Old", "Other"]);
    }

    #[test]
    fn rename_may_change_only_the_case() {
        let dir = TempDir::new().unwrap();
        let (id, _) = add_mod(dir.path(), "old", None).unwrap();
        rename_mod(dir.path(), &id, "OLD").unwrap();
        assert_eq!(names(dir.path()), ["OLD"]);
    }

    #[test]
    fn rename_needs_the_folder_to_exist() {
        let dir = TempDir::new().unwrap();
        let (id, path) = add_mod(dir.path(), "Gone", None).unwrap();
        fs::remove_dir(path).unwrap();
        assert!(rename_mod(dir.path(), &id, "New").is_err());
    }

    // --- rules / delete ---

    #[test]
    fn set_mod_files_replaces_and_clears_rules() {
        let dir = TempDir::new().unwrap();
        let (id, _) = add_mod(dir.path(), "M", None).unwrap();
        let files = |dir: &Path| list_mods(dir).unwrap()[0].files.clone();

        set_mod_files(dir.path(), &id, Some(vec![rule("a.txt", "b/")])).unwrap();
        assert_eq!(files(dir.path()).unwrap().len(), 1);

        assert!(set_mod_files(dir.path(), &id, Some(vec![rule("..", "x")])).is_err());
        assert!(files(dir.path()).is_some(), "unchanged");

        set_mod_files(dir.path(), &id, None).unwrap();
        assert!(files(dir.path()).is_none());
        assert!(set_mod_files(dir.path(), "ghost-0000", None).is_err());
    }

    #[test]
    fn delete_removes_the_folder_and_entry_only() {
        let dir = TempDir::new().unwrap();
        let (gone, path) = add_mod(dir.path(), "Gone", Some(vec![rule("a", "b")])).unwrap();
        fs::write(path.join("file.txt"), "x").unwrap();
        add_mod(dir.path(), "Stays", None).unwrap();

        delete_mod(dir.path(), &gone).unwrap();

        assert_eq!(names(dir.path()), ["Stays"]);
        assert!(!dir.path().join("Gone").exists());
        assert!(delete_mod(dir.path(), &gone).is_err());
    }

    #[test]
    fn delete_can_forget_an_entry_whose_folder_is_already_gone() {
        let dir = TempDir::new().unwrap();
        let (id, path) = add_mod(dir.path(), "Gone", None).unwrap();
        fs::remove_dir(path).unwrap();

        delete_mod(dir.path(), &id).unwrap();

        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn hand_edited_names_that_could_escape_staging_are_refused() {
        let dir = TempDir::new().unwrap();
        let staging = dir.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir(dir.path().join("victim")).unwrap();
        fs::write(
            staging.join(LIBRARY_FILE),
            r#"{ "mods": { "evil-0000": { "name": "../victim" } } }"#,
        )
        .unwrap();

        assert!(delete_mod(&staging, "evil-0000").is_err());
        assert!(rename_mod(&staging, "evil-0000", "x").is_err());
        assert!(mod_dir(&staging, "../victim").is_err());
        assert!(dir.path().join("victim").is_dir());
    }
}
