//! Validation and joining of user-supplied relative paths (mod folders,
//! file-rule sources and targets, ids). Anything typed into the UI or a
//! hand-edited profile goes through here before it touches the disk, so a
//! path can never climb out of STAGING_DIR or GAME_DIR.

use anyhow::{Result, bail};
use std::path::{Component, Path, PathBuf};

/// A single path component, such as a mod's folder id: no separators, not
/// `.`/`..`, not hidden.
pub fn validate_component(value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with('.')
        || value.contains(['/', '\\', '\0'])
        || value.trim() != value
    {
        bail!("'{value}' is not a valid folder name");
    }
    Ok(())
}

/// The folder-safe version of a display name, keeping the name readable:
/// path separators and control characters are removed, leading dots
/// stripped and the ends trimmed. A name that is already a valid component
/// comes back unchanged. May be empty if nothing usable is left.
pub fn clean_component(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| !matches!(c, '/' | '\\') && !c.is_control())
        .collect();
    kept.trim().trim_start_matches('.').trim_start().to_string()
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
        for ok in ["cool_mod", "Test Mod 1", "cleo-5.4.0"] {
            assert!(validate_component(ok).is_ok(), "{ok:?}");
        }
        for bad in ["", ".", "..", ".hidden", "a/b", "a\\b", " pad", "pad "] {
            assert!(validate_component(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn clean_component_keeps_names_readable() {
        assert_eq!(clean_component("Cool Car Pack"), "Cool Car Pack");
        assert_eq!(clean_component("  Cool Car Pack  "), "Cool Car Pack");
        assert_eq!(clean_component("a/b\\c"), "abc");
        assert_eq!(clean_component("..hidden"), "hidden");
        assert_eq!(clean_component("tab\there"), "tabhere");
        assert_eq!(clean_component("///"), "");
        // valid components are left exactly as they are
        for ok in ["Test Mod 1", "cleo-5.4.0", "a  b"] {
            assert!(validate_component(ok).is_ok());
            assert_eq!(clean_component(ok), ok);
        }
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
