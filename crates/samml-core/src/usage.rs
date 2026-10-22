//! Operations that touch both the library (staging) and the profiles: who
//! uses a mod, and renaming or deleting a mod everywhere it is used.
//!
//! `library` and `profile_mods` each stay ignorant of the other; this is
//! the one place that keeps them consistent. Profiles refer to mods by
//! name, so a rename must rewrite every profile that lists the mod. If a
//! profile cannot be written, the error says which, and the stale entry
//! shows up as a missing mod (`resolve::missing_mods`) until fixed.

use crate::manifest::{AppManifest, Profile, ProfileInfo};
use crate::{library, profile_mods, profiles};
use anyhow::{Result, bail};
use std::path::Path;

fn uses_mod(profile: &Profile, name: &str) -> bool {
    profile.mods.iter().any(|m| m.name == name)
}

/// The profiles that list the mod (active or not), sorted by name.
pub fn profiles_using_mod(profiles_dir: &Path, name: &str) -> Result<Vec<ProfileInfo>> {
    Ok(profiles::load_all(profiles_dir)?
        .into_iter()
        .filter(|(_, profile)| uses_mod(profile, name))
        .map(|(slug, profile)| ProfileInfo {
            slug,
            name: profile.name,
        })
        .collect())
}

/// Saves each edited profile, trying them all, and reports every failure.
fn save_all(profiles_dir: &Path, edited: &[(String, Profile)], what: &str) -> Result<()> {
    let failures: Vec<String> = edited
        .iter()
        .filter_map(|(slug, profile)| {
            profiles::save_profile(profiles_dir, slug, profile)
                .err()
                .map(|e| format!("'{slug}': {e:#}"))
        })
        .collect();
    if !failures.is_empty() {
        bail!(
            "{what}, but these profiles could not be updated: {}",
            failures.join("; ")
        );
    }
    Ok(())
}

/// Renames the mod's folder and every profile entry that refers to it.
/// Everything is read and the new name validated before anything changes.
pub fn rename_mod_everywhere(app: &AppManifest, name: &str, new_name: &str) -> Result<()> {
    let mut affected: Vec<(String, Profile)> = profiles::load_all(&app.profiles_dir)?
        .into_iter()
        .filter(|(_, profile)| uses_mod(profile, name))
        .collect();

    library::rename_mod(&app.staging_dir, name, new_name)?;

    for (_, profile) in &mut affected {
        for entry in profile.mods.iter_mut().filter(|m| m.name == name) {
            entry.name = new_name.to_string();
        }
    }
    save_all(
        &app.profiles_dir,
        &affected,
        &format!("'{name}' was renamed to '{new_name}'"),
    )
}

