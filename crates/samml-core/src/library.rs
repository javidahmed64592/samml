//! The mod library: the mods in `STAGING_DIR`.
//!
//! Every folder directly inside STAGING_DIR is a mod, and the folder name is
//! the mod's name (also its `modloader/<name>` folder, so it is what shows
//! in game). Dropping a folder into staging makes it available. Deploy rules
//! for mods that need them live in one file, `STAGING_DIR/.samml_library.json`,
//! so zipping the staging directory backs up the mods and their rules
//! together.
//!
//! This module only knows about staging. Which profiles use a mod is
//! `usage`'s concern; adding a mod to a profile is `profile_mods`.

use crate::manifest::{FileRule, Library, LibraryMod, ModSettings};
use crate::{paths, rules};
use anyhow::{Context, Result, anyhow, bail};
use std::fs;
use std::path::{Path, PathBuf};

pub const LIBRARY_FILE: &str = ".samml_library.json";

// --- The library file ----------------------------------------------------

fn library_path(staging_dir: &Path) -> PathBuf {
    staging_dir.join(LIBRARY_FILE)
}

/// A missing file is an empty library: every mod uses the default
/// deployment.
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

// --- Reading -------------------------------------------------------------

/// Names of the visible folders in staging (hidden ones, with a leading
/// `.`, are never mods), sorted case-insensitively.
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
    names.sort_by_cached_key(|n| (n.to_lowercase(), n.clone()));
    Ok(names)
}

/// Every mod in the library, with its deploy rules and any problem that
/// stops it being used.
pub fn list_mods(staging_dir: &Path) -> Result<Vec<LibraryMod>> {
    let mut library = load_library(staging_dir)?;
    let mods = folder_names(staging_dir)?
        .into_iter()
        .map(|name| LibraryMod {
            files: library.mods.remove(&name).and_then(|s| s.files),
            problem: paths::validate_component(&name)
                .err()
                .map(|reason| format!("'{name}' cannot be used as a mod name: {reason}")),
            name,
        })
        .collect();
    Ok(mods)
}

/// The mod's folder in staging. Errors if `name` could point anywhere other
/// than a direct child of `staging_dir`.
pub fn mod_dir(staging_dir: &Path, name: &str) -> Result<PathBuf> {
    paths::validate_component(name)
        .map_err(|reason| anyhow!("'{name}' is not a valid mod name: {reason}"))?;
    Ok(staging_dir.join(name))
}

/// `name` must be a valid folder name and must not clash (ignoring case)
/// with another mod. `except` is the mod being renamed, which may keep or
/// re-case its own name.
fn validate_new_name(staging_dir: &Path, name: &str, except: Option<&str>) -> Result<()> {
    paths::validate_component(name)
        .map_err(|reason| anyhow!("'{name}' is not a valid mod name: {reason}"))?;
    let clash = folder_names(staging_dir)?
        .iter()
        .filter(|existing| Some(existing.as_str()) != except)
        .any(|existing| existing.to_lowercase() == name.to_lowercase());
    if clash {
        bail!("a mod named '{name}' already exists");
    }
    Ok(())
}

fn require_mod_dir(staging_dir: &Path, name: &str) -> Result<PathBuf> {
    let dir = mod_dir(staging_dir, name)?;
    if !dir.is_dir() {
        bail!(
            "there is no mod named '{name}' in {}",
            staging_dir.display()
        );
    }
    Ok(dir)
}

// --- Editing -------------------------------------------------------------

/// Creates an empty mod folder named `name`, ready for files to be copied
/// in, and returns its path. `files` are its deploy rules (`None` or empty
/// deploys it to `modloader/<name>`).
pub fn add_mod(staging_dir: &Path, name: &str, files: Option<Vec<FileRule>>) -> Result<PathBuf> {
    validate_new_name(staging_dir, name, None)?;
    let files = rules::normalize_rules(files)?;
    let dir = mod_dir(staging_dir, name)?;
    fs::create_dir_all(staging_dir)
        .with_context(|| format!("creating {}", staging_dir.display()))?;
    fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;

    if files.is_some() {
        let mut library = load_library(staging_dir)?;
        library.mods.insert(name.to_string(), ModSettings { files });
        if let Err(err) = save_library(staging_dir, &library) {
            let _ = fs::remove_dir(&dir);
            return Err(err);
        }
    }
    Ok(dir)
}

