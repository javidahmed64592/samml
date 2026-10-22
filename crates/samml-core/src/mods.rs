//! Editing the mod list of a `Profile` in memory: add, remove, rename,
//! reorder, toggle, and set deploy rules.
//!
//! Pure edits plus validation; nothing here writes to disk. Callers load a
//! profile (`profiles`), edit it here, then save it. Copying mod files into
//! staging is `storage`'s job (a later step).
//!
//! A mod's `name` is also the folder modloader lists in game, so it must be
//! a valid folder name; invalid names are rejected with the reason, never
//! quietly changed. Names are unique (case-insensitively) within a profile,
//! and mods are looked up by name. Renaming a mod renames its folder on the
//! next deploy. Where its files live in staging (`mod_path`, a slug chosen
//! when the mod is added) never changes, because other profiles may share it.

use crate::manifest::{FileRule, ModManifest, Profile};
use crate::{paths, slug};
use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;

/// Staging sub-folder that holds mods added through the app.
const MODS_DIR: &str = "mods";

// --- Validation ----------------------------------------------------------

/// `except` is the mod being renamed, so it may keep (or re-case) its own name.
fn validate_name(profile: &Profile, name: &str, except: Option<&str>) -> Result<()> {
    paths::validate_component(name)
        .map_err(|reason| anyhow!("'{name}' is not a valid mod name: {reason}"))?;
    let clash = profile
        .manifests
        .iter()
        .filter(|m| Some(m.name.as_str()) != except)
        .any(|m| m.name.to_lowercase() == name.to_lowercase());
    if clash {
        bail!("this profile already has a mod named '{name}'");
    }
    Ok(())
}

/// Checks every rule stays inside the mod folder and the game folder.
/// A target without a trailing slash is the exact destination, so it must
/// name something (`.` and `./` only make sense as "inside the game folder").
pub fn validate_rules(rules: &[FileRule]) -> Result<()> {
    for rule in rules {
        paths::validate_relative(&rule.source)
            .with_context(|| format!("invalid source '{}'", rule.source))?;
        paths::validate_relative(&rule.target)
            .with_context(|| format!("invalid target '{}'", rule.target))?;
        if !rule.target.ends_with('/') && !paths::names_something(&rule.target) {
            bail!(
                "target '{}' needs a name, or a trailing '/' to place the item inside the game folder",
                rule.target
            );
        }
    }
    Ok(())
}

/// No rules means "deploy to modloader", the same as leaving them blank.
fn normalize_rules(files: Option<Vec<FileRule>>) -> Result<Option<Vec<FileRule>>> {
    match files {
        Some(rules) if !rules.is_empty() => {
            validate_rules(&rules)?;
            Ok(Some(rules))
        }
        _ => Ok(None),
    }
}

// --- Lookups -------------------------------------------------------------

fn position(profile: &Profile, name: &str) -> Result<usize> {
    profile
        .manifests
        .iter()
        .position(|m| m.name == name)
        .with_context(|| format!("no mod named '{name}' in this profile"))
}

fn find_mut<'a>(profile: &'a mut Profile, name: &str) -> Result<&'a mut ModManifest> {
    let index = position(profile, name)?;
    Ok(&mut profile.manifests[index])
}

// --- Edits ---------------------------------------------------------------

/// Appends a new mod and returns its staging folder (relative to
/// STAGING_DIR), where its files should be copied. The folder is
/// `mods/<slug of the name>`, made unique against this profile and what is
/// already in staging; players never see it, so it is a slug. It is not
/// created here.
pub fn add_mod(
    profile: &mut Profile,
    staging_dir: &Path,
    name: &str,
    files: Option<Vec<FileRule>>,
) -> Result<String> {
    validate_name(profile, name, None)?;
    let files = normalize_rules(files)?;
    let folder = slug::unique(&slug::slugify(name), |candidate| {
        let path = format!("{MODS_DIR}/{candidate}");
        profile.manifests.iter().any(|m| m.mod_path == path) || staging_dir.join(&path).exists()
    });
    let mod_path = format!("{MODS_DIR}/{folder}");
    profile.manifests.push(ModManifest {
        name: name.to_string(),
        mod_path: mod_path.clone(),
        active: true,
        files,
    });
    Ok(mod_path)
}

/// Removes the mod from the profile and returns it. Staged files are not
/// touched.
pub fn remove_mod(profile: &mut Profile, name: &str) -> Result<ModManifest> {
    let index = position(profile, name)?;
    Ok(profile.manifests.remove(index))
}

