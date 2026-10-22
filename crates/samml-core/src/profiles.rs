//! Profiles on disk: `PROFILES_DIR/<slug>.json`, one file per profile.
//!
//! Owns reading, listing, creating, saving, renaming, duplicating and
//! deleting profile files. The slug (the file stem) is the stable handle and
//! never changes when a profile is renamed. Editing the mods inside a
//! profile is not done here; that is `mods` (load, edit, then `save_profile`).

use crate::manifest::{Profile, ProfileInfo};
use crate::slug;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

const PROFILE_EXTENSION: &str = "json";

// --- Slugs ---------------------------------------------------------------

/// Picks a slug based on `base_slug` that no existing profile file uses.
fn unique_slug(profiles_dir: &Path, base_slug: &str) -> String {
    slug::unique(base_slug, |candidate| {
        slug_path(profiles_dir, candidate).is_file()
    })
}

/// Rejects anything that could name a file outside `profiles_dir`. Slugs
/// come from the UI, so they are never trusted.
fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() || slug.starts_with('.') || slug.contains(['/', '\\', '\0']) {
        bail!("invalid profile slug '{slug}'");
    }
    Ok(())
}

fn slug_path(profiles_dir: &Path, slug: &str) -> PathBuf {
    profiles_dir.join(format!("{slug}.{PROFILE_EXTENSION}"))
}

/// Path of an existing or future profile file, for a validated slug.
pub fn profile_path(profiles_dir: &Path, slug: &str) -> Result<PathBuf> {
    validate_slug(slug)?;
    Ok(slug_path(profiles_dir, slug))
}

fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("profile name cannot be empty");
    }
    Ok(name.to_string())
}

// --- Reading -------------------------------------------------------------

pub fn load_profile(path: &Path) -> Result<Profile> {
    let text =
        fs::read_to_string(path).with_context(|| format!("reading profile {}", path.display()))?;
    let profile: Profile = serde_json::from_str(&text)
        .with_context(|| format!("parsing profile {}", path.display()))?;
    Ok(profile)
}

/// Looks up a profile by its on-disk slug (the CLI/UI's stable handle),
/// e.g. `find_profile_by_slug(dir, "default_profile")` reads
/// `dir/default_profile.json`.
pub fn find_profile_by_slug(profiles_dir: &Path, slug: &str) -> Result<Option<Profile>> {
    let path = profile_path(profiles_dir, slug)?;
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(load_profile(&path)?))
}

/// Like `find_profile_by_slug`, but a missing profile is an error.
fn require_profile(profiles_dir: &Path, slug: &str) -> Result<Profile> {
    find_profile_by_slug(profiles_dir, slug)?
        .with_context(|| format!("profile '{slug}' not found in {}", profiles_dir.display()))
}

/// Every profile in `profiles_dir` with its slug, sorted by name. A missing
/// directory has no profiles.
pub fn load_all(profiles_dir: &Path) -> Result<Vec<(String, Profile)>> {
    let mut profiles = Vec::new();
    if !profiles_dir.is_dir() {
        return Ok(profiles);
    }
    for entry in fs::read_dir(profiles_dir)
        .with_context(|| format!("reading profiles directory {}", profiles_dir.display()))?
    {
        let entry = entry.with_context(|| {
            format!(
                "reading entry in profiles directory {}",
                profiles_dir.display()
            )
        })?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some(PROFILE_EXTENSION) {
            let slug = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            profiles.push((slug, load_profile(&path)?));
        }
    }
    profiles.sort_by_cached_key(|(slug, p)| (p.name.to_lowercase(), slug.clone()));
    Ok(profiles)
}

/// Lists every profile in `profiles_dir`, sorted by name. Reads each file to
/// get its canonical `name` - cheap at the scale of a handful of profile
/// files, and the only way to show accurate names without assuming the
/// filename matches them.
pub fn list_profiles(profiles_dir: &Path) -> Result<Vec<ProfileInfo>> {
    Ok(load_all(profiles_dir)?
        .into_iter()
        .map(|(slug, profile)| ProfileInfo {
            slug,
            name: profile.name,
        })
        .collect())
}

// --- Writing -------------------------------------------------------------

/// Writes `profile` to `<slug>.json` atomically (temp file in the same
/// directory, then rename) so a crash never leaves a half-written profile.
/// Creates the profile if it does not exist yet.
pub fn save_profile(profiles_dir: &Path, slug: &str, profile: &Profile) -> Result<()> {
    let path = profile_path(profiles_dir, slug)?;
    fs::create_dir_all(profiles_dir)
        .with_context(|| format!("creating {}", profiles_dir.display()))?;
    let tmp = profiles_dir.join(format!(".{slug}.{PROFILE_EXTENSION}.tmp"));
    fs::write(&tmp, serde_json::to_string_pretty(profile)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("saving {}", path.display()))
}