/// Deletes the mod's files and removes it from every profile that uses it,
/// returning those profiles. Profiles are updated first, so a failure never
/// leaves a profile pointing at deleted files.
pub fn delete_mod_everywhere(app: &AppManifest, name: &str) -> Result<Vec<ProfileInfo>> {
    // Fail before changing anything if the mod does not exist.
    library::mod_dir(&app.staging_dir, name)
        .ok()
        .filter(|dir| dir.is_dir())
        .ok_or_else(|| anyhow::anyhow!("there is no mod named '{name}'"))?;

    let mut affected: Vec<(String, Profile)> = profiles::load_all(&app.profiles_dir)?
        .into_iter()
        .filter(|(_, profile)| uses_mod(profile, name))
        .collect();
    for (_, profile) in &mut affected {
        profile_mods::remove_mod(profile, name)?;
    }
    save_all(&app.profiles_dir, &affected, "no files were deleted")?;

    library::delete_mod(&app.staging_dir, name)?;
    Ok(affected
        .into_iter()
        .map(|(slug, profile)| ProfileInfo {
            slug,
            name: profile.name,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn app(root: &TempDir) -> AppManifest {
        AppManifest {
            game_dir: root.path().join("game"),
            staging_dir: root.path().join("staging"),
            profiles_dir: root.path().join("profiles"),
            steam_app_id: None,
        }
    }

    /// Two profiles ("One" uses A and B, "Two" uses B only) and a third that
    /// uses neither.
    fn setup(root: &TempDir) -> AppManifest {
        let app = app(root);
        for name in ["A", "B"] {
            library::add_mod(&app.staging_dir, name, None).unwrap();
        }
        for (profile_name, mods) in [("One", &["A", "B"][..]), ("Two", &["B"]), ("Three", &[])] {
            let (slug, _) = profiles::create_profile(&app.profiles_dir, profile_name).unwrap();
            let mut profile = profiles::find_profile_by_slug(&app.profiles_dir, &slug)
                .unwrap()
                .unwrap();
            for m in mods {
                profile_mods::add_mod(&mut profile, m).unwrap();
            }
            profiles::save_profile(&app.profiles_dir, &slug, &profile).unwrap();
        }
        app
    }

    fn mod_names(app: &AppManifest, slug: &str) -> Vec<String> {
        profiles::find_profile_by_slug(&app.profiles_dir, slug)
            .unwrap()
            .unwrap()
            .mods
            .into_iter()
            .map(|m| m.name)
            .collect()
    }

    fn slugs(infos: &[ProfileInfo]) -> Vec<&str> {
        infos.iter().map(|p| p.slug.as_str()).collect()
    }

    #[test]
    fn finds_the_profiles_using_a_mod() {
        let root = TempDir::new().unwrap();
        let app = setup(&root);

        let using_b = profiles_using_mod(&app.profiles_dir, "B").unwrap();
        assert_eq!(slugs(&using_b), ["one", "two"]);
        let using_a = profiles_using_mod(&app.profiles_dir, "A").unwrap();
        assert_eq!(slugs(&using_a), ["one"]);
        assert!(
            profiles_using_mod(&app.profiles_dir, "Nope")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn rename_updates_the_folder_and_every_profile_using_the_mod() {
        let root = TempDir::new().unwrap();
        let app = setup(&root);

        rename_mod_everywhere(&app, "B", "Better B").unwrap();

        assert!(app.staging_dir.join("Better B").is_dir());
        assert!(!app.staging_dir.join("B").exists());
        assert_eq!(mod_names(&app, "one"), ["A", "Better B"]);
        assert_eq!(mod_names(&app, "two"), ["Better B"]);
        assert!(mod_names(&app, "three").is_empty());
    }

    #[test]
    fn invalid_rename_changes_nothing_anywhere() {
        let root = TempDir::new().unwrap();
        let app = setup(&root);

        assert!(rename_mod_everywhere(&app, "B", "bad/name").is_err());
        assert!(
            rename_mod_everywhere(&app, "B", "a").is_err(),
            "clashes with A"
        );

        assert!(app.staging_dir.join("B").is_dir());
        assert_eq!(mod_names(&app, "one"), ["A", "B"]);
        assert_eq!(mod_names(&app, "two"), ["B"]);
    }

    #[test]
    fn delete_removes_the_files_and_every_profile_entry() {
        let root = TempDir::new().unwrap();
        let app = setup(&root);

        let affected = delete_mod_everywhere(&app, "B").unwrap();

        assert_eq!(slugs(&affected), ["one", "two"]);
        assert!(!app.staging_dir.join("B").exists());
        assert_eq!(mod_names(&app, "one"), ["A"]);
        assert!(mod_names(&app, "two").is_empty());
        assert!(app.staging_dir.join("A").is_dir(), "other mods stay");
    }

    #[test]
    fn deleting_an_unknown_mod_changes_nothing() {
        let root = TempDir::new().unwrap();
        let app = setup(&root);

        assert!(delete_mod_everywhere(&app, "Ghost").is_err());
        assert!(delete_mod_everywhere(&app, "../x").is_err());

        assert_eq!(mod_names(&app, "one"), ["A", "B"]);
    }
}