/// Renames the mod. Its modloader folder follows on the next deploy; its
/// staged files do not move.
pub fn rename_mod(profile: &mut Profile, name: &str, new_name: &str) -> Result<()> {
    let index = position(profile, name)?;
    validate_name(profile, new_name, Some(name))?;
    profile.manifests[index].name = new_name.to_string();
    Ok(())
}

pub fn set_mod_active(profile: &mut Profile, name: &str, active: bool) -> Result<()> {
    find_mut(profile, name)?.active = active;
    Ok(())
}

/// Sets where the mod's files are deployed. `None` (or an empty list)
/// deploys the whole mod folder to `modloader/<name>`.
pub fn set_mod_files(
    profile: &mut Profile,
    name: &str,
    files: Option<Vec<FileRule>>,
) -> Result<()> {
    let files = normalize_rules(files)?;
    find_mut(profile, name)?.files = files;
    Ok(())
}

/// Moves the mod so it ends up at `new_index` in deploy order.
pub fn move_mod(profile: &mut Profile, name: &str, new_index: usize) -> Result<()> {
    if new_index >= profile.manifests.len() {
        bail!(
            "position {new_index} is out of range for {} mod(s)",
            profile.manifests.len()
        );
    }
    let index = position(profile, name)?;
    let moved = profile.manifests.remove(index);
    profile.manifests.insert(new_index, moved);
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

    fn profile() -> Profile {
        Profile {
            name: "P".into(),
            manifests: Vec::new(),
        }
    }

    fn names(profile: &Profile) -> Vec<&str> {
        profile.manifests.iter().map(|m| m.name.as_str()).collect()
    }

    fn with_mods(staging: &TempDir, mod_names: &[&str]) -> Profile {
        let mut p = profile();
        for n in mod_names {
            add_mod(&mut p, staging.path(), n, None).unwrap();
        }
        p
    }

    // --- add ---

    #[test]
    fn add_mod_keeps_the_name_and_slugs_the_staging_folder() {
        let staging = TempDir::new().unwrap();
        let mut p = profile();

        let mod_path = add_mod(&mut p, staging.path(), "Cool Car Pack", None).unwrap();

        assert_eq!(mod_path, "mods/cool_car_pack");
        let m = &p.manifests[0];
        assert_eq!(m.name, "Cool Car Pack");
        assert_eq!(m.mod_path, mod_path);
        assert!(m.active);
        assert!(m.files.is_none());
    }

    #[test]
    fn add_mod_numbers_clashing_staging_folders() {
        let staging = TempDir::new().unwrap();
        std::fs::create_dir_all(staging.path().join("mods/taken")).unwrap();
        let mut p = profile();

        // all three slug to "taken", so each needs its own folder
        add_mod(&mut p, staging.path(), "Taken", None).unwrap();
        add_mod(&mut p, staging.path(), "Taken!", None).unwrap();
        add_mod(&mut p, staging.path(), "TAKEN?", None).unwrap();

        let folders: Vec<_> = p.manifests.iter().map(|m| m.mod_path.as_str()).collect();
        assert_eq!(folders, ["mods/taken_2", "mods/taken_3", "mods/taken_4"]);
    }

    #[test]
    fn add_mod_rejects_invalid_names_with_the_reason() {
        let staging = TempDir::new().unwrap();
        let mut p = profile();
        let cases = [
            ("", "cannot be empty"),
            ("  ", "start or end with a space"),
            (" Mod", "start or end with a space"),
            ("a/b", "cannot contain '/'"),
            ("a\\b", "cannot contain '\\'"),
            (".hidden", "start with a '.'"),
            ("bad\tname", "control characters"),
        ];
        for (name, reason) in cases {
            let err = add_mod(&mut p, staging.path(), name, None).unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("not a valid mod name"), "{name:?}: {msg}");
            assert!(msg.contains(reason), "{name:?}: {msg}");
        }
        assert!(p.manifests.is_empty());
    }

    #[test]
    fn add_mod_rejects_duplicate_names_ignoring_case() {
        let staging = TempDir::new().unwrap();
        let mut p = with_mods(&staging, &["Mod"]);

        let err = add_mod(&mut p, staging.path(), "MOD", None).unwrap_err();

        assert!(err.to_string().contains("already has a mod named 'MOD'"));
        assert_eq!(p.manifests.len(), 1);
    }

    #[test]
    fn add_mod_validates_rules_and_treats_empty_as_default() {
        let staging = TempDir::new().unwrap();
        let mut p = profile();

        assert!(add_mod(&mut p, staging.path(), "Bad", Some(vec![rule("../x", "y")])).is_err());
        assert!(p.manifests.is_empty());

        add_mod(&mut p, staging.path(), "Empty", Some(vec![])).unwrap();
        assert!(p.manifests[0].files.is_none());
    }

    // --- rules ---

    #[test]
    fn rules_must_stay_inside_their_folders() {
        assert!(validate_rules(&[rule("a/", "b/"), rule("file.txt", "x/y.txt")]).is_ok());
        assert!(
            validate_rules(&[rule("dir", "./")]).is_ok(),
            "into game folder"
        );
        assert!(validate_rules(&[rule("../x", "y")]).is_err());
        assert!(validate_rules(&[rule("x", "../y")]).is_err());
        assert!(validate_rules(&[rule("/etc/passwd", "y")]).is_err());
        assert!(validate_rules(&[rule("x", "/usr/bin")]).is_err());
        assert!(validate_rules(&[rule("", "y")]).is_err());
        assert!(validate_rules(&[rule("x", "")]).is_err());
    }

    #[test]
    fn exact_target_must_name_something() {
        assert!(validate_rules(&[rule("x", ".")]).is_err());
        assert!(validate_rules(&[rule("x", "./")]).is_ok());
    }

    #[test]
    fn set_mod_files_replaces_and_clears_rules() {
        let staging = TempDir::new().unwrap();
        let mut p = with_mods(&staging, &["M"]);

        set_mod_files(&mut p, "M", Some(vec![rule("a.txt", "b/")])).unwrap();
        assert_eq!(p.manifests[0].files.as_ref().unwrap().len(), 1);

        assert!(set_mod_files(&mut p, "M", Some(vec![rule("..", "x")])).is_err());
        assert_eq!(p.manifests[0].files.as_ref().unwrap().len(), 1, "unchanged");

        set_mod_files(&mut p, "M", None).unwrap();
        assert!(p.manifests[0].files.is_none());
    }

    // --- rename / toggle / remove / move ---

    #[test]
    fn rename_changes_the_name_but_not_the_staging_folder() {
        let staging = TempDir::new().unwrap();
        let mut p = with_mods(&staging, &["Old", "Other"]);

        rename_mod(&mut p, "Old", "New Name").unwrap();

        assert_eq!(names(&p), ["New Name", "Other"]);
        assert_eq!(p.manifests[0].mod_path, "mods/old");
    }

    #[test]
    fn rename_validates_the_new_name_and_leaves_the_mod_unchanged_on_error() {
        let staging = TempDir::new().unwrap();
        let mut p = with_mods(&staging, &["Old", "Other"]);

        let err = rename_mod(&mut p, "Old", "a/b").unwrap_err();
        assert!(err.to_string().contains("cannot contain '/'"));
        assert!(rename_mod(&mut p, "Old", "other").is_err(), "name taken");
        assert!(rename_mod(&mut p, "Old", "").is_err());
        assert!(rename_mod(&mut p, "Ghost", "Fine").is_err(), "unknown mod");
        assert_eq!(names(&p), ["Old", "Other"]);

        rename_mod(&mut p, "Old", "OLD").unwrap(); // own name, new case
        assert_eq!(names(&p), ["OLD", "Other"]);
    }

    #[test]
    fn set_active_toggles_the_mod() {
        let staging = TempDir::new().unwrap();
        let mut p = with_mods(&staging, &["M"]);

        set_mod_active(&mut p, "M", false).unwrap();
        assert!(!p.manifests[0].active);
        assert!(set_mod_active(&mut p, "ghost", true).is_err());
    }

    #[test]
    fn remove_returns_the_mod_and_keeps_order() {
        let staging = TempDir::new().unwrap();
        let mut p = with_mods(&staging, &["A", "B", "C"]);

        let removed = remove_mod(&mut p, "B").unwrap();

        assert_eq!(removed.name, "B");
        assert_eq!(names(&p), ["A", "C"]);
        assert!(remove_mod(&mut p, "B").is_err());
    }

    #[test]
    fn move_mod_reorders() {
        let staging = TempDir::new().unwrap();
        let mut p = with_mods(&staging, &["A", "B", "C", "D"]);

        move_mod(&mut p, "D", 0).unwrap();
        assert_eq!(names(&p), ["D", "A", "B", "C"]);
        move_mod(&mut p, "D", 3).unwrap();
        assert_eq!(names(&p), ["A", "B", "C", "D"]);
        move_mod(&mut p, "A", 2).unwrap();
        assert_eq!(names(&p), ["B", "C", "A", "D"]);

        assert!(move_mod(&mut p, "A", 4).is_err());
        assert!(move_mod(&mut p, "ghost", 0).is_err());
    }
}
