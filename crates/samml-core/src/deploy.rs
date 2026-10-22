use crate::manifest::{AppManifest, ModManifest};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

const RECEIPT_FILE: &str = ".samml_receipt.json";
const BACKUP_DIR: &str = ".samml_backups";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Receipt {
    /// Every symlink path (relative to GAME_DIR) this tool created.
    links: Vec<String>,
    /// Every directory (relative to GAME_DIR) this tool had to create to
    /// make room for a symlink, that did NOT already exist beforehand.
    /// Deepest-first, matching creation order.
    #[serde(default)]
    created_dirs: Vec<String>,
}

fn backup_path_for(game_dir: &Path, rel: &str) -> PathBuf {
    game_dir.join(BACKUP_DIR).join(rel)
}

/// `fs::create_dir_all`, but recording every ancestor directory that didn't
/// already exist (deepest first) into `created_dirs`, so teardown can
/// remove exactly those later - never a directory that was already there
/// before we touched anything (e.g. the game's own `data/` folder).
fn create_dir_all_tracked(
    path: &Path,
    game_dir: &Path,
    created_dirs: &mut Vec<String>,
) -> Result<()> {
    let mut missing = Vec::new();
    let mut cur = path;
    while !cur.exists() {
        missing.push(cur.to_path_buf());
        match cur.parent() {
            Some(p) => cur = p,
            None => break,
        }
    }
    fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))?;
    for p in missing {
        let rel = p
            .strip_prefix(game_dir)
            .unwrap_or(&p)
            .to_string_lossy()
            .to_string();
        created_dirs.push(rel);
    }
    Ok(())
}

/// Remove every symlink listed in the last receipt, then restore any stock
/// file/folder that was backed up to make room for it, then remove any
/// directory we created that's now empty. We never touch anything that
/// isn't either one of our own symlinks, one of our own backups, or one of
/// our own (now-empty) directories.
///
/// Symlinks are processed in REVERSE of creation order. Some entries are
/// themselves a directory deployed as a single symlink (e.g. the
/// `modloader` or `cleo` system mod's folder) with other mods' entries
/// placed as children *inside* that symlink - meaning those children
/// actually live inside STAGING_DIR, reached only by resolving through the
/// container symlink. Removing the container first would sever that path
/// and orphan its children in the staging tree with no way left to find
/// them. Since a container symlink is always created before anything is
/// placed inside it, undoing in reverse order guarantees children are
/// removed first.
fn teardown(game_dir: &Path) -> Result<()> {
    let receipt_path = game_dir.join(RECEIPT_FILE);
    if !receipt_path.exists() {
        return Ok(());
    }
    let receipt: Receipt = serde_json::from_str(&fs::read_to_string(&receipt_path)?)?;

    for rel in receipt.links.iter().rev() {
        let path = game_dir.join(rel);
        if path.is_symlink() {
            fs::remove_file(&path)
                .with_context(|| format!("removing stale symlink {}", path.display()))?;
        }

        let backup = backup_path_for(game_dir, rel);
        if backup.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&backup, &path).with_context(|| {
                format!(
                    "restoring backup {} -> {}",
                    backup.display(),
                    path.display()
                )
            })?;
        }
    }

    // Deepest-first (matches creation order), so a now-empty child
    // directory is gone before we try its now-maybe-empty parent.
    for rel in &receipt.created_dirs {
        let path = game_dir.join(rel);
        if !path.is_dir() {
            continue;
        }
        match fs::remove_dir(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty => {
                // Something else (another mod, the user, the game itself)
                // put real content in here - leave it alone.
            }
            Err(e) => {
                return Err(e).with_context(|| format!("removing directory {}", path.display()));
            }
        }
    }

    Ok(())
}