/// Renames the mod's folder and moves its settings with it. Profiles that
/// use the mod are not touched here; see `usage::rename_mod_everywhere`.
pub fn rename_mod(staging_dir: &Path, name: &str, new_name: &str) -> Result<()> {
    let old_dir = require_mod_dir(staging_dir, name)?;
    validate_new_name(staging_dir, new_name, Some(name))?;
    let new_dir = mod_dir(staging_dir, new_name)?;

    fs::rename(&old_dir, &new_dir)
        .with_context(|| format!("renaming {} to {}", old_dir.display(), new_dir.display()))?;

    let mut library = load_library(staging_dir)?;
    if let Some(settings) = library.mods.remove(name) {
        library.mods.insert(new_name.to_string(), settings);
        if let Err(err) = save_library(staging_dir, &library) {
            let _ = fs::rename(&new_dir, &old_dir);
            return Err(err);
        }
    }
    Ok(())
}

/// Sets where the mod's files are deployed. `None` (or an empty list)
/// deploys the whole mod folder to `modloader/<name>`.
pub fn set_mod_files(staging_dir: &Path, name: &str, files: Option<Vec<FileRule>>) -> Result<()> {
    require_mod_dir(staging_dir, name)?;
    let files = rules::normalize_rules(files)?;
    let mut library = load_library(staging_dir)?;
    match files {
        Some(_) => {
            library.mods.insert(name.to_string(), ModSettings { files });
        }
        None => {
            library.mods.remove(name);
        }
    }
    save_library(staging_dir, &library)
}

