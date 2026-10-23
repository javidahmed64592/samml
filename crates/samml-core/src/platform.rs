//! Where things live on this Linux system, and finding the game.
//!
//! Owns the XDG locations SAMML uses (config file, default staging and
//! profiles folders) and detecting GTA: San Andreas in Steam libraries.
//! Pure path logic over a `PlatformDirs` value, so tests can use a fake
//! home directory. Checking and saving the config is `config`.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The folder name Steam gives the game.
pub const GAME_FOLDER_NAME: &str = "Grand Theft Auto San Andreas";

/// The executable that marks a folder as a GTA: San Andreas install.
pub const GAME_EXECUTABLE: &str = "gta_sa.exe";

/// Steam's app id for GTA: San Andreas.
pub const GTA_SA_STEAM_APP_ID: u32 = 12120;

const APP_DIR: &str = "samml";
const CONFIG_FILE: &str = "config.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformDirs {
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME`, or `~/.config`.
    pub config_home: PathBuf,
    /// `$XDG_DATA_HOME`, or `~/.local/share`.
    pub data_home: PathBuf,
}

impl PlatformDirs {
    /// XDG variables only count when they are absolute paths, as the spec
    /// says.
    pub fn new(
        home: PathBuf,
        xdg_config_home: Option<String>,
        xdg_data_home: Option<String>,
    ) -> Self {
        let absolute = |value: Option<String>| value.map(PathBuf::from).filter(|p| p.is_absolute());
        PlatformDirs {
            config_home: absolute(xdg_config_home).unwrap_or_else(|| home.join(".config")),
            data_home: absolute(xdg_data_home).unwrap_or_else(|| home.join(".local/share")),
            home,
        }
    }

    pub fn from_env() -> Result<Self> {
        let home = std::env::var("HOME").context("the HOME environment variable is not set")?;
        Ok(Self::new(
            PathBuf::from(home),
            std::env::var("XDG_CONFIG_HOME").ok(),
            std::env::var("XDG_DATA_HOME").ok(),
        ))
    }

    /// Where the app config is stored.
    pub fn config_file(&self) -> PathBuf {
        self.config_home.join(APP_DIR).join(CONFIG_FILE)
    }

    pub fn default_staging_dir(&self) -> PathBuf {
        self.data_home.join(APP_DIR).join("staging")
    }

    pub fn default_profiles_dir(&self) -> PathBuf {
        self.data_home.join(APP_DIR).join("profiles")
    }
}

// --- Finding the game ----------------------------------------------------

/// True if `dir` holds the game executable (matched ignoring case).
pub fn is_game_dir(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(GAME_EXECUTABLE)
        })
    })
}

/// The quoted strings on a line of Valve's key-value format, with `\\` and
/// `\"` unescaped.
fn quoted_strings(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut value = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => value.extend(chars.next()),
                other => value.push(other),
            }
        }
        out.push(value);
    }
    out
}

/// The library folders listed in Steam's `libraryfolders.vdf`.
pub fn parse_library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("\"path\""))
        .filter_map(|line| quoted_strings(line).into_iter().nth(1))
        .map(PathBuf::from)
        .collect()
}

/// Where Steam keeps its files on Linux (native install, the usual
/// symlinks, and the Flatpak).
fn steam_roots(dirs: &PlatformDirs) -> Vec<PathBuf> {
    vec![
        dirs.data_home.join("Steam"),
        dirs.home.join(".steam/steam"),
        dirs.home.join(".steam/root"),
        dirs.home
            .join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ]
}

