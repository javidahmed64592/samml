//! Working out which symlinks a mod needs in the game directory, without
//! creating any.
//!
//! Owns the rules for turning a mod (its name and optional file rules) into
//! `(link path, target)` pairs, including the rsync-style trailing-slash
//! convention. `deploy` creates the links; `plan` shows them to the player
//! first. Both use this module, so what is shown is what gets deployed.

use crate::manifest::DeployMod;
use crate::paths;
use crate::rules::validate_rules;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

/// The game folder modloader reads mods from.
pub const MODLOADER_DIR: &str = "modloader";

/// A symlink to create: `link` (in the game directory) pointing at
/// `target` (in staging).
pub type LinkPair = (PathBuf, PathBuf);

/// Why a mod without file rules cannot be deployed before `modloader/`
/// exists.
pub fn modloader_missing_message(mod_name: &str) -> String {
    format!(
        "{mod_name}: {MODLOADER_DIR}/ does not exist yet - deploy the mod that \
         creates it (e.g. a 'modloader' mod) earlier in the profile"
    )
}

/// Resolve one FileRule into concrete (link_path, target_real_path) pairs,
/// applying the rsync-style trailing-slash convention.
fn resolve_rule(
    mod_root: &Path,
    game_dir: &Path,
    source: &str,
    target: &str,
) -> Result<Vec<LinkPair>> {
    let merge_contents = source.ends_with('/');
    let place_inside = target.ends_with('/');

    let source_path = paths::join_normalized(mod_root, source);
    let target_path = paths::join_normalized(game_dir, target);

    let mut out = Vec::new();

    if merge_contents {
        let mut entries: Vec<_> = fs::read_dir(&source_path)
            .with_context(|| format!("reading {}", source_path.display()))?
            .collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let link_path = target_path.join(entry.file_name());
            out.push((link_path, entry.path()));
        }
    } else if place_inside {
        let name = source_path
            .file_name()
            .context("source path has no file name")?;
        out.push((target_path.join(name), source_path));
    } else {
        out.push((target_path, source_path));
    }

    Ok(out)
}

/// Every symlink the mod needs, in the order they should be created: its
/// file rules resolved, or (when it has none) the whole mod folder as a
/// unit at `modloader/<name>`.
///
/// The name, rules and sources are validated first: a hand-edited profile
/// or library gets the same checks as one edited through the app.
pub fn mod_links(m: &DeployMod, staging_dir: &Path, game_dir: &Path) -> Result<Vec<LinkPair>> {
    paths::validate_component(&m.name)
        .with_context(|| format!("'{}' cannot be used as a mod name", m.name))?;
    let mod_root = staging_dir.join(&m.name);
    if !mod_root.is_dir() {
        bail!("{} has no folder at {}", m.name, mod_root.display());
    }

    match &m.files {
        Some(rules) => {
            validate_rules(rules).with_context(|| format!("{}: invalid file rules", m.name))?;
            let mut links = Vec::new();
            for rule in rules {
                for (link_path, target) in
                    resolve_rule(&mod_root, game_dir, &rule.source, &rule.target)?
                {
                    if !target.exists() {
                        bail!("source does not exist: {}", target.display());
                    }
                    links.push((link_path, target));
                }
            }
            Ok(links)
        }
        None => Ok(vec![(game_dir.join(MODLOADER_DIR).join(&m.name), mod_root)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::FileRule;
    use tempfile::TempDir;

    fn rule(source: &str, target: &str) -> FileRule {
        FileRule {
            source: source.into(),
            target: target.into(),
        }
    }

    fn staged(files: &[&str]) -> (TempDir, PathBuf, PathBuf) {
        let root = TempDir::new().unwrap();
        let staging = root.path().join("staging");
        let game = root.path().join("game");
        fs::create_dir_all(&game).unwrap();
        for f in files {
            let path = staging.join(f);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        (root, staging, game)
    }

    #[test]
    fn resolves_rsync_style_rules() {
        let (_root, staging, game) = staged(&["m/dir/a.txt"]);
        let root = staging.join("m");

        // trailing slash on source: contents merged into target
        let r = resolve_rule(&root, &game, "dir/", "merged/").unwrap();
        assert_eq!(r, vec![(game.join("merged/a.txt"), root.join("dir/a.txt"))]);
        // no slashes: item placed at the exact path (rename)
        let r = resolve_rule(&root, &game, "dir", "renamed").unwrap();
        assert_eq!(r, vec![(game.join("renamed"), root.join("dir"))]);
        // trailing slash on target: item placed inside, keeping its name
        let r = resolve_rule(&root, &game, "dir", "container/").unwrap();
        assert_eq!(r, vec![(game.join("container/dir"), root.join("dir"))]);
        // `./` places the item directly in the game folder
        let r = resolve_rule(&root, &game, "dir", "./").unwrap();
        assert_eq!(r, vec![(game.join("dir"), root.join("dir"))]);
    }

    #[test]
    fn merged_contents_come_out_in_name_order() {
        let (_root, staging, game) = staged(&["m/d/c", "m/d/a", "m/d/b"]);
        let r = resolve_rule(&staging.join("m"), &game, "d/", "out/").unwrap();
        let names: Vec<_> = r
            .iter()
            .map(|(link, _)| link.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn a_mod_without_rules_is_one_link_into_modloader() {
        let (_root, staging, game) = staged(&["Cool Cars/file.txt"]);
        let m = DeployMod {
            name: "Cool Cars".into(),
            files: None,
        };

        let links = mod_links(&m, &staging, &game).unwrap();

        assert_eq!(
            links,
            vec![(game.join("modloader/Cool Cars"), staging.join("Cool Cars"))]
        );
    }

    #[test]
    fn a_mod_with_rules_gets_its_rules_in_order() {
        let (_root, staging, game) = staged(&["M/a.txt", "M/b.txt"]);
        let m = DeployMod {
            name: "M".into(),
            files: Some(vec![rule("b.txt", "b.txt"), rule("a.txt", "sub/")]),
        };

        let links = mod_links(&m, &staging, &game).unwrap();

        assert_eq!(
            links,
            vec![
                (game.join("b.txt"), staging.join("M/b.txt")),
                (game.join("sub/a.txt"), staging.join("M/a.txt")),
            ]
        );
    }

    #[test]
    fn bad_mods_are_reported_with_the_reason() {
        let (_root, staging, game) = staged(&["M/a.txt"]);
        let err = |m: DeployMod| format!("{:#}", mod_links(&m, &staging, &game).unwrap_err());
        let with = |name: &str, files| DeployMod {
            name: name.into(),
            files,
        };

        assert!(err(with("../escape", None)).contains("cannot be used as a mod name"));
        assert!(err(with("Ghost", None)).contains("has no folder"));
        assert!(
            err(with("M", Some(vec![rule("a.txt", "../../evil")]))).contains("invalid file rules")
        );
        assert!(
            err(with("M", Some(vec![rule("nope.txt", "x")]))).contains("source does not exist")
        );
        assert!(err(with("M", Some(vec![rule("nodir/", "x/")]))).contains("reading"));
    }
}
