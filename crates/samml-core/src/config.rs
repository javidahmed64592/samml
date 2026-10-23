//! The app config: loading, checking and saving `AppManifest`, and working
//! out where the player stands on first run.
//!
//! Owns reading and writing the config file and deciding whether its paths
//! are usable. Where the file lives and where the game is come from
//! `platform`. The CLI and the UI use `config_status` to decide between
//! "show the setup screen" and "carry on".

use crate::manifest::{AppManifest, ConfigField, ConfigProblem, ConfigStatus, ConfigSuggestion};
use crate::platform::{self, GTA_SA_STEAM_APP_ID, PlatformDirs};
use anyhow::{Context, Result, bail};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

pub fn load_app_manifest(path: &Path) -> Result<AppManifest> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading app manifest {}", path.display()))?;
    let manifest: AppManifest = serde_json::from_str(&text)
        .with_context(|| format!("parsing app manifest {}", path.display()))?;
    Ok(manifest)
}

// --- Checking ------------------------------------------------------------

fn problem(field: ConfigField, message: impl Into<String>) -> ConfigProblem {
    ConfigProblem {
        field,
        message: message.into(),
    }
}

fn check_game_dir(dir: &Path, problems: &mut Vec<ConfigProblem>) {
    let mut add = |message: String| problems.push(problem(ConfigField::GameDir, message));
    if !dir.is_absolute() {
        add("must be an absolute path".to_string());
    } else if !dir.exists() {
        add(format!("{} does not exist", dir.display()));
    } else if !dir.is_dir() {
        add(format!("{} is not a folder", dir.display()));
    } else if !platform::is_game_dir(dir) {
        add(format!(
            "{} does not look like a GTA San Andreas folder ({} not found)",
            dir.display(),
            platform::GAME_EXECUTABLE
        ));
    }
}

/// A folder SAMML will create if needed: it may be missing, but if it is
/// there it must be a folder.
fn check_data_dir(field: ConfigField, dir: &Path, problems: &mut Vec<ConfigProblem>) {
    if !dir.is_absolute() {
        problems.push(problem(field, "must be an absolute path"));
    } else if dir.exists() && !dir.is_dir() {
        problems.push(problem(
            field,
            format!("{} exists but is not a folder", dir.display()),
        ));
    }
}

/// Folders must not overlap: deploy writes into the game folder, and the
/// staging folder lists every folder in it as a mod.
fn check_overlaps(config: &AppManifest, problems: &mut Vec<ConfigProblem>) {
    let (game, staging, profiles) = (&config.game_dir, &config.staging_dir, &config.profiles_dir);
    if !(game.is_absolute() && staging.is_absolute() && profiles.is_absolute()) {
        return;
    }
    if staging == profiles {
        problems.push(problem(
            ConfigField::ProfilesDir,
            "must be a different folder from the staging folder",
        ));
    } else if profiles.starts_with(staging) {
        problems.push(problem(
            ConfigField::ProfilesDir,
            "must not be inside the staging folder (it would show up as a mod)",
        ));
    } else if staging.starts_with(profiles) {
        problems.push(problem(
            ConfigField::StagingDir,
            "must not be inside the profiles folder",
        ));
    }
    for (field, dir) in [
        (ConfigField::StagingDir, staging),
        (ConfigField::ProfilesDir, profiles),
    ] {
        if dir.starts_with(game) {
            problems.push(problem(field, "must not be inside the game folder"));
        } else if game.starts_with(dir) {
            problems.push(problem(field, "must not contain the game folder"));
        }
    }
}

/// Everything wrong with the config's paths; empty if it is usable. Does
/// not change anything on disk.
pub fn check_config(config: &AppManifest) -> Vec<ConfigProblem> {
    let mut problems = Vec::new();
    check_game_dir(&config.game_dir, &mut problems);
    check_data_dir(ConfigField::StagingDir, &config.staging_dir, &mut problems);
    check_data_dir(
        ConfigField::ProfilesDir,
        &config.profiles_dir,
        &mut problems,
    );
    check_overlaps(config, &mut problems);
    problems
}

/// Where the config file stands: missing (first run), unreadable, usable,
/// or readable with paths that need fixing.
pub fn config_status(path: &Path) -> ConfigStatus {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return ConfigStatus::Missing,
        Err(e) => {
            return ConfigStatus::Broken {
                reason: format!("could not read {}: {e}", path.display()),
            };
        }
    };
    let config: AppManifest = match serde_json::from_str(&text) {
        Ok(config) => config,
        Err(e) => {
            return ConfigStatus::Broken {
                reason: format!("{} is not a valid config file: {e}", path.display()),
            };
        }
    };
    let problems = check_config(&config);
    if problems.is_empty() {
        ConfigStatus::Ready { config }
    } else {
        ConfigStatus::NeedsAttention { config, problems }
    }
}

