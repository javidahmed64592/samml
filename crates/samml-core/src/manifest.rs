//! Plain data types shared by core, the CLI and (via serde) the GUI.
//! No filesystem access or behaviour lives here.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

// Mod library: every folder in STAGING_DIR is a mod, named after its folder.
// Settings are only stored for mods that need something other than the
// default of deploying the whole folder to `modloader/<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRule {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModSettings {
    /// Where the mod's files go. `None` deploys the whole mod folder to
    /// `modloader/<name>`.
    #[serde(default)]
    pub files: Option<Vec<FileRule>>,
}

/// The contents of `STAGING_DIR/.samml_library.json`, keyed by mod name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub mods: BTreeMap<String, ModSettings>,
}

/// A mod as the UI sees it: a folder in staging joined with its settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryMod {
    /// The folder name, which is also the name shown in game.
    pub name: String,
    pub files: Option<Vec<FileRule>>,
    /// Why the folder cannot be used as a mod yet (e.g. its name is not
    /// valid), or `None` if it is fine.
    pub problem: Option<String>,
}

// Mod profiles: a choice of mods from the library, in deploy order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMod {
    /// The library mod's name.
    pub name: String,
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub mods: Vec<ProfileMod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub slug: String,
    pub name: String,
}

/// What deploy needs to know about one mod: a profile entry joined with the
/// library. Only active mods are resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployMod {
    pub name: String,
    pub files: Option<Vec<FileRule>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_MANIFEST: &str = include_str!("../../../tests/app_manifest.json");
    const TEST_PROFILE: &str = include_str!("../../../tests/profiles/test_profile.json");
    const TEST_LIBRARY: &str = include_str!("../../../tests/library/.samml_library.json");

    #[test]
    fn parses_app_manifest() {
        let manifest: AppManifest = serde_json::from_str(APP_MANIFEST).unwrap();
        assert!(!manifest.game_dir.as_os_str().is_empty());
        assert!(!manifest.staging_dir.as_os_str().is_empty());
        assert!(!manifest.profiles_dir.as_os_str().is_empty());
        assert_eq!(manifest.steam_app_id, Some(12120));
    }

    #[test]
    fn parses_test_profile() {
        let profile: Profile = serde_json::from_str(TEST_PROFILE).unwrap();
        assert_eq!(profile.name, "Test Profile");
        let names: Vec<_> = profile.mods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            ["CLEO 5.4.0", "Modloader", "Test Mod 1", "Test Mod 2"]
        );
        assert!(profile.mods.iter().all(|m| m.active));
    }

    #[test]
    fn mods_without_settings_use_the_default_deployment() {
        let library: Library = serde_json::from_str(TEST_LIBRARY).unwrap();
        assert!(!library.mods.contains_key("Test Mod 1"));
    }

    #[test]
    fn parses_trailing_slash_rules() {
        let library: Library = serde_json::from_str(TEST_LIBRARY).unwrap();
        let files = library.mods["Test Mod 2"].files.as_ref().unwrap();

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
