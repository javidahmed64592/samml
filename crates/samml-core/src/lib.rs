use crate::manifest::{AppManifest, Profile, ProfileInfo};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod deploy;
pub mod launch;
pub mod manifest;

pub fn load_app_manifest(path: &Path) -> Result<AppManifest> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading app manifest {}", path.display()))?;
    let manifest: AppManifest = serde_json::from_str(&text)
        .with_context(|| format!("parsing app manifest {}", path.display()))?;
    Ok(manifest)
}

pub fn load_profile(path: &Path) -> Result<Profile> {
    let text =
        fs::read_to_string(path).with_context(|| format!("reading profile {}", path.display()))?;
    let profile: Profile = serde_json::from_str(&text)
        .with_context(|| format!("parsing profile {}", path.display()))?;
    Ok(profile)
}

/// Lists every profile in `profiles_dir`. Reads each file to get its
/// canonical `name` - cheap at the scale of a handful of profile files,
/// and the only way to show accurate names without assuming the filename
/// matches them.
pub fn list_profiles(profiles_dir: &Path) -> Result<Vec<ProfileInfo>> {
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
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            let slug = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let profile = load_profile(&path)?;
            profiles.push(ProfileInfo {
                slug,
                name: profile.name,
            });
        }
    }
    Ok(profiles)
}

/// Lowercase, hyphenated, filesystem-safe handle derived from a display
/// name. Falls back to a timestamp-based slug if nothing alphanumeric
/// survives (e.g. a name made entirely of emoji/punctuation).
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_hyphen = false;
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if (c.is_whitespace() || c == '-' || c == '_') && !last_was_hyphen {
            slug.push('-');
            last_was_hyphen = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("profile-{ts}")
    } else {
        slug
    }
}

/// Appends -2, -3, ... to `base_slug` until it no longer collides with an
/// existing profile file.
fn unique_slug(profiles_dir: &Path, base_slug: &str) -> String {
    let mut candidate = base_slug.to_string();
    let mut n = 2;
    while profiles_dir.join(format!("{candidate}.json")).is_file() {
        candidate = format!("{base_slug}-{n}");
        n += 1;
    }
    candidate
}

/// Looks up a profile by its on-disk slug (the CLI/UI's stable handle),
/// e.g. `find_profile_by_slug(dir, "default-profile")` reads
/// `dir/default-profile.json`.
pub fn find_profile_by_slug(profiles_dir: &Path, slug: &str) -> Result<Option<Profile>> {
    let path = profiles_dir.join(format!("{slug}.json"));
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(load_profile(&path)?))
}

/// Creates a new, empty profile named `name`, returning its slug (the
/// handle to pass to deploy/launch/run) and the path it was written to.
pub fn create_profile(profiles_dir: &Path, name: &str) -> Result<(String, PathBuf)> {
    fs::create_dir_all(profiles_dir)
        .with_context(|| format!("creating {}", profiles_dir.display()))?;
    let slug = unique_slug(profiles_dir, &slugify(name));
    let path = profiles_dir.join(format!("{slug}.json"));
    let profile = Profile {
        name: name.to_string(),
        manifests: Vec::new(),
    };
    fs::write(&path, serde_json::to_string_pretty(&profile)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok((slug, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    #[test]
    fn loads_app_manifest() {
        let manifest = load_app_manifest(&repo_path("app_manifest.json")).unwrap();
        assert!(!manifest.game_dir.as_os_str().is_empty());
        assert!(!manifest.staging_dir.as_os_str().is_empty());
        assert!(!manifest.profiles_dir.as_os_str().is_empty());
        assert_eq!(manifest.steam_app_id, Some(12120));
    }

    #[test]
    fn load_app_manifest_reports_missing_file() {
        let err = load_app_manifest(&repo_path("does_not_exist.json")).unwrap_err();
        assert!(err.to_string().contains("reading app manifest"));
    }

    #[test]
    fn loads_profile() {
        let profile = load_profile(&repo_path("tests/profiles/test_profile.json")).unwrap();
        assert_eq!(profile.name, "Test Profile");
        assert_eq!(profile.manifests.len(), 4);
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
}