/// Deletes the mod's folder and its settings. Profiles that use the mod are
/// not touched here; see `usage::delete_mod_everywhere`.
pub fn delete_mod(staging_dir: &Path, name: &str) -> Result<()> {
    let dir = require_mod_dir(staging_dir, name)?;
    fs::remove_dir_all(&dir).with_context(|| format!("deleting {}", dir.display()))?;
    let mut library = load_library(staging_dir)?;
    if library.mods.remove(name).is_some() {
        save_library(staging_dir, &library)?;
    }
    Ok(())
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

    // --- reading ---

    #[test]
    fn loads_the_fixture_library() {
        let library = load_library(&repo_fixture()).unwrap();
        assert_eq!(library.mods.len(), 3);
        assert_eq!(library.mods["Modloader"].files.as_ref().unwrap().len(), 2);
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
    fn every_visible_folder_is_a_mod_sorted_by_name() {
        let dir = TempDir::new().unwrap();
        for n in ["zebra", "Apple", "mango", ".hidden"] {
            fs::create_dir(dir.path().join(n)).unwrap();
        }
        fs::write(dir.path().join("loose_file.txt"), "x").unwrap();

        assert_eq!(names(dir.path()), ["Apple", "mango", "zebra"]);
    }

    #[test]
    fn listing_a_missing_staging_dir_is_empty() {
        let dir = TempDir::new().unwrap();
        assert!(list_mods(&dir.path().join("nope")).unwrap().is_empty());
    }

    #[test]
    fn listing_joins_folders_with_their_rules() {
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

    // --- add ---

    #[test]
    fn add_mod_creates_the_folder_and_records_rules() {
        let dir = TempDir::new().unwrap();
        let staging = dir.path().join("staging");

        let path = add_mod(&staging, "Cool Cars", Some(vec![rule("x/", "y/")])).unwrap();

        assert_eq!(path, staging.join("Cool Cars"));
        assert!(path.is_dir());
        let library = load_library(&staging).unwrap();
        assert_eq!(library.mods["Cool Cars"].files.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn add_mod_without_rules_writes_no_library_entry() {
        let dir = TempDir::new().unwrap();
        add_mod(dir.path(), "Plain", Some(vec![])).unwrap();
        assert!(!dir.path().join(LIBRARY_FILE).exists());
        assert_eq!(names(dir.path()), ["Plain"]);
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

        let err = add_mod(dir.path(), "MOD", None).unwrap_err();

        assert!(err.to_string().contains("already exists"));
        assert_eq!(names(dir.path()), ["Mod"]);
    }

    #[test]
    fn add_mod_rejects_bad_rules_without_creating_anything() {
        let dir = TempDir::new().unwrap();
        assert!(add_mod(dir.path(), "Bad", Some(vec![rule("../x", "y")])).is_err());
        assert!(names(dir.path()).is_empty());
    }

    // --- rename ---

    #[test]
    fn rename_moves_the_folder_and_its_rules() {
        let dir = TempDir::new().unwrap();
        let path = add_mod(dir.path(), "Old", Some(vec![rule("a", "b")])).unwrap();
        fs::write(path.join("file.txt"), "keep").unwrap();

        rename_mod(dir.path(), "Old", "New Name").unwrap();

        assert_eq!(names(dir.path()), ["New Name"]);
        assert_eq!(
            fs::read_to_string(dir.path().join("New Name/file.txt")).unwrap(),
            "keep"
        );
        let library = load_library(dir.path()).unwrap();
        assert!(!library.mods.contains_key("Old"));
        assert!(library.mods.contains_key("New Name"));
    }

    #[test]
    fn rename_validates_the_new_name_and_changes_nothing_on_error() {
        let dir = TempDir::new().unwrap();
        add_mod(dir.path(), "Old", None).unwrap();
        add_mod(dir.path(), "Other", None).unwrap();

        let err = rename_mod(dir.path(), "Old", "a/b").unwrap_err();
        assert!(err.to_string().contains("cannot contain '/'"));
        assert!(
            rename_mod(dir.path(), "Old", "other").is_err(),
            "name taken"
        );
        assert!(
            rename_mod(dir.path(), "Ghost", "Fine").is_err(),
            "unknown mod"
        );
        assert_eq!(names(dir.path()), ["Old", "Other"]);
    }

    #[test]
    fn rename_may_change_only_the_case() {
        let dir = TempDir::new().unwrap();
        add_mod(dir.path(), "old", None).unwrap();
        rename_mod(dir.path(), "old", "OLD").unwrap();
        assert_eq!(names(dir.path()), ["OLD"]);
    }

    // --- rules / delete ---

    #[test]
    fn set_mod_files_replaces_and_clears_rules() {
        let dir = TempDir::new().unwrap();
        add_mod(dir.path(), "M", None).unwrap();

        set_mod_files(dir.path(), "M", Some(vec![rule("a.txt", "b/")])).unwrap();
        assert_eq!(
            list_mods(dir.path()).unwrap()[0]
                .files
                .as_ref()
                .unwrap()
                .len(),
            1
        );

        assert!(set_mod_files(dir.path(), "M", Some(vec![rule("..", "x")])).is_err());
        assert!(
            list_mods(dir.path()).unwrap()[0].files.is_some(),
            "unchanged"
        );

        set_mod_files(dir.path(), "M", None).unwrap();
        assert!(list_mods(dir.path()).unwrap()[0].files.is_none());
        assert!(load_library(dir.path()).unwrap().mods.is_empty());
        assert!(set_mod_files(dir.path(), "Ghost", None).is_err());
    }

    #[test]
    fn delete_removes_the_folder_and_its_rules_only() {
        let dir = TempDir::new().unwrap();
        let gone = add_mod(dir.path(), "Gone", Some(vec![rule("a", "b")])).unwrap();
        fs::write(gone.join("file.txt"), "x").unwrap();
        add_mod(dir.path(), "Stays", Some(vec![rule("c", "d")])).unwrap();

        delete_mod(dir.path(), "Gone").unwrap();

        assert_eq!(names(dir.path()), ["Stays"]);
        let library = load_library(dir.path()).unwrap();
        assert_eq!(library.mods.keys().collect::<Vec<_>>(), ["Stays"]);
        assert!(delete_mod(dir.path(), "Gone").is_err());
    }

    #[test]
    fn names_that_could_escape_staging_are_rejected_everywhere() {
        let dir = TempDir::new().unwrap();
        let staging = dir.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir(dir.path().join("victim")).unwrap();

        for bad in ["../victim", "..", "a/../../victim"] {
            assert!(delete_mod(&staging, bad).is_err(), "{bad}");
            assert!(rename_mod(&staging, bad, "x").is_err(), "{bad}");
            assert!(set_mod_files(&staging, bad, None).is_err(), "{bad}");
            assert!(mod_dir(&staging, bad).is_err(), "{bad}");
        }
        assert!(dir.path().join("victim").is_dir());
    }
}
