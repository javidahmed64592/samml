//! The app config: loading, checking and saving `AppConfig`, and working
//! out where the player stands on first run.
//!
//! Owns reading and writing the config file and deciding whether its paths
//! are usable. Where the file lives and where the game is come from
//! `platform`. The CLI and the UI use `config_status` to decide between
//! "show the setup screen" and "carry on".

use crate::manifest::{AppConfig, ConfigField, ConfigProblem, ConfigStatus, ConfigSuggestion};
use crate::platform::{self, GTA_SA_STEAM_APP_ID, PlatformDirs};
use anyhow::{Context, Result, bail};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Folder names inside the SAMML folder.
const STAGING_DIR_NAME: &str = "staging";
const PROFILES_DIR_NAME: &str = "profiles";

impl AppConfig {
    /// The mod library: one folder per mod.
    pub fn staging_dir(&self) -> PathBuf {
        self.samml_dir.join(STAGING_DIR_NAME)
    }

    /// The profile files.
    pub fn profiles_dir(&self) -> PathBuf {
        self.samml_dir.join(PROFILES_DIR_NAME)
    }
}

pub fn load_config(path: &Path) -> Result<AppConfig> {
    let text =
        fs::read_to_string(path).with_context(|| format!("reading config {}", path.display()))?;
    let manifest: AppConfig = serde_json::from_str(&text)
        .with_context(|| format!("parsing config {}", path.display()))?;
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

/// The SAMML folder is created on demand: it may be missing, but if it is
/// there it must be a folder.
fn check_samml_dir(dir: &Path, problems: &mut Vec<ConfigProblem>) {
    let field = ConfigField::SammlDir;
    if !dir.is_absolute() {
        problems.push(problem(field, "must be an absolute path"));
    } else if dir.exists() && !dir.is_dir() {
        problems.push(problem(
            field,
            format!("{} exists but is not a folder", dir.display()),
        ));
    }
}

/// The SAMML folder and the game folder must not overlap: deploy writes
/// into the game folder, and the library is a folder of mods.
fn check_overlap(config: &AppConfig, problems: &mut Vec<ConfigProblem>) {
    let (game, samml) = (&config.game_dir, &config.samml_dir);
    if !(game.is_absolute() && samml.is_absolute()) {
        return;
    }
    if samml.starts_with(game) {
        problems.push(problem(
            ConfigField::SammlDir,
            "must not be inside the game folder",
        ));
    } else if game.starts_with(samml) {
        problems.push(problem(
            ConfigField::SammlDir,
            "must not contain the game folder",
        ));
    }
}

/// Everything wrong with the config's paths; empty if it is usable. Does
/// not change anything on disk.
pub fn check_config(config: &AppConfig) -> Vec<ConfigProblem> {
    let mut problems = Vec::new();
    check_game_dir(&config.game_dir, &mut problems);
    check_samml_dir(&config.samml_dir, &mut problems);
    check_overlap(config, &mut problems);
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
    let config: AppConfig = match serde_json::from_str(&text) {
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
pub fn save_config(path: &Path, config: &AppConfig) -> Result<()> {
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

/// Creates the SAMML folder with its staging and profiles folders if they
/// do not exist yet.
pub fn ensure_dirs(config: &AppConfig) -> Result<()> {
    for dir in [config.staging_dir(), config.profiles_dir()] {
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    Ok(())
}

// --- First run -----------------------------------------------------------

/// Starting values for the first-run screen: the game if it can be found,
/// and the standard SAMML folder.
pub fn suggest_config(dirs: &PlatformDirs) -> ConfigSuggestion {
    let found_game_dirs = platform::find_game_dirs(dirs);
    ConfigSuggestion {
        game_dir: found_game_dirs.first().cloned(),
        found_game_dirs,
        samml_dir: dirs.default_samml_dir(),
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
        config: AppConfig,
    }

    impl Env {
        /// A fake game folder with a SAMML folder beside it.
        fn new() -> Self {
            let root = TempDir::new().unwrap();
            let game = root.path().join("Game");
            fs::create_dir_all(&game).unwrap();
            fs::write(game.join("gta-sa.exe"), "").unwrap();
            let config = AppConfig {
                game_dir: game,
                samml_dir: root.path().join("samml"),
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
    fn loads_config() {
        let manifest = load_config(&repo_path("tests/config.json")).unwrap();
        assert!(!manifest.game_dir.as_os_str().is_empty());
        assert_eq!(manifest.steam_app_id, Some(12120));
    }

    #[test]
    fn load_config_reports_missing_file() {
        let err = load_config(&repo_path("does_not_exist.json")).unwrap_err();
        assert!(err.to_string().contains("reading config"));
    }

    // --- checking ---

    #[test]
    fn a_good_config_has_no_problems_even_if_the_samml_folder_does_not_exist_yet() {
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
            problems[0].1.contains("gta-sa.exe not found"),
            "{problems:?}"
        );
    }

    #[test]
    fn the_samml_folder_must_be_an_absolute_folder() {
        let mut env = Env::new();
        let file = env.root.path().join("a_file");
        fs::write(&file, "").unwrap();

        env.config.samml_dir = PathBuf::from("samml");
        let problems = env.problems();
        assert_eq!(problems[0].0, ConfigField::SammlDir);
        assert!(problems[0].1.contains("absolute"));

        env.config.samml_dir = file;
        assert!(env.problems()[0].1.contains("not a folder"));
    }

    #[test]
    fn the_samml_and_game_folders_must_not_overlap() {
        let mut env = Env::new();
        let root = env.root.path().to_path_buf();

        env.config.samml_dir = root.join("Game/samml");
        assert_eq!(
            env.problems(),
            [(
                ConfigField::SammlDir,
                "must not be inside the game folder".into()
            )]
        );

        env.config.samml_dir = env.config.game_dir.clone();
        assert!(env.problems()[0].1.contains("inside the game folder"));

        env.config.samml_dir = root;
        assert!(
            env.problems()
                .iter()
                .any(|(_, m)| m.contains("must not contain the game folder"))
        );
    }

    #[test]
    fn staging_and_profiles_are_inside_the_samml_folder() {
        let env = Env::new();
        let samml = &env.config.samml_dir;
        assert_eq!(env.config.staging_dir(), samml.join("staging"));
        assert_eq!(env.config.profiles_dir(), samml.join("profiles"));
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
        assert_eq!(load_config(&file).unwrap().samml_dir, env.config.samml_dir);
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
    fn ensure_dirs_creates_the_samml_folder_with_staging_and_profiles() {
        let env = Env::new();
        ensure_dirs(&env.config).unwrap();
        ensure_dirs(&env.config).unwrap(); // harmless the second time
        assert!(env.config.staging_dir().is_dir() && env.config.profiles_dir().is_dir());
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
        fs::write(game.join("gta-sa.exe"), "").unwrap();
        let dirs = PlatformDirs::new(home.path().to_path_buf(), None, None);

        let suggestion = suggest_config(&dirs);

        assert_eq!(suggestion.game_dir.as_deref(), Some(game.as_path()));
        assert_eq!(suggestion.found_game_dirs, [game]);
        assert_eq!(suggestion.samml_dir, home.path().join(".local/share/samml"));
        assert_eq!(suggestion.steam_app_id, 12120);
    }

    #[test]
    fn a_suggestion_for_a_machine_without_the_game_has_no_game_dir() {
        let home = TempDir::new().unwrap();
        let dirs = PlatformDirs::new(home.path().to_path_buf(), None, None);
        assert_eq!(suggest_config(&dirs).game_dir, None);
    }
}
