//! Plain data types shared by core, the CLI and (via serde) the GUI.
//! No filesystem access or behaviour lives here.

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub slug: String,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_MANIFEST: &str = include_str!("../../../tests/app_manifest.json");
    const TEST_PROFILE: &str = include_str!("../../../tests/profiles/test_profile.json");

    fn load_app_manifest() -> AppManifest {
        serde_json::from_str(APP_MANIFEST).expect("app manifest should parse")
    }

    fn load_profile() -> Profile {
        serde_json::from_str(TEST_PROFILE).expect("test profile should parse")
    }

    #[test]
    fn parses_app_manifest() {
        let manifest = load_app_manifest();
        assert!(!manifest.game_dir.as_os_str().is_empty());
        assert!(!manifest.staging_dir.as_os_str().is_empty());
        assert!(!manifest.profiles_dir.as_os_str().is_empty());
        assert_eq!(manifest.steam_app_id, Some(12120));
    }

    #[test]
    fn parses_test_profile() {
        let profile = load_profile();
        assert_eq!(profile.name, "Test Profile");
        assert_eq!(profile.manifests.len(), 4);
        assert!(profile.manifests.iter().all(|m| m.active));

        // Mods without a `files` array deploy to the modloader directory by default
        assert!(profile.manifests[2].files.is_none());
    }

    #[test]
    fn parses_trailing_slash_rules() {
        let profile = load_profile();
        let mod2 = profile
            .manifests
            .iter()
            .find(|m| m.name == "Test Mod 2")
            .expect("Test Mod 2 should exist");
        let files = mod2.files.as_ref().expect("Test Mod 2 should have files");

        let rule = |source: &str| {
            files
                .iter()
                .find(|f| f.source == source)
                .unwrap_or_else(|| panic!("missing rule for {source}"))
        };

        // Trailing slashes must survive parsing untouched
        assert_eq!(rule("contents_dir/").target, "merged_dir/");
        assert_eq!(rule("contents_dir_b/").target, "merged_dir_b");
        assert_eq!(rule("whole_dir").target, "container_dir/");
        assert_eq!(rule("whole_dir_b").target, "renamed_dir");
        assert_eq!(rule("file.txt").target, "container_dir/");
        assert_eq!(rule("file_b.txt").target, "renamed_file.txt");
    }
}