/// Writes `profile` under a fresh slug derived from `name`.
fn save_as_new(profiles_dir: &Path, name: &str, profile: &Profile) -> Result<(String, PathBuf)> {
    let slug = unique_slug(profiles_dir, &slug::slugify(name));
    save_profile(profiles_dir, &slug, profile)?;
    let path = slug_path(profiles_dir, &slug);
    Ok((slug, path))
}

/// Creates a new, empty profile named `name`, returning its slug (the
/// handle to pass to deploy/launch/run) and the path it was written to.
pub fn create_profile(profiles_dir: &Path, name: &str) -> Result<(String, PathBuf)> {
    let name = validate_name(name)?;
    let profile = Profile {
        name: name.clone(),
        mods: Vec::new(),
    };
    save_as_new(profiles_dir, &name, &profile)
}

/// Changes a profile's display name. The slug (and so the file) is unchanged.
pub fn rename_profile(profiles_dir: &Path, slug: &str, new_name: &str) -> Result<()> {
    let name = validate_name(new_name)?;
    let mut profile = require_profile(profiles_dir, slug)?;
    profile.name = name;
    save_profile(profiles_dir, slug, &profile)
}

/// Copies a profile's mod list under a new name, returning the new slug and
/// path. Mod folders in staging are shared, not copied.
pub fn duplicate_profile(
    profiles_dir: &Path,
    slug: &str,
    new_name: &str,
) -> Result<(String, PathBuf)> {
    let name = validate_name(new_name)?;
    let mut profile = require_profile(profiles_dir, slug)?;
    profile.name = name.clone();
    save_as_new(profiles_dir, &name, &profile)
}