/// Create `link_path -> target`, making sure parent directories exist
/// (tracked via `create_dir_all_tracked` so teardown can clean them back
/// up if they end up empty).
///
/// If something real (not one of our symlinks) already sits at
/// `link_path` - e.g. a stock game file a mod wants to override - it gets
/// moved into `${GAME_DIR}/.samml_backups/<rel path>` first, so `teardown`
/// can put it back later. We refuse to proceed if a backup already exists
/// at that location, since that means a previous run left things in an
/// inconsistent state and blindly overwriting could lose the real original.
fn link(
    target: &Path,
    link_path: &Path,
    created: &mut Vec<String>,
    created_dirs: &mut Vec<String>,
    game_dir: &Path,
) -> Result<()> {
    if !target.exists() {
        bail!("source does not exist: {}", target.display());
    }
    if let Some(parent) = link_path.parent() {
        create_dir_all_tracked(parent, game_dir, created_dirs)?;
    }

    let rel = link_path
        .strip_prefix(game_dir)
        .unwrap_or(link_path)
        .to_string_lossy()
        .to_string();

    if link_path.is_symlink() {
        fs::remove_file(link_path)?;
    } else if link_path.exists() {
        let backup = backup_path_for(game_dir, &rel);
        if backup.exists() {
            bail!(
                "a backup already exists at {} but {} is not a symlink - \
                 refusing to proceed, this looks like a leftover inconsistent state",
                backup.display(),
                link_path.display()
            );
        }
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(link_path, &backup).with_context(|| {
            format!("backing up {} -> {}", link_path.display(), backup.display())
        })?;
    }

    symlink(target, link_path)
        .with_context(|| format!("linking {} -> {}", link_path.display(), target.display()))?;
    created.push(rel);
    Ok(())
}

