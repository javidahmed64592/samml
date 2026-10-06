use crate::manifest::{AppManifest, ModManifest};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

const RECEIPT_FILE: &str = ".samml_receipt.json";
const BACKUP_DIR: &str = ".samml_backups";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Receipt {
    /// Every symlink path (relative to GAME_DIR) this tool created.
    links: Vec<String>,
}

fn backup_path_for(game_dir: &Path, rel: &str) -> PathBuf {
    game_dir.join(BACKUP_DIR).join(rel)
}

/// Remove every symlink listed in the last receipt, then restore any stock
/// file/folder that was backed up to make room for it. We never touch
/// anything that isn't either one of our own symlinks or one of our own
/// backups.
fn teardown(game_dir: &Path) -> Result<()> {
    let receipt_path = game_dir.join(RECEIPT_FILE);
    if !receipt_path.exists() {
        return Ok(());
    }
    let receipt: Receipt = serde_json::from_str(&fs::read_to_string(&receipt_path)?)?;
    for rel in &receipt.links {
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
    Ok(())
}

/// Create `link_path -> target`, making sure parent directories exist.
///
/// If something real (not one of our symlinks) already sits at
/// `link_path` - e.g. a stock game file a mod wants to override - it gets
/// moved into `${GAME_DIR}/.samml_backups/<rel path>` first, so `teardown`
/// can put it back later. We refuse to proceed if a backup already exists
/// at that location, since that means a previous run left things in an
/// inconsistent state and blindly overwriting could lose the real original.
fn link(target: &Path, link_path: &Path, created: &mut Vec<String>, game_dir: &Path) -> Result<()> {
    if !target.exists() {
        bail!("source does not exist: {}", target.display());
    }
    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
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
                    link(&target_path, &link_path, created, game_dir)?;
                }
            }
        }
        None => {
            let modloader_dir = game_dir.join("modloader");
            if !modloader_dir.exists() {
                bail!(
                    "{}: modloader/ does not exist yet - deploy the manifest entry that \
                     creates it (e.g. a 'modloader' mod) earlier in app-manifest.json",
                    m.name
                );
            }
            let link_path = modloader_dir.join(&m.name);
            link(&mod_root, &link_path, created, game_dir)?;
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
    for m in mods.iter().filter(|m| m.active) {
        deploy_mod(m, staging_dir, game_dir, &mut created)
            .with_context(|| format!("deploying mod '{}'", m.name))?;
    }

    let receipt = Receipt { links: created };
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
