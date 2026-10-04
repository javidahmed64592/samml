use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// One (source -> target) rule inside a mod's `files` array.
///
/// Trailing-slash convention (mirrors `rsync`):
/// - `source` ends with '/'  => deploy the CONTENTS of that directory,
///   merging them into `target` (the directory itself is not recreated).
/// - `source` has no trailing slash => deploy the item itself (file or
///   whole directory) as a unit.
/// - `target` ends with '/'  => place the item INSIDE that directory,
///   keeping the source's own name.
/// - `target` has no trailing slash => the item is placed at that EXACT
///   path (i.e. may rename it).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRule {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModManifest {
    pub name: String,
    /// Path relative to STAGING_DIR.
    pub mod_path: String,
    #[serde(default)]
    pub active: bool,
    /// - Omitted (`null`/absent) => deploy the whole `mod_path` folder as a
    ///   unit to `${GAME_DIR}/modloader/${name}`.
    /// - Present (even `[]`)     => deploy ONLY the listed rules; every
    ///   other file/folder under `mod_path` is ignored.
    #[serde(default)]
    pub files: Option<Vec<FileRule>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppManifest {
    pub game_dir: PathBuf,
    pub staging_dir: PathBuf,
    /// Steam App ID for GTA: San Andreas, used by the `launch`/`run`
    /// commands (`steam -applaunch <id>`).
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
