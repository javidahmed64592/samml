//! Loading the app manifest (`app_manifest.json`).
//!
//! Owns reading the app config from disk. Profiles live in `profiles`.

use crate::manifest::AppManifest;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub fn load_app_manifest(path: &Path) -> Result<AppManifest> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading app manifest {}", path.display()))?;
    let manifest: AppManifest = serde_json::from_str(&text)
        .with_context(|| format!("parsing app manifest {}", path.display()))?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    #[test]
    fn loads_app_manifest() {
        let manifest = load_app_manifest(&repo_path("tests/app_manifest.json")).unwrap();
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
}
