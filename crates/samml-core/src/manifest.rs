use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// App manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppManifest {
    pub game_dir: PathBuf,
    pub staging_dir: PathBuf,
    pub profiles_dir: PathBuf,
    #[serde(default)]
    pub steam_app_id: Option<u32>,
}

// Mod manifests
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

// Mod profiles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub manifests: Vec<ModManifest>,
}

#[derive(Debug, Clone)]
pub struct ProfileInfo {
    pub slug: String,
    pub name: String,
}
