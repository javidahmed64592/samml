//! Validation and joining of user-supplied relative paths (mod folders,
//! file-rule sources and targets, mod names). Anything typed into the UI or a
//! hand-edited profile goes through here before it touches the disk, so a
//! path can never climb out of STAGING_DIR or GAME_DIR.

use anyhow::{Result, bail};
use std::path::{Component, Path, PathBuf};

/// Longest single file or folder name Linux filesystems allow, in bytes.
const MAX_COMPONENT_BYTES: usize = 255;

/// A single path component, such as a mod's name (which is also its folder
/// name in game). Errors say what is wrong, since they are shown to players.
pub fn validate_component(value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("cannot be empty");
    }
    if value.trim() != value {
        bail!("cannot start or end with a space");
    }
    if let Some(c) = value.chars().find(|c| matches!(c, '/' | '\\')) {
        bail!("cannot contain '{c}'");
    }
    if value.chars().any(char::is_control) {
        bail!("cannot contain control characters");
    }
    if value.starts_with('.') {
        bail!("cannot start with a '.' (those folders are hidden)");
    }
    if value.len() > MAX_COMPONENT_BYTES {
        bail!("is too long (at most {MAX_COMPONENT_BYTES} bytes)");
    }
    Ok(())
}

/// A relative path that stays inside its base directory: non-empty, no
/// absolute prefix, no `..`. `.` and trailing slashes are allowed.
pub fn validate_relative(value: &str) -> Result<()> {
    if value.is_empty() || value.contains('\0') {
        bail!("path cannot be empty");
    }
    for component in Path::new(value).components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!(
                    "'{value}' must be a relative path inside its folder (no '..' or leading '/')"
                )
            }
        }
    }
    Ok(())
}

/// True if the path names something (as opposed to just `.` or `./`).
pub fn names_something(value: &str) -> bool {
    Path::new(value)
        .components()
        .any(|c| matches!(c, Component::Normal(_)))
}

/// `base` joined with `rel`, ignoring `.` components and trailing slashes,
/// so `join_normalized(game, "./")` is just `game`. `rel` must already have
/// passed `validate_relative`.
pub fn join_normalized(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for component in Path::new(rel).components() {
        if let Component::Normal(part) = component {
            out.push(part);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_must_be_a_single_plain_name() {
        for ok in [
            "cool_mod",
            "Test Mod 1",
            "cleo-5.4.0",
            "a  b",
            "Mod (v2) [HD]!",
        ] {
            assert!(validate_component(ok).is_ok(), "{ok:?}");
        }
    }

    #[test]
    fn invalid_components_say_why() {
        let reason = |v: &str| validate_component(v).unwrap_err().to_string();
        assert!(reason("").contains("empty"));
        assert!(reason(" pad").contains("start or end with a space"));
        assert!(reason("pad ").contains("start or end with a space"));
        assert!(reason("a/b").contains("'/'"));
        assert!(reason("a\\b").contains("'\\'"));
        assert!(reason("tab\there").contains("control characters"));
        assert!(reason(".hidden").contains("start with a '.'"));
        assert!(reason("..").contains("start with a '.'"));
        assert!(reason(&"x".repeat(256)).contains("too long"));
        assert!(validate_component(&"x".repeat(255)).is_ok());
    }

    #[test]
    fn relative_paths_cannot_escape() {
        for ok in [
            "a",
            "a/b",
            "a/b/",
            "./a",
            ".",
            "./",
            "a/./b",
            "dir with spaces/x.txt",
        ] {
            assert!(validate_relative(ok).is_ok(), "{ok:?}");
        }
        for bad in ["", "/etc", "../x", "a/../../x", "a/..", ".."] {
            assert!(validate_relative(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn names_something_ignores_dots() {
        assert!(names_something("a"));
        assert!(names_something("./a/"));
        assert!(!names_something("."));
        assert!(!names_something("./"));
    }

    #[test]
    fn join_normalized_drops_dots_and_trailing_slashes() {
        let base = Path::new("/game");
        assert_eq!(join_normalized(base, "./"), PathBuf::from("/game"));
        assert_eq!(join_normalized(base, "a/./b/"), PathBuf::from("/game/a/b"));
        assert_eq!(join_normalized(base, "x.txt"), PathBuf::from("/game/x.txt"));
    }
}
