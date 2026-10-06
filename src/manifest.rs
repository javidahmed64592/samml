use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRule {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModManifest {
    pub name: String,
    pub mod_path: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub files: Option<Vec<FileRule>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppManifest {
    pub game_dir: PathBuf,
    pub staging_dir: PathBuf,
    pub profiles_dir: PathBuf,
    #[serde(default)]
    pub steam_app_id: Option<u32>,
}

/// `name` is the only source of truth for display - free text, any
/// characters. The file it's stored under is a separate, filesystem-safe
/// slug (see `slugify`/`create_profile`) and is never assumed to match
/// `name` exactly, so renaming a profile later never requires a file
/// rename.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub manifests: Vec<ModManifest>,
}

/// A profile's on-disk identity (its filename stem, the slug) paired with
/// its canonical display name read from inside the file. Returned by
/// `list_profiles` so a UI can show `name` while using `slug` as the
/// stable handle for lookups.
#[derive(Debug, Clone)]
pub struct ProfileInfo {
    pub slug: String,
    pub name: String,
}

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