/// Resolve one FileRule into concrete (link_path, target_real_path) pairs,
/// applying the rsync-style trailing-slash convention.
fn resolve_rule(
    mod_root: &Path,
    game_dir: &Path,
    source: &str,
    target: &str,
) -> Result<Vec<(PathBuf, PathBuf)>> {
    let merge_contents = source.ends_with('/');
    let place_inside = target.ends_with('/');

    let source_path = mod_root.join(source.trim_end_matches('/'));
    let target_path = game_dir.join(target.trim_end_matches('/'));

    let mut out = Vec::new();

    if merge_contents {
        for entry in fs::read_dir(&source_path)
            .with_context(|| format!("reading {}", source_path.display()))?
        {
            let entry = entry?;
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

/// Deploy a single mod: either its explicit `files` rules, or (when `files`
/// is omitted entirely) the whole mod folder as a unit to modloader/<name>.
fn deploy_mod(
    m: &ModManifest,
    staging_dir: &Path,
    game_dir: &Path,
    created: &mut Vec<String>,
    created_dirs: &mut Vec<String>,
) -> Result<()> {
    let mod_root = staging_dir.join(&m.mod_path);
    if !mod_root.is_dir() {
        bail!("{} has no folder at {}", m.name, mod_root.display());
    }

    match &m.files {
        Some(rules) => {
            for rule in rules {
                for (link_path, target_path) in
                    resolve_rule(&mod_root, game_dir, &rule.source, &rule.target)?
                {
                    link(&target_path, &link_path, created, created_dirs, game_dir)?;
                }
            }
        }
        None => {
            let modloader_dir = game_dir.join("modloader");
            if !modloader_dir.exists() {
                bail!(
                    "{}: modloader/ does not exist yet - deploy the manifest entry that \
                     creates it (e.g. a 'modloader' mod) earlier in the profile",
                    m.name
                );
            }
            let link_path = modloader_dir.join(&m.name);
            link(&mod_root, &link_path, created, created_dirs, game_dir)?;
        }
    }
    Ok(())
}

/// Deploy every active manifest, in array order. Symlinks straight from
/// STAGING_DIR into GAME_DIR; nothing is copied. Any stock file a mod
/// overwrites is backed up first and restored on the next teardown.
pub fn deploy(app: &AppManifest, mods: &[ModManifest]) -> Result<()> {
    let game_dir = &app.game_dir;
    let staging_dir = &app.staging_dir;

    teardown(game_dir)?;

    let mut created = Vec::new();
    let mut created_dirs = Vec::new();
    for m in mods.iter().filter(|m| m.active) {
        deploy_mod(m, staging_dir, game_dir, &mut created, &mut created_dirs)
            .with_context(|| format!("deploying mod '{}'", m.name))?;
    }

    let receipt = Receipt {
        links: created,
        created_dirs,
    };
    fs::write(
        game_dir.join(RECEIPT_FILE),
        serde_json::to_string_pretty(&receipt)?,
    )?;
    Ok(())
}

/// Tear down to a clean vanilla state without deploying anything.
pub fn clean(app: &AppManifest) -> Result<()> {
    teardown(&app.game_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::FileRule;
    use tempfile::TempDir;

    struct Env {
        _root: TempDir,
        app: AppManifest,
    }

    impl Env {
        fn new() -> Self {
            let root = TempDir::new().unwrap();
            let game_dir = root.path().join("game");
            let staging_dir = root.path().join("staging");
            fs::create_dir_all(&game_dir).unwrap();
            fs::create_dir_all(&staging_dir).unwrap();
            let app = AppManifest {
                game_dir,
                staging_dir,
                profiles_dir: root.path().join("profiles"),
                steam_app_id: None,
            };
            Env { _root: root, app }
        }

        fn game(&self, rel: &str) -> PathBuf {
            self.app.game_dir.join(rel)
        }

        fn stage(&self, rel: &str, contents: &str) {
            let path = self.app.staging_dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }

        fn write_game(&self, rel: &str, contents: &str) {
            let path = self.game(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    fn rule(source: &str, target: &str) -> FileRule {
        FileRule {
            source: source.into(),
            target: target.into(),
        }
    }

    fn mod_with_rules(name: &str, path: &str, files: Vec<FileRule>) -> ModManifest {
        ModManifest {
            name: name.into(),
            mod_path: path.into(),
            active: true,
            files: Some(files),
        }
    }

    fn plain_mod(name: &str, path: &str) -> ModManifest {
        ModManifest {
            name: name.into(),
            mod_path: path.into(),
            active: true,
            files: None,
        }
    }

    fn modloader() -> ModManifest {
        mod_with_rules(
            "Modloader",
            "modloader",
            vec![rule("modloader", "modloader")],
        )
    }

    #[test]
    fn resolves_rsync_style_rules() {
        let env = Env::new();
        env.stage("m/dir/a.txt", "a");
        let root = env.app.staging_dir.join("m");
        let game = &env.app.game_dir;

        // trailing slash on source: contents merged into target
        let r = resolve_rule(&root, game, "dir/", "merged/").unwrap();
        assert_eq!(r, vec![(game.join("merged/a.txt"), root.join("dir/a.txt"))]);
        // no slashes: item placed at the exact path (rename)
        let r = resolve_rule(&root, game, "dir", "renamed").unwrap();
        assert_eq!(r, vec![(game.join("renamed"), root.join("dir"))]);
        // trailing slash on target: item placed inside, keeping its name
        let r = resolve_rule(&root, game, "dir", "container/").unwrap();
        assert_eq!(r, vec![(game.join("container/dir"), root.join("dir"))]);
    }

    #[test]
    fn deploys_default_mod_into_modloader() {
        let env = Env::new();
        env.stage("modloader/modloader/modloader.asi", "loader");
        env.stage("mods/Cool/file.txt", "hi");
        let mods = [modloader(), plain_mod("Cool", "mods/Cool")];

        deploy(&env.app, &mods).unwrap();

        assert!(env.game("modloader").is_symlink());
        assert_eq!(
            fs::read_to_string(env.game("modloader/Cool/file.txt")).unwrap(),
            "hi"
        );
        assert!(env.game(RECEIPT_FILE).is_file());
    }

    #[test]
    fn deploys_explicit_rules() {
        let env = Env::new();
        env.stage("m/contents/a.txt", "a");
        env.stage("m/whole/b.txt", "b");
        env.stage("m/file.txt", "c");
        let mods = [mod_with_rules(
            "M",
            "m",
            vec![
                rule("contents/", "merged/"),
                rule("whole", "renamed"),
                rule("file.txt", "container/"),
            ],
        )];

        deploy(&env.app, &mods).unwrap();

        assert_eq!(fs::read_to_string(env.game("merged/a.txt")).unwrap(), "a");
        assert_eq!(fs::read_to_string(env.game("renamed/b.txt")).unwrap(), "b");
        assert_eq!(
            fs::read_to_string(env.game("container/file.txt")).unwrap(),
            "c"
        );
        // contents were merged, so `merged` is a real dir, not a symlink
        assert!(env.game("merged").is_dir() && !env.game("merged").is_symlink());
        assert!(env.game("merged/a.txt").is_symlink());
    }

    #[test]
    fn skips_inactive_mods() {
        let env = Env::new();
        env.stage("m/file.txt", "x");
        let mut m = mod_with_rules("M", "m", vec![rule("file.txt", "file.txt")]);
        m.active = false;

        deploy(&env.app, &[m]).unwrap();

        assert!(!env.game("file.txt").exists());
    }

    #[test]
    fn default_mod_requires_modloader_first() {
        let env = Env::new();
        env.stage("mods/Cool/file.txt", "hi");

        let err = deploy(&env.app, &[plain_mod("Cool", "mods/Cool")]).unwrap_err();

        assert!(format!("{err:#}").contains("modloader/ does not exist"));
    }

    #[test]
    fn errors_on_missing_mod_folder_or_source() {
        let env = Env::new();
        let err = deploy(&env.app, &[plain_mod("Ghost", "mods/Ghost")]).unwrap_err();
        assert!(format!("{err:#}").contains("has no folder"));

        env.stage("m/other.txt", "x");
        let m = mod_with_rules("M", "m", vec![rule("missing.txt", "missing.txt")]);
        let err = deploy(&env.app, &[m]).unwrap_err();
        assert!(format!("{err:#}").contains("source does not exist"));
    }

    #[test]
    fn backs_up_and_restores_stock_files() {
        let env = Env::new();
        env.write_game("data/stock.dat", "original");
        env.stage("m/stock.dat", "modded");
        let mods = [mod_with_rules(
            "M",
            "m",
            vec![rule("stock.dat", "data/stock.dat")],
        )];

        deploy(&env.app, &mods).unwrap();
        assert_eq!(
            fs::read_to_string(env.game("data/stock.dat")).unwrap(),
            "modded"
        );
        assert_eq!(
            fs::read_to_string(env.game(".samml_backups/data/stock.dat")).unwrap(),
            "original"
        );

        clean(&env.app).unwrap();
        let restored = env.game("data/stock.dat");
        assert!(!restored.is_symlink());
        assert_eq!(fs::read_to_string(restored).unwrap(), "original");
        assert!(!env.game(".samml_backups/data/stock.dat").exists());
    }

    #[test]
    fn clean_removes_links_and_created_dirs_only() {
        let env = Env::new();
        env.write_game("data/stock.dat", "original");
        env.stage("m/a.txt", "a");
        env.stage("m/b.txt", "b");
        let mods = [mod_with_rules(
            "M",
            "m",
            vec![rule("a.txt", "data/a.txt"), rule("b.txt", "new/deep/b.txt")],
        )];

        deploy(&env.app, &mods).unwrap();
        assert!(env.game("new/deep/b.txt").is_symlink());

        clean(&env.app).unwrap();

        assert!(!env.game("data/a.txt").exists());
        assert!(!env.game("new").exists(), "dirs we created are removed");
        assert!(
            env.game("data/stock.dat").is_file(),
            "pre-existing dirs stay"
        );
        // staging is untouched
        assert!(env.app.staging_dir.join("m/a.txt").is_file());
    }

    #[test]
    fn clean_leaves_foreign_files_in_created_dirs() {
        let env = Env::new();
        env.stage("m/a.txt", "a");
        let mods = [mod_with_rules("M", "m", vec![rule("a.txt", "new/a.txt")])];
        deploy(&env.app, &mods).unwrap();
        env.write_game("new/user.txt", "keep me");

        clean(&env.app).unwrap();

        assert!(!env.game("new/a.txt").exists());
        assert!(env.game("new/user.txt").is_file());
    }

    #[test]
    fn clean_is_a_noop_without_a_receipt() {
        let env = Env::new();
        env.write_game("data/stock.dat", "original");

        clean(&env.app).unwrap();

        assert!(env.game("data/stock.dat").is_file());
    }

    #[test]
    fn redeploy_replaces_previous_deployment() {
        let env = Env::new();
        env.stage("m/a.txt", "a");
        env.stage("m/b.txt", "b");
        let first = [mod_with_rules("M", "m", vec![rule("a.txt", "a.txt")])];
        let second = [mod_with_rules("M", "m", vec![rule("b.txt", "b.txt")])];

        deploy(&env.app, &first).unwrap();
        deploy(&env.app, &second).unwrap();

        assert!(!env.game("a.txt").exists());
        assert!(env.game("b.txt").is_symlink());
    }

    #[test]
    fn refuses_to_overwrite_an_existing_backup() {
        let env = Env::new();
        env.write_game("stock.dat", "original");
        env.write_game(".samml_backups/stock.dat", "older original");
        env.stage("m/stock.dat", "modded");
        let mods = [mod_with_rules(
            "M",
            "m",
            vec![rule("stock.dat", "stock.dat")],
        )];

        let err = deploy(&env.app, &mods).unwrap_err();

        assert!(format!("{err:#}").contains("leftover inconsistent state"));
        assert_eq!(
            fs::read_to_string(env.game(".samml_backups/stock.dat")).unwrap(),
            "older original"
        );
    }
}
