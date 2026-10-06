use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub manifests: Vec<ModManifest>,
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

pub fn list_profiles(profiles_dir: &Path) -> Result<Vec<String>> {
    let mut profiles = Vec::new();
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
            profiles.push(
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string(),
            );
        }
    }
    Ok(profiles)
}

pub fn check_profile_exists(profiles: &Vec<String>, profile_name: &str) -> bool {
    profiles.iter().any(|p| p == profile_name)
}
