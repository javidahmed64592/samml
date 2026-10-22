//! Turning a profile into what deploy needs: its active mods, in order,
//! joined with their deploy rules from the library.
//!
//! Reports every mod the profile names that is missing from staging, so the
//! player can fix them all at once.

use crate::library;
use crate::manifest::{DeployMod, Profile};
use anyhow::{Result, bail};
use std::path::Path;

/// Names of the profile's mods (active or not) that have no folder in
/// staging.
pub fn missing_mods(staging_dir: &Path, profile: &Profile) -> Result<Vec<String>> {
    let mut missing = Vec::new();
    for entry in &profile.mods {
        let present = library::mod_dir(staging_dir, &entry.name)
            .map(|dir| dir.is_dir())
            .unwrap_or(false);
        if !present {
            missing.push(entry.name.clone());
        }
    }
    Ok(missing)
}

/// The profile's active mods in deploy order. Fails, naming every missing
/// mod, if any active mod has no folder in staging.
pub fn resolve_profile(staging_dir: &Path, profile: &Profile) -> Result<Vec<DeployMod>> {
    let missing: Vec<_> = missing_mods(staging_dir, profile)?
        .into_iter()
        .filter(|name| profile.mods.iter().any(|m| &m.name == name && m.active))
        .collect();
    if !missing.is_empty() {
        bail!(
            "profile '{}' uses mods that are not in {}: {}",
            profile.name,
            staging_dir.display(),
            missing.join(", ")
        );
    }

    let mut library = library::load_library(staging_dir)?;
    Ok(profile
        .mods
        .iter()
        .filter(|m| m.active)
        .map(|m| DeployMod {
            files: library.mods.get_mut(&m.name).and_then(|s| s.files.take()),
            name: m.name.clone(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{FileRule, ProfileMod};
    use tempfile::TempDir;

    fn entry(name: &str, active: bool) -> ProfileMod {
        ProfileMod {
            name: name.into(),
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
    fn resolves_active_mods_in_profile_order_with_their_rules() {
        let dir = TempDir::new().unwrap();
        let rules = vec![FileRule {
            source: "a".into(),
            target: "b".into(),
        }];
        library::add_mod(dir.path(), "First", Some(rules.clone())).unwrap();
        library::add_mod(dir.path(), "Second", None).unwrap();
        library::add_mod(dir.path(), "Off", None).unwrap();
        let p = profile(vec![
            entry("Second", true),
            entry("Off", false),
            entry("First", true),
        ]);

        let resolved = resolve_profile(dir.path(), &p).unwrap();

        let names: Vec<_> = resolved.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Second", "First"]);
        assert_eq!(resolved[0].files, None);
        assert_eq!(resolved[1].files, Some(rules));
    }

    #[test]
    fn names_every_missing_active_mod() {
        let dir = TempDir::new().unwrap();
        library::add_mod(dir.path(), "Here", None).unwrap();
        let p = profile(vec![
            entry("Here", true),
            entry("Gone1", true),
            entry("Gone2", true),
            entry("Gone3", false),
        ]);

        let err = resolve_profile(dir.path(), &p).unwrap_err().to_string();

        assert!(err.contains("Gone1, Gone2"), "{err}");
        assert!(
            !err.contains("Gone3"),
            "inactive mods are not needed: {err}"
        );
        assert_eq!(
            missing_mods(dir.path(), &p).unwrap(),
            ["Gone1", "Gone2", "Gone3"]
        );
    }

    #[test]
    fn a_missing_library_file_means_default_deployment() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("Plain")).unwrap();

        let resolved = resolve_profile(dir.path(), &profile(vec![entry("Plain", true)])).unwrap();

        assert_eq!(resolved[0].files, None);
    }

    #[test]
    fn entries_with_unsafe_names_count_as_missing() {
        let dir = TempDir::new().unwrap();
        let p = profile(vec![entry("../etc", true)]);
        assert!(resolve_profile(dir.path(), &p).is_err());
    }
}
