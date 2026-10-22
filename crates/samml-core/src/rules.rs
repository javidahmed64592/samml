//! Validation of a mod's file rules (`source` -> `target` pairs).
//!
//! Rules are typed in the UI or hand-edited, so they are checked before
//! being saved and again before deploy. Resolving rules into links is
//! `deploy`'s job.

use crate::manifest::FileRule;
use crate::paths;
use anyhow::{Context, Result, bail};

/// Checks every rule stays inside the mod folder and the game folder.
/// A target without a trailing slash is the exact destination, so it must
/// name something (`.` and `./` only make sense as "inside the game folder").
pub fn validate_rules(rules: &[FileRule]) -> Result<()> {
    for rule in rules {
        paths::validate_relative(&rule.source)
            .with_context(|| format!("invalid source '{}'", rule.source))?;
        paths::validate_relative(&rule.target)
            .with_context(|| format!("invalid target '{}'", rule.target))?;
        if !rule.target.ends_with('/') && !paths::names_something(&rule.target) {
            bail!(
                "target '{}' needs a name, or a trailing '/' to place the item inside the game folder",
                rule.target
            );
        }
    }
    Ok(())
}

/// Validates the rules and turns an empty list into `None`: no rules means
/// "deploy to modloader", the same as leaving them blank.
pub fn normalize_rules(files: Option<Vec<FileRule>>) -> Result<Option<Vec<FileRule>>> {
    match files {
        Some(rules) if !rules.is_empty() => {
            validate_rules(&rules)?;
            Ok(Some(rules))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(source: &str, target: &str) -> FileRule {
        FileRule {
            source: source.into(),
            target: target.into(),
        }
    }

    #[test]
    fn rules_must_stay_inside_their_folders() {
        assert!(validate_rules(&[rule("a/", "b/"), rule("file.txt", "x/y.txt")]).is_ok());
        assert!(
            validate_rules(&[rule("dir", "./")]).is_ok(),
            "into game folder"
        );
        assert!(validate_rules(&[rule("../x", "y")]).is_err());
        assert!(validate_rules(&[rule("x", "../y")]).is_err());
        assert!(validate_rules(&[rule("/etc/passwd", "y")]).is_err());
        assert!(validate_rules(&[rule("x", "/usr/bin")]).is_err());
        assert!(validate_rules(&[rule("", "y")]).is_err());
        assert!(validate_rules(&[rule("x", "")]).is_err());
    }

    #[test]
    fn exact_target_must_name_something() {
        assert!(validate_rules(&[rule("x", ".")]).is_err());
        assert!(validate_rules(&[rule("x", "./")]).is_ok());
    }

    #[test]
    fn empty_rules_mean_the_default_deployment() {
        assert_eq!(normalize_rules(None).unwrap(), None);
        assert_eq!(normalize_rules(Some(vec![])).unwrap(), None);
        assert!(
            normalize_rules(Some(vec![rule("a", "b")]))
                .unwrap()
                .is_some()
        );
        assert!(normalize_rules(Some(vec![rule("..", "b")])).is_err());
    }
}