// --- Saving --------------------------------------------------------------

/// Saves the config (atomically), creating the file's folder if needed.
/// Refuses a config whose paths are not usable, so a saved config is always
/// one the app can start with.
pub fn save_config(path: &Path, config: &AppManifest) -> Result<()> {
    let problems = check_config(config);
    if !problems.is_empty() {
        let messages: Vec<_> = problems
            .iter()
            .map(|p| format!("{:?}: {}", p.field, p.message))
            .collect();
        bail!("the config is not usable: {}", messages.join("; "));
    }
    let dir = path.parent().context("the config path has no folder")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = dir.join(".config.json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(config)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("saving {}", path.display()))
}

/// Creates the staging and profiles folders if they do not exist yet.
pub fn ensure_dirs(config: &AppManifest) -> Result<()> {
    for dir in [&config.staging_dir, &config.profiles_dir] {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    Ok(())
}

// --- First run -----------------------------------------------------------

/// Starting values for the first-run screen: the game if it can be found,
/// and the standard folders for staging and profiles.
pub fn suggest_config(dirs: &PlatformDirs) -> ConfigSuggestion {
    let found_game_dirs = platform::find_game_dirs(dirs);
    ConfigSuggestion {
        game_dir: found_game_dirs.first().cloned(),
        found_game_dirs,
        staging_dir: dirs.default_staging_dir(),
        profiles_dir: dirs.default_profiles_dir(),
        steam_app_id: GTA_SA_STEAM_APP_ID,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn repo_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    struct Env {
        root: TempDir,
        config: AppManifest,
    }

    impl Env {
        /// A fake game folder plus staging and profiles folders beside it.
        fn new() -> Self {
            let root = TempDir::new().unwrap();
            let game = root.path().join("Game");
            fs::create_dir_all(&game).unwrap();
            fs::write(game.join("gta_sa.exe"), "").unwrap();
            let config = AppManifest {
                game_dir: game,
                staging_dir: root.path().join("staging"),
                profiles_dir: root.path().join("profiles"),
                steam_app_id: Some(12120),
            };
            Env { root, config }
        }

        fn problems(&self) -> Vec<(ConfigField, String)> {
            check_config(&self.config)
                .into_iter()
                .map(|p| (p.field, p.message))
                .collect()
        }

        fn config_file(&self) -> PathBuf {
            self.root.path().join("cfg/samml/config.json")
        }
    }

    // --- loading ---

    #[test]
    fn loads_app_manifest() {
        let manifest = load_app_manifest(&repo_path("tests/app_manifest.json")).unwrap();
        assert!(!manifest.game_dir.as_os_str().is_empty());
        assert_eq!(manifest.steam_app_id, Some(12120));
    }

    #[test]
    fn load_app_manifest_reports_missing_file() {
        let err = load_app_manifest(&repo_path("does_not_exist.json")).unwrap_err();
        assert!(err.to_string().contains("reading app manifest"));
    }

    // --- checking ---

    #[test]
    fn a_good_config_has_no_problems_even_if_staging_does_not_exist_yet() {
        let env = Env::new();
        assert!(env.problems().is_empty());
    }

    #[test]
    fn the_game_folder_must_exist_and_be_the_game() {
        let mut env = Env::new();

        env.config.game_dir = PathBuf::from("relative/game");
        assert!(env.problems()[0].1.contains("absolute"));

        env.config.game_dir = env.root.path().join("nope");
        assert!(env.problems()[0].1.contains("does not exist"));

        let file = env.root.path().join("a_file");
        fs::write(&file, "").unwrap();
        env.config.game_dir = file;
        assert!(env.problems()[0].1.contains("not a folder"));

        let empty = env.root.path().join("empty");
        fs::create_dir(&empty).unwrap();
        env.config.game_dir = empty;
        let problems = env.problems();
        assert_eq!(problems[0].0, ConfigField::GameDir);
        assert!(
            problems[0].1.contains("gta_sa.exe not found"),
            "{problems:?}"
        );
    }

    #[test]
    fn staging_and_profiles_must_be_absolute_folders() {
        let mut env = Env::new();
        let file = env.root.path().join("a_file");
        fs::write(&file, "").unwrap();
        env.config.staging_dir = PathBuf::from("staging");
        env.config.profiles_dir = file;

        let problems = env.problems();

        assert!(
            problems
                .iter()
                .any(|(f, m)| *f == ConfigField::StagingDir && m.contains("absolute"))
        );
        assert!(
            problems
                .iter()
                .any(|(f, m)| *f == ConfigField::ProfilesDir && m.contains("not a folder"))
        );
    }

    #[test]
    fn the_three_folders_must_not_overlap() {
        let mut env = Env::new();
        let root = env.root.path().to_path_buf();

        env.config.profiles_dir = env.config.staging_dir.clone();
        assert!(env.problems()[0].1.contains("different folder"));

        env.config.profiles_dir = root.join("staging/profiles");
        assert!(env.problems()[0].1.contains("inside the staging folder"));

        env.config.profiles_dir = root.join("profiles");
        env.config.staging_dir = root.join("profiles/staging");
        assert!(env.problems()[0].1.contains("inside the profiles folder"));

        env.config.staging_dir = root.join("Game/staging");
        let problems = env.problems();
        assert_eq!(
            problems[0],
            (
                ConfigField::StagingDir,
                "must not be inside the game folder".into()
            )
        );

        env.config.staging_dir = root.to_path_buf();
        assert!(
            env.problems()
                .iter()
                .any(|(_, m)| m.contains("must not contain the game folder"))
        );
    }

    // --- status ---

    #[test]
    fn status_distinguishes_missing_broken_attention_and_ready() {
        let env = Env::new();
        let file = env.config_file();
        assert_eq!(config_status(&file), ConfigStatus::Missing);

        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "{ not json").unwrap();
        let ConfigStatus::Broken { reason } = config_status(&file) else {
            panic!("expected Broken");
        };
        assert!(reason.contains("not a valid config file"), "{reason}");

        let mut moved = env.config.clone();
        moved.game_dir = env.root.path().join("moved_away");
        fs::write(&file, serde_json::to_string(&moved).unwrap()).unwrap();
        let ConfigStatus::NeedsAttention { config, problems } = config_status(&file) else {
            panic!("expected NeedsAttention");
        };
        assert_eq!(config.game_dir, moved.game_dir);
        assert_eq!(problems[0].field, ConfigField::GameDir);

        fs::write(&file, serde_json::to_string(&env.config).unwrap()).unwrap();
        assert!(matches!(config_status(&file), ConfigStatus::Ready { .. }));
    }

    // --- saving ---

    #[test]
    fn saving_creates_the_config_folder_and_round_trips() {
        let env = Env::new();
        let file = env.config_file();

        save_config(&file, &env.config).unwrap();

        assert!(matches!(config_status(&file), ConfigStatus::Ready { .. }));
        assert_eq!(
            load_app_manifest(&file).unwrap().staging_dir,
            env.config.staging_dir
        );
        let leftovers: Vec<_> = fs::read_dir(file.parent().unwrap()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "no temp file is left behind");
    }

    #[test]
    fn saving_an_unusable_config_is_refused_and_changes_nothing() {
        let mut env = Env::new();
        let file = env.config_file();
        save_config(&file, &env.config).unwrap();
        let before = fs::read_to_string(&file).unwrap();

        env.config.game_dir = env.root.path().join("nope");
        let err = save_config(&file, &env.config).unwrap_err();

        assert!(err.to_string().contains("GameDir"), "{err}");
        assert_eq!(fs::read_to_string(&file).unwrap(), before);
    }

    #[test]
    fn ensure_dirs_creates_staging_and_profiles() {
        let env = Env::new();
        ensure_dirs(&env.config).unwrap();
        ensure_dirs(&env.config).unwrap(); // harmless the second time
        assert!(env.config.staging_dir.is_dir() && env.config.profiles_dir.is_dir());
    }

    // --- first run ---

    #[test]
    fn suggestions_use_the_detected_game_and_the_standard_folders() {
        let home = TempDir::new().unwrap();
        let game = home
            .path()
            .join(".local/share/Steam/steamapps/common")
            .join(platform::GAME_FOLDER_NAME);
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("gta_sa.exe"), "").unwrap();
        let dirs = PlatformDirs::new(home.path().to_path_buf(), None, None);

        let suggestion = suggest_config(&dirs);

        assert_eq!(suggestion.game_dir.as_deref(), Some(game.as_path()));
        assert_eq!(suggestion.found_game_dirs, [game]);
        assert_eq!(
            suggestion.staging_dir,
            home.path().join(".local/share/samml/staging")
        );
        assert_eq!(suggestion.steam_app_id, 12120);
    }

    #[test]
    fn a_suggestion_for_a_machine_without_the_game_has_no_game_dir() {
        let home = TempDir::new().unwrap();
        let dirs = PlatformDirs::new(home.path().to_path_buf(), None, None);
        assert_eq!(suggest_config(&dirs).game_dir, None);
    }
}
