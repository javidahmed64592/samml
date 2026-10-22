//! Editing the mod list of a `Profile` in memory: add, remove, rename,
//! reorder, toggle, and set deploy rules.
//!
//! Pure edits plus validation; nothing here writes to disk. Callers load a
//! profile (`profiles`), edit it here, then save it.
//! Copying mod files into staging is `storage`'s job (a later step, `storage`).
//!
//! Every mod has an immutable `id`: a folder-safe name that is used as its
//! `modloader/<id>` link name. The display `name` is free text and is never
//! used as a path. Ids are unique within a profile; names are unique
//! (case-insensitively) within a profile.

use crate::manifest::{FileRule, ModManifest, Profile};
use crate::{paths, slug};
use anyhow::{Context, Result, bail};
use std::path::Path;

/// Staging sub-folder that holds mods added through the app.
const MODS_DIR: &str = "mods";

// --- Validation ----------------------------------------------------------

fn validate_name(profile: &Profile, name: &str, except_id: Option<&str>) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("mod name cannot be empty");
    }
    let clash = profile
        .manifests
        .iter()
        .filter(|m| Some(m.id.as_str()) != except_id)
        .any(|m| m.name.to_lowercase() == name.to_lowercase());
    if clash {
        bail!("this profile already has a mod named '{name}'");
    }
    Ok(name.to_string())
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

// --- Ids -----------------------------------------------------------------

/// Gives every mod without an id one, so profiles written before ids
/// existed keep working. The id keeps the mod's old modloader folder name
/// (its `name`) when that is a valid folder name, so nothing moves in the
/// game directory. Deterministic, so loading twice gives the same ids.
pub fn assign_missing_ids(profile: &mut Profile) {
    for i in 0..profile.manifests.len() {
        if !profile.manifests[i].id.is_empty() {
            continue;
        }
        let name = profile.manifests[i].name.clone();
        let base = if paths::validate_component(&name).is_ok() {
            name
        } else {
            slug::slugify(&name)
        };
        let id = slug::unique(&base, |candidate| {
            profile.manifests.iter().any(|m| m.id == candidate)
        });
        profile.manifests[i].id = id;
    }
}

// --- Lookups -------------------------------------------------------------

fn position(profile: &Profile, id: &str) -> Result<usize> {
    profile
        .manifests
        .iter()
        .position(|m| m.id == id)
        .with_context(|| format!("no mod with id '{id}' in this profile"))
}

fn find_mut<'a>(profile: &'a mut Profile, id: &str) -> Result<&'a mut ModManifest> {
    let index = position(profile, id)?;
    Ok(&mut profile.manifests[index])
}

// --- Edits ---------------------------------------------------------------

/// Appends a new mod and returns its id. The id and its staging folder
/// (`mods/<id>`) are derived from the name and chosen so they collide with
/// nothing in this profile or already in staging. The folder itself is not
/// created here.
pub fn add_mod(
    profile: &mut Profile,
    staging_dir: &Path,
    name: &str,
    files: Option<Vec<FileRule>>,
) -> Result<String> {
    let name = validate_name(profile, name, None)?;
    let files = normalize_rules(files)?;
    let id = slug::unique(&slug::slugify(&name), |candidate| {
        profile.manifests.iter().any(|m| m.id == candidate)
            || staging_dir.join(MODS_DIR).join(candidate).exists()
    });
    profile.manifests.push(ModManifest {
        name,
        id: id.clone(),
        mod_path: format!("{MODS_DIR}/{id}"),
        active: true,
        files,
    });
    Ok(id)
}

/// Removes the mod from the profile and returns it. Staged files are not
/// touched.
pub fn remove_mod(profile: &mut Profile, id: &str) -> Result<ModManifest> {
    let index = position(profile, id)?;
    Ok(profile.manifests.remove(index))
}

/// Changes the display name only; the id and folder never change.
pub fn rename_mod(profile: &mut Profile, id: &str, new_name: &str) -> Result<()> {
    let name = validate_name(profile, new_name, Some(id))?;
    find_mut(profile, id)?.name = name;
    Ok(())
}

pub fn set_mod_active(profile: &mut Profile, id: &str, active: bool) -> Result<()> {
    find_mut(profile, id)?.active = active;
    Ok(())
}

/// Sets where the mod's files are deployed. `None` (or an empty list)
/// deploys the whole mod folder to `modloader/<id>`.
pub fn set_mod_files(profile: &mut Profile, id: &str, files: Option<Vec<FileRule>>) -> Result<()> {
    let files = normalize_rules(files)?;
    find_mut(profile, id)?.files = files;
    Ok(())
}

