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
// Each mod has a stable id; profiles refer to mods by id, so renaming a mod
// (and its folder) only changes the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRule {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModSettings {
    /// The mod's name: its folder in staging and its `modloader/<name>`
    /// folder, so what shows in game.
    pub name: String,
    /// Where the mod's files go. `None` deploys the whole mod folder to
    /// `modloader/<name>`.
    #[serde(default)]
    pub files: Option<Vec<FileRule>>,
}

/// The contents of `STAGING_DIR/.samml_library.json`, keyed by mod id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub mods: BTreeMap<String, ModSettings>,
}

/// A mod as the UI sees it: a library entry joined with its folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryMod {
    pub id: String,
    pub name: String,
    pub files: Option<Vec<FileRule>>,
    /// Why the mod cannot be used yet (e.g. its folder is missing or its
    /// name is not valid), or `None` if it is fine.
    pub problem: Option<String>,
}

// Mod profiles: a choice of mods from the library, in deploy order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMod {
    /// The library mod's id.
    pub id: String,
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

/// A profile entry as the UI sees it: the entry joined with the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileModView {
    pub id: String,
    /// `None` if the id is not in the library at all.
    pub name: Option<String>,
    pub active: bool,
    /// Why the mod cannot be deployed (not in the library, folder missing,
    /// ...), or `None` if it is fine.
    pub problem: Option<String>,
}

/// What deploy needs to know about one mod: a profile entry joined with the
/// library. Only active mods are resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployMod {
    pub name: String,
    pub files: Option<Vec<FileRule>>,
}

// Importing files into a mod, and browsing what is in one.

/// Where imported sources end up inside the mod's folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Placement {
    /// Each source (file or folder) is placed inside the mod folder under
    /// its own name.
    Items,
    /// A single source folder's contents are merged into the mod folder, so
    /// the chosen folder becomes the mod's root.
    Contents,
}

/// What to do when an imported file already exists in the mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictPolicy {
    /// Refuse to import anything.
    Fail,
    /// Replace the existing files.
    Overwrite,
}

/// What an import would do, worked out without changing anything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportPreview {
    pub files: usize,
    pub bytes: u64,
    /// Existing files (relative to the mod folder) that would be replaced.
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportProgress {
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub files_copied: usize,
    pub bytes_copied: u64,
    /// Existing files (relative to the mod folder) that were replaced.
    pub overwritten: Vec<String>,
}

/// One item in a mod folder, for the rule picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    /// Size in bytes; `None` for folders.
    pub size: Option<u64>,
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
        let ids: Vec<_> = profile.mods.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "cleo_5_4_0-4f2a",
                "modloader-91c3",
                "test_mod_1-77de",
                "test_mod_2-0b5e"
            ]
        );
        assert!(profile.mods.iter().all(|m| m.active));
    }

    #[test]
    fn mods_without_rules_use_the_default_deployment() {
        let library: Library = serde_json::from_str(TEST_LIBRARY).unwrap();
        let plain = &library.mods["test_mod_1-77de"];
        assert_eq!(plain.name, "Test Mod 1");
        assert!(plain.files.is_none());
    }

    #[test]
    fn parses_trailing_slash_rules() {
        let library: Library = serde_json::from_str(TEST_LIBRARY).unwrap();
        let files = library.mods["test_mod_2-0b5e"].files.as_ref().unwrap();

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