/// Deletes the profile file only; staged mod files are untouched. Callers
/// that know about deployment should check whether it is currently deployed.
pub fn delete_profile(profiles_dir: &Path, slug: &str) -> Result<()> {
    let path = profile_path(profiles_dir, slug)?;
    if !path.is_file() {
        bail!("profile '{slug}' not found in {}", profiles_dir.display());
    }
    fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ProfileMod;
    use tempfile::TempDir;

    fn repo_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    fn a_mod(name: &str) -> ProfileMod {
        ProfileMod {
            name: name.into(),
            active: true,
        }
    }

    // --- fixtures in tests/profiles ---

    #[test]
    fn loads_profile() {
        let profile = load_profile(&repo_path("tests/profiles/test_profile.json")).unwrap();
        assert_eq!(profile.name, "Test Profile");
        assert_eq!(profile.mods.len(), 4);
    }

    #[test]
    fn load_profile_reports_missing_file() {
        let err = load_profile(&repo_path("tests/does_not_exist.json")).unwrap_err();
        assert!(err.to_string().contains("reading profile"));
    }

    #[test]
    fn lists_profiles() {
        let profiles = list_profiles(&repo_path("tests/profiles")).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].slug, "test_profile");
        assert_eq!(profiles[0].name, "Test Profile");
    }

    #[test]
    fn lists_no_profiles_for_missing_dir() {
        let profiles = list_profiles(&repo_path("tests/no_such_dir")).unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn finds_profile_by_slug() {
        let profile = find_profile_by_slug(&repo_path("tests/profiles"), "test_profile")
            .unwrap()
            .expect("test_profile should exist");
        assert_eq!(profile.name, "Test Profile");
    }

    #[test]
    fn find_profile_by_slug_returns_none_for_unknown_slug() {
        let profile =
            find_profile_by_slug(&repo_path("tests/profiles"), "no_such_profile").unwrap();
        assert!(profile.is_none());
    }

    // --- slugs ---

    #[test]
    fn unique_slug_appends_number() {
        let unique = unique_slug(&repo_path("tests/profiles"), "test_profile");
        assert_eq!(unique, "test_profile_2");
    }

    #[test]
    fn unique_slug_returns_base_if_available() {
        let unique = unique_slug(&repo_path("tests/profiles"), "new_profile");
        assert_eq!(unique, "new_profile");
    }

    #[test]
    fn rejects_slugs_that_could_escape_the_profiles_dir() {
        let dir = TempDir::new().unwrap();
        for bad in ["", "..", ".hidden", "a/b", "../x", "a\\b"] {
            assert!(profile_path(dir.path(), bad).is_err(), "{bad:?}");
            assert!(find_profile_by_slug(dir.path(), bad).is_err(), "{bad:?}");
            assert!(delete_profile(dir.path(), bad).is_err(), "{bad:?}");
        }
    }

    // --- writing (temp dir) ---

    #[test]
    fn creates_an_empty_profile() {
        let dir = TempDir::new().unwrap();
        let profiles_dir = dir.path().join("profiles");

        let (slug, path) = create_profile(&profiles_dir, "  My Profile ").unwrap();

        assert_eq!(slug, "my_profile");
        assert_eq!(path, profiles_dir.join("my_profile.json"));
        let profile = load_profile(&path).unwrap();
        assert_eq!(profile.name, "My Profile");
        assert!(profile.mods.is_empty());
    }

    #[test]
    fn create_gives_colliding_names_distinct_slugs() {
        let dir = TempDir::new().unwrap();
        let (a, _) = create_profile(dir.path(), "Same").unwrap();
        let (b, _) = create_profile(dir.path(), "Same").unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("same", "same_2"));
    }

    #[test]
    fn rejects_blank_names() {
        let dir = TempDir::new().unwrap();
        assert!(create_profile(dir.path(), "   ").is_err());
        let (slug, _) = create_profile(dir.path(), "Real").unwrap();
        assert!(rename_profile(dir.path(), &slug, "").is_err());
        assert!(duplicate_profile(dir.path(), &slug, " ").is_err());
    }

    #[test]
    fn save_round_trips_and_leaves_no_temp_file() {
        let dir = TempDir::new().unwrap();
        let (slug, _) = create_profile(dir.path(), "P").unwrap();
        let mut profile = find_profile_by_slug(dir.path(), &slug).unwrap().unwrap();
        profile.mods = vec![a_mod("One"), a_mod("Two")];

        save_profile(dir.path(), &slug, &profile).unwrap();

        let loaded = find_profile_by_slug(dir.path(), &slug).unwrap().unwrap();
        let names: Vec<_> = loaded.mods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["One", "Two"]);
        let files: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(files.len(), 1, "only the profile file remains");
    }

    #[test]
    fn rename_changes_the_name_but_not_the_slug() {
        let dir = TempDir::new().unwrap();
        let (slug, _) = create_profile(dir.path(), "Old").unwrap();
        let mut profile = find_profile_by_slug(dir.path(), &slug).unwrap().unwrap();
        profile.mods = vec![a_mod("Keep")];
        save_profile(dir.path(), &slug, &profile).unwrap();

        rename_profile(dir.path(), &slug, "New Name").unwrap();

        let loaded = find_profile_by_slug(dir.path(), &slug).unwrap().unwrap();
        assert_eq!(loaded.name, "New Name");
        assert_eq!(loaded.mods.len(), 1, "mods are kept");
        assert_eq!(list_profiles(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn rename_unknown_profile_is_an_error() {
        let dir = TempDir::new().unwrap();
        let err = rename_profile(dir.path(), "ghost", "X").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn duplicate_copies_mods_under_a_new_slug() {
        let dir = TempDir::new().unwrap();
        let (slug, _) = create_profile(dir.path(), "Original").unwrap();
        let mut profile = find_profile_by_slug(dir.path(), &slug).unwrap().unwrap();
        profile.mods = vec![a_mod("One")];
        save_profile(dir.path(), &slug, &profile).unwrap();

        let (copy_slug, _) = duplicate_profile(dir.path(), &slug, "Copy").unwrap();

        assert_eq!(copy_slug, "copy");
        let copy = find_profile_by_slug(dir.path(), &copy_slug)
            .unwrap()
            .unwrap();
        assert_eq!(copy.name, "Copy");
        assert_eq!(copy.mods.len(), 1);
        let original = find_profile_by_slug(dir.path(), &slug).unwrap().unwrap();
        assert_eq!(original.name, "Original", "source is untouched");
    }

    #[test]
    fn delete_removes_only_that_profile() {
        let dir = TempDir::new().unwrap();
        let (a, _) = create_profile(dir.path(), "A").unwrap();
        let (b, _) = create_profile(dir.path(), "B").unwrap();

        delete_profile(dir.path(), &a).unwrap();

        assert!(find_profile_by_slug(dir.path(), &a).unwrap().is_none());
        assert!(find_profile_by_slug(dir.path(), &b).unwrap().is_some());
        assert!(delete_profile(dir.path(), &a).is_err(), "already gone");
    }

    #[test]
    fn lists_profiles_sorted_by_name() {
        let dir = TempDir::new().unwrap();
        for name in ["zebra", "Apple", "mango"] {
            create_profile(dir.path(), name).unwrap();
        }
        let names: Vec<_> = list_profiles(dir.path())
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["Apple", "mango", "zebra"]);
    }
}