/// Every GTA: San Andreas install found in a Steam library, without
/// duplicates, in the order they were found.
pub fn find_game_dirs(dirs: &PlatformDirs) -> Vec<PathBuf> {
    let mut libraries = Vec::new();
    for root in steam_roots(dirs) {
        let listed = fs::read_to_string(root.join("steamapps/libraryfolders.vdf"))
            .map(|text| parse_library_paths(&text))
            .unwrap_or_default();
        libraries.push(root);
        libraries.extend(listed);
    }

    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for library in libraries {
        let candidate = library.join("steamapps/common").join(GAME_FOLDER_NAME);
        let Ok(real) = candidate.canonicalize() else {
            continue;
        };
        if is_game_dir(&real) && seen.insert(real) {
            found.push(candidate);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn dirs_for(home: &Path) -> PlatformDirs {
        PlatformDirs::new(home.to_path_buf(), None, None)
    }

    fn install_game(library: &Path) -> PathBuf {
        let game = library.join("steamapps/common").join(GAME_FOLDER_NAME);
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join(GAME_EXECUTABLE), "").unwrap();
        game
    }

    // --- XDG locations ---

    #[test]
    fn defaults_follow_the_xdg_spec() {
        let dirs = PlatformDirs::new("/home/u".into(), None, None);
        assert_eq!(
            dirs.config_file(),
            Path::new("/home/u/.config/samml/config.json")
        );
        assert_eq!(
            dirs.default_staging_dir(),
            Path::new("/home/u/.local/share/samml/staging")
        );
        assert_eq!(
            dirs.default_profiles_dir(),
            Path::new("/home/u/.local/share/samml/profiles")
        );
    }

    #[test]
    fn absolute_xdg_variables_are_used_and_relative_ones_ignored() {
        let dirs = PlatformDirs::new("/home/u".into(), Some("/cfg".into()), Some("/data".into()));
        assert_eq!(dirs.config_file(), Path::new("/cfg/samml/config.json"));
        assert_eq!(dirs.default_staging_dir(), Path::new("/data/samml/staging"));

        let relative = PlatformDirs::new(
            "/home/u".into(),
            Some("relative/cfg".into()),
            Some(String::new()),
        );
        assert_eq!(relative.config_home, Path::new("/home/u/.config"));
        assert_eq!(relative.data_home, Path::new("/home/u/.local/share"));
    }

    // --- game detection ---

    #[test]
    fn a_game_dir_has_the_executable_in_any_case() {
        let dir = TempDir::new().unwrap();
        assert!(!is_game_dir(dir.path()));
        assert!(!is_game_dir(&dir.path().join("nope")));
        fs::write(dir.path().join("GTA_SA.EXE"), "").unwrap();
        assert!(is_game_dir(dir.path()));
    }

    #[test]
    fn reads_library_paths_from_steams_vdf() {
        let vdf = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/u/.local/share/Steam"
		"label"		""
		"apps"
		{
			"12120"		"1234"
		}
	}
	"1"
	{
		"path"		"/mnt/My \"Games\"/SteamLibrary"
	}
}
"#;
        assert_eq!(
            parse_library_paths(vdf),
            [
                PathBuf::from("/home/u/.local/share/Steam"),
                PathBuf::from("/mnt/My \"Games\"/SteamLibrary"),
            ]
        );
        assert!(parse_library_paths("").is_empty());
    }

    #[test]
    fn finds_the_game_in_the_default_steam_library() {
        let home = TempDir::new().unwrap();
        let steam = home.path().join(".local/share/Steam");
        let game = install_game(&steam);

        assert_eq!(find_game_dirs(&dirs_for(home.path())), [game]);
    }

    #[test]
    fn finds_the_game_in_a_secondary_library_listed_in_the_vdf() {
        let home = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        let steam = home.path().join(".local/share/Steam");
        fs::create_dir_all(steam.join("steamapps")).unwrap();
        fs::write(
            steam.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                other.path().display()
            ),
        )
        .unwrap();
        let game = install_game(other.path());

        assert_eq!(find_game_dirs(&dirs_for(home.path())), [game]);
    }

    #[test]
    fn the_same_install_reached_two_ways_is_reported_once() {
        let home = TempDir::new().unwrap();
        let steam = home.path().join(".local/share/Steam");
        install_game(&steam);
        fs::create_dir_all(home.path().join(".steam")).unwrap();
        std::os::unix::fs::symlink(&steam, home.path().join(".steam/steam")).unwrap();

        assert_eq!(find_game_dirs(&dirs_for(home.path())).len(), 1);
    }

    #[test]
    fn an_empty_game_folder_or_no_steam_finds_nothing() {
        let home = TempDir::new().unwrap();
        assert!(find_game_dirs(&dirs_for(home.path())).is_empty());

        let steam = home.path().join(".local/share/Steam");
        fs::create_dir_all(steam.join("steamapps/common").join(GAME_FOLDER_NAME)).unwrap();
        assert!(find_game_dirs(&dirs_for(home.path())).is_empty());
    }
}
