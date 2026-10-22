//! Turning a profile into what deploy needs: its active mods, in order,
//! joined with their names and deploy rules from the library.
//!
//! `describe_profile` is also what the UI and CLI use to show a profile: it
//! says, for every entry, which mod it is and whether it can be deployed.
//! A problem is reported for every mod that has one, so the player can fix
//! them all at once.

use crate::library;
use crate::manifest::{DeployMod, Library, Profile, ProfileModView};
use crate::paths;
use anyhow::{Result, bail};
use std::path::Path;

fn problem_for(
    staging_dir: &Path,
    library: &Library,
    id: &str,
) -> (Option<String>, Option<String>) {
    let Some(settings) = library.mods.get(id) else {
        return (None, Some("it is not in the library".to_string()));
    };
    let name = Some(settings.name.clone());
    let problem = match paths::validate_component(&settings.name) {
        Err(reason) => Some(format!(
            "'{}' cannot be used as a mod name: {reason}",
            settings.name
        )),
        Ok(()) if !staging_dir.join(&settings.name).is_dir() => Some(format!(
            "its folder '{}' is missing from staging",
            settings.name
        )),
        Ok(()) => None,
    };
    (name, problem)
}

/// Every entry of the profile (active or not) with its mod's name and any
/// problem. Read-only.
pub fn describe_profile(staging_dir: &Path, profile: &Profile) -> Result<Vec<ProfileModView>> {
    let library = library::load_library(staging_dir)?;
    Ok(profile
        .mods
        .iter()
        .map(|entry| {
            let (name, problem) = problem_for(staging_dir, &library, &entry.id);
            ProfileModView {
                id: entry.id.clone(),
                name,
                active: entry.active,
                problem,
            }
        })
        .collect())
}

/// The profile's active mods in deploy order. Fails, naming every active
/// mod that has a problem, before anything is deployed.
pub fn resolve_profile(staging_dir: &Path, profile: &Profile) -> Result<Vec<DeployMod>> {
    let views = describe_profile(staging_dir, profile)?;
    let broken: Vec<String> = views
        .iter()
        .filter(|v| v.active)
        .filter_map(|v| {
            v.problem
                .as_ref()
                .map(|p| format!("{} ({p})", v.name.as_deref().unwrap_or(&v.id)))
        })
        .collect();
    if !broken.is_empty() {
        bail!(
            "profile '{}' cannot be deployed: {}",
            profile.name,
            broken.join("; ")
        );
    }

    let library = library::load_library(staging_dir)?;
    Ok(views
        .into_iter()
        .filter(|v| v.active)
        .filter_map(|v| {
            let settings = library.mods.get(&v.id)?;
            Some(DeployMod {
                name: settings.name.clone(),
                files: settings.files.clone(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{FileRule, ProfileMod};
    use tempfile::TempDir;

    fn entry(id: &str, active: bool) -> ProfileMod {
        ProfileMod {
            id: id.into(),
            active,
        }
    }

    fn profile(mods: Vec<ProfileMod>) -> Profile {
        Profile {
            name: "P".into(),
            mods,
        }
    }

    #[test]
    fn resolves_active_mods_in_profile_order_with_names_and_rules() {
        let dir = TempDir::new().unwrap();
        let rules = vec![FileRule {
            source: "a".into(),
            target: "b".into(),
        }];
        let (first, _) = library::add_mod(dir.path(), "First", Some(rules.clone())).unwrap();
        let (second, _) = library::add_mod(dir.path(), "Second", None).unwrap();
        let (off, _) = library::add_mod(dir.path(), "Off", None).unwrap();
        let p = profile(vec![
            entry(&second, true),
            entry(&off, false),
            entry(&first, true),
        ]);

        let resolved = resolve_profile(dir.path(), &p).unwrap();

        let names: Vec<_> = resolved.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Second", "First"]);
        assert_eq!(resolved[0].files, None);
        assert_eq!(resolved[1].files, Some(rules));
    }

    #[test]
    fn a_renamed_mod_still_resolves_under_its_new_name() {
        let dir = TempDir::new().unwrap();
        let (id, _) = library::add_mod(dir.path(), "Old", None).unwrap();
        let p = profile(vec![entry(&id, true)]);

        library::rename_mod(dir.path(), &id, "New").unwrap();

        let resolved = resolve_profile(dir.path(), &p).unwrap();
        assert_eq!(resolved[0].name, "New");
    }

    #[test]
    fn describes_every_entry_and_names_what_is_wrong() {
        let dir = TempDir::new().unwrap();
        let (ok, _) = library::add_mod(dir.path(), "Fine", None).unwrap();
        let (no_folder, path) = library::add_mod(dir.path(), "NoFolder", None).unwrap();
        std::fs::remove_dir(path).unwrap();
        let p = profile(vec![
            entry(&ok, true),
            entry(&no_folder, false),
            entry("unknown-0000", true),
        ]);

        let views = describe_profile(dir.path(), &p).unwrap();

        assert_eq!(views[0].name.as_deref(), Some("Fine"));
        assert!(views[0].problem.is_none());
        assert_eq!(views[1].name.as_deref(), Some("NoFolder"));
        assert!(!views[1].active);
        assert!(views[1].problem.as_ref().unwrap().contains("folder"));
        assert_eq!(views[2].name, None);
        assert!(
            views[2]
                .problem
                .as_ref()
                .unwrap()
                .contains("not in the library")
        );
    }

    #[test]
    fn names_every_broken_active_mod_and_ignores_inactive_ones() {
        let dir = TempDir::new().unwrap();
        let (here, _) = library::add_mod(dir.path(), "Here", None).unwrap();
        let (gone, path) = library::add_mod(dir.path(), "Gone", None).unwrap();
        std::fs::remove_dir(path).unwrap();
        let p = profile(vec![
            entry(&here, true),
            entry(&gone, true),
            entry("unknown-0000", true),
            entry("ignored-0000", false),
        ]);

        let err = resolve_profile(dir.path(), &p).unwrap_err().to_string();

        assert!(err.contains("Gone"), "{err}");
        assert!(err.contains("unknown-0000"), "{err}");
        assert!(!err.contains("ignored-0000"), "{err}");
        assert!(!err.contains("Here"), "{err}");
    }

    #[test]
    fn unsafe_names_in_a_hand_edited_library_are_problems() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(library::LIBRARY_FILE),
            r#"{ "mods": { "evil-0000": { "name": "../etc" } } }"#,
        )
        .unwrap();
        let p = profile(vec![entry("evil-0000", true)]);

        assert!(resolve_profile(dir.path(), &p).is_err());
    }
}
