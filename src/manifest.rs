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
    #[serde(default)]
    pub steam_app_id: Option<u32>,
    pub manifests: Vec<ModManifest>,
}

pub fn load_app_manifest(path: &Path) -> Result<AppManifest> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading app manifest {}", path.display()))?;
    let manifest: AppManifest = serde_json::from_str(&text)
        .with_context(|| format!("parsing app manifest {}", path.display()))?;
    Ok(manifest)
}
