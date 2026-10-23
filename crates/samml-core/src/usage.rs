//! Operations that touch both the library (staging) and the profiles: who
//! uses a mod, and deleting a mod everywhere it is used.
//!
//! `library` and `profile_mods` each stay ignorant of the other; this is
//! the one place that keeps them consistent. Renaming a mod does not need
//! anything here: profiles refer to mods by id, so a rename only changes
//! the library (`library::rename_mod`).

use crate::manifest::{AppManifest, Profile, ProfileInfo};
use crate::{library, profile_mods, profiles};
use anyhow::{Result, bail};
use std::path::Path;

fn uses_mod(profile: &Profile, id: &str) -> bool {
    profile.mods.iter().any(|m| m.id == id)
}

/// The profiles that list the mod (active or not), sorted by name.
pub fn profiles_using_mod(profiles_dir: &Path, id: &str) -> Result<Vec<ProfileInfo>> {
    Ok(profiles::load_all(profiles_dir)?
        .into_iter()
        .filter(|(_, profile)| uses_mod(profile, id))
        .map(|(slug, profile)| ProfileInfo {
            slug,
            name: profile.name,
        })
        .collect())
}

/// Deletes the mod's files and removes it from every profile that uses it,
/// returning those profiles. Profiles are updated first, so a failure never
/// leaves a profile pointing at deleted files; if one cannot be written,
/// nothing is deleted.
pub fn delete_mod_everywhere(app: &AppManifest, id: &str) -> Result<Vec<ProfileInfo>> {
    // Fail before changing anything if the mod does not exist.
    library::entry(&library::load_library(&app.staging_dir)?, id)?;

    let mut affected: Vec<(String, Profile)> = profiles::load_all(&app.profiles_dir)?
        .into_iter()
        .filter(|(_, profile)| uses_mod(profile, id))
        .collect();
    for (_, profile) in &mut affected {
        profile_mods::remove_mod(profile, id)?;
    }

    let failures: Vec<String> = affected
        .iter()
        .filter_map(|(slug, profile)| {
            profiles::save_profile(&app.profiles_dir, slug, profile)
                .err()
                .map(|e| format!("'{slug}': {e:#}"))
        })
        .collect();
    if !failures.is_empty() {
        bail!(
            "no files were deleted, but these profiles could not be updated: {}",
            failures.join("; ")
        );
    }

    library::delete_mod(&app.staging_dir, id)?;
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

    struct Setup {
        app: AppManifest,
        a: String,
        b: String,
    }

    /// Two mods and three profiles: "One" uses A and B, "Two" uses B only,
    /// "Three" uses neither.
    fn setup(root: &TempDir) -> Setup {
        let app = AppManifest {
            game_dir: root.path().join("game"),
            staging_dir: root.path().join("staging"),
            profiles_dir: root.path().join("profiles"),
            steam_app_id: None,
        };
        let (a, _) = library::add_mod(&app.staging_dir, "A", None).unwrap();
        let (b, _) = library::add_mod(&app.staging_dir, "B", None).unwrap();
        for (profile_name, mods) in [("One", vec![&a, &b]), ("Two", vec![&b]), ("Three", vec![])] {
            let (slug, _) = profiles::create_profile(&app.profiles_dir, profile_name).unwrap();
            let mut profile = profiles::find_profile_by_slug(&app.profiles_dir, &slug)
                .unwrap()
                .unwrap();
            for id in mods {
                profile_mods::add_mod(&mut profile, id).unwrap();
            }
            profiles::save_profile(&app.profiles_dir, &slug, &profile).unwrap();
        }
        Setup { app, a, b }
    }

    fn mod_ids(app: &AppManifest, slug: &str) -> Vec<String> {
        profiles::find_profile_by_slug(&app.profiles_dir, slug)
            .unwrap()
            .unwrap()
            .mods
            .into_iter()
            .map(|m| m.id)
            .collect()
    }

    fn slugs(infos: &[ProfileInfo]) -> Vec<&str> {
        infos.iter().map(|p| p.slug.as_str()).collect()
    }

    #[test]
    fn finds_the_profiles_using_a_mod() {
        let root = TempDir::new().unwrap();
        let s = setup(&root);

        let using_b = profiles_using_mod(&s.app.profiles_dir, &s.b).unwrap();
        assert_eq!(slugs(&using_b), ["one", "two"]);
        let using_a = profiles_using_mod(&s.app.profiles_dir, &s.a).unwrap();
        assert_eq!(slugs(&using_a), ["one"]);
        assert!(
            profiles_using_mod(&s.app.profiles_dir, "nope")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn renaming_a_mod_leaves_every_profile_untouched() {
        let root = TempDir::new().unwrap();
        let s = setup(&root);

        library::rename_mod(&s.app.staging_dir, &s.b, "Better B").unwrap();

        assert!(s.app.staging_dir.join("Better B").is_dir());
        assert_eq!(mod_ids(&s.app, "one"), [s.a.clone(), s.b.clone()]);
        assert_eq!(mod_ids(&s.app, "two"), std::slice::from_ref(&s.b));
        assert_eq!(
            profiles_using_mod(&s.app.profiles_dir, &s.b).unwrap().len(),
            2
        );
    }

    #[test]
    fn delete_removes_the_files_and_every_profile_entry() {
        let root = TempDir::new().unwrap();
        let s = setup(&root);

        let affected = delete_mod_everywhere(&s.app, &s.b).unwrap();

        assert_eq!(slugs(&affected), ["one", "two"]);
        assert!(!s.app.staging_dir.join("B").exists());
        assert_eq!(mod_ids(&s.app, "one"), std::slice::from_ref(&s.a));
        assert!(mod_ids(&s.app, "two").is_empty());
        assert!(s.app.staging_dir.join("A").is_dir(), "other mods stay");
    }

    #[test]
    fn deleting_an_unknown_mod_changes_nothing() {
        let root = TempDir::new().unwrap();
        let s = setup(&root);

        assert!(delete_mod_everywhere(&s.app, "ghost-0000").is_err());

        assert_eq!(mod_ids(&s.app, "one"), [s.a.clone(), s.b.clone()]);
        assert!(s.app.staging_dir.join("B").is_dir());
    }
}