/// Moves the mod so it ends up at `new_index` in deploy order.
pub fn move_mod(profile: &mut Profile, id: &str, new_index: usize) -> Result<()> {
    if new_index >= profile.manifests.len() {
        bail!(
            "position {new_index} is out of range for {} mod(s)",
            profile.manifests.len()
        );
    }
    let index = position(profile, id)?;
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

    fn ids(profile: &Profile) -> Vec<&str> {
        profile.manifests.iter().map(|m| m.id.as_str()).collect()
    }

    fn legacy_mod(name: &str) -> ModManifest {
        ModManifest {
            name: name.into(),
            id: String::new(),
            mod_path: format!("mods/{name}"),
            active: true,
            files: None,
        }
    }

    // --- ids ---

    #[test]
    fn migration_keeps_the_old_folder_name() {
        let mut p = profile();
        p.manifests = vec![legacy_mod("Test Mod 1"), legacy_mod("CLEO 5.4.0")];
        assign_missing_ids(&mut p);
        assert_eq!(ids(&p), ["Test Mod 1", "CLEO 5.4.0"]);
    }

    #[test]
    fn migration_slugifies_names_that_are_not_folder_safe() {
        let mut p = profile();
        p.manifests = vec![legacy_mod("a/b"), legacy_mod(".hidden")];
        assign_missing_ids(&mut p);
        assert_eq!(ids(&p), ["ab", "hidden"]);
    }

    #[test]
    fn migration_keeps_existing_ids_and_makes_duplicates_unique() {
        let mut p = profile();
        let mut keep = legacy_mod("Keep");
        keep.id = "custom".into();
        p.manifests = vec![keep, legacy_mod("Same"), legacy_mod("Same")];
        assign_missing_ids(&mut p);
        assert_eq!(ids(&p), ["custom", "Same", "Same_2"]);

        let before = ids(&p).join(",");
        assign_missing_ids(&mut p);
        assert_eq!(ids(&p).join(","), before, "idempotent");
    }

    // --- add ---

    #[test]
    fn add_mod_derives_id_and_folder_from_the_name() {
        let staging = TempDir::new().unwrap();
        let mut p = profile();

        let id = add_mod(&mut p, staging.path(), "  Cool Car Pack ", None).unwrap();

        assert_eq!(id, "cool_car_pack");
        let m = &p.manifests[0];
        assert_eq!(m.name, "Cool Car Pack");
        assert_eq!(m.mod_path, "mods/cool_car_pack");
        assert!(m.active);
        assert!(m.files.is_none());
    }

    #[test]
    fn add_mod_avoids_ids_used_in_the_profile_or_in_staging() {
        let staging = TempDir::new().unwrap();
        std::fs::create_dir_all(staging.path().join("mods/taken")).unwrap();
        let mut p = profile();

        let a = add_mod(&mut p, staging.path(), "Taken", None).unwrap();
        let b = add_mod(&mut p, staging.path(), "Taken!", None).unwrap();

        assert_eq!((a.as_str(), b.as_str()), ("taken_2", "taken_3"));
    }

    #[test]
    fn add_mod_rejects_blank_and_duplicate_names() {
        let staging = TempDir::new().unwrap();
        let mut p = profile();
        add_mod(&mut p, staging.path(), "Mod", None).unwrap();

        assert!(add_mod(&mut p, staging.path(), "  ", None).is_err());
        assert!(add_mod(&mut p, staging.path(), "MOD", None).is_err());
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
        let mut p = profile();
        let id = add_mod(&mut p, staging.path(), "M", None).unwrap();

        set_mod_files(&mut p, &id, Some(vec![rule("a.txt", "b/")])).unwrap();
        assert_eq!(p.manifests[0].files.as_ref().unwrap().len(), 1);

        assert!(set_mod_files(&mut p, &id, Some(vec![rule("..", "x")])).is_err());
        assert_eq!(p.manifests[0].files.as_ref().unwrap().len(), 1, "unchanged");

        set_mod_files(&mut p, &id, None).unwrap();
        assert!(p.manifests[0].files.is_none());
    }

    // --- rename / toggle / remove / move ---

    #[test]
    fn rename_changes_the_name_but_not_the_id_or_folder() {
        let staging = TempDir::new().unwrap();
        let mut p = profile();
        let id = add_mod(&mut p, staging.path(), "Old", None).unwrap();
        add_mod(&mut p, staging.path(), "Other", None).unwrap();

        rename_mod(&mut p, &id, "New Name").unwrap();
        assert_eq!(p.manifests[0].name, "New Name");
        assert_eq!(p.manifests[0].id, id);
        assert_eq!(p.manifests[0].mod_path, "mods/old");

        assert!(rename_mod(&mut p, &id, "other").is_err(), "name taken");
        assert!(rename_mod(&mut p, &id, "").is_err());
        rename_mod(&mut p, &id, "NEW NAME").unwrap(); // own name, new case
    }

    #[test]
    fn set_active_toggles_the_mod() {
        let staging = TempDir::new().unwrap();
        let mut p = profile();
        let id = add_mod(&mut p, staging.path(), "M", None).unwrap();

        set_mod_active(&mut p, &id, false).unwrap();
        assert!(!p.manifests[0].active);
        assert!(set_mod_active(&mut p, "ghost", true).is_err());
    }

    #[test]
    fn remove_returns_the_mod_and_keeps_order() {
        let staging = TempDir::new().unwrap();
        let mut p = profile();
        for n in ["A", "B", "C"] {
            add_mod(&mut p, staging.path(), n, None).unwrap();
        }

        let removed = remove_mod(&mut p, "b").unwrap();

        assert_eq!(removed.name, "B");
        assert_eq!(ids(&p), ["a", "c"]);
        assert!(remove_mod(&mut p, "b").is_err());
    }

    #[test]
    fn move_mod_reorders() {
        let staging = TempDir::new().unwrap();
        let mut p = profile();
        for n in ["A", "B", "C", "D"] {
            add_mod(&mut p, staging.path(), n, None).unwrap();
        }

        move_mod(&mut p, "d", 0).unwrap();
        assert_eq!(ids(&p), ["d", "a", "b", "c"]);
        move_mod(&mut p, "d", 3).unwrap();
        assert_eq!(ids(&p), ["a", "b", "c", "d"]);
        move_mod(&mut p, "a", 2).unwrap();
        assert_eq!(ids(&p), ["b", "c", "a", "d"]);

        assert!(move_mod(&mut p, "a", 4).is_err());
        assert!(move_mod(&mut p, "ghost", 0).is_err());
    }
}
