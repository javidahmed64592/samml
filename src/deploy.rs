use crate::manifest::{AppManifest, ModManifest};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

const RECEIPT_FILE: &str = ".samml_receipt.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Receipt {
    /// Every symlink path (relative to GAME_DIR) this tool created.
    links: Vec<String>,
}

/// Remove every symlink listed in the last receipt, then nothing else —
/// we never touch files/folders we didn't create ourselves.
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
    }
    Ok(())
}

/// Create `link_path -> target`, making sure parent directories exist.
/// Refuses to touch anything at `link_path` that isn't a symlink we can
/// safely replace — we never delete real files/directories that happen to
/// collide with a deploy target.
fn link(target: &Path, link_path: &Path, created: &mut Vec<String>, game_dir: &Path) -> Result<()> {
    if !target.exists() {
        bail!("source does not exist: {}", target.display());
    }
    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    if link_path.is_symlink() {
        fs::remove_file(link_path)?;
    } else if link_path.exists() {
        bail!(
            "refusing to overwrite non-symlink path {} — remove it manually first",
            link_path.display()
        );
    }
    symlink(target, link_path)
        .with_context(|| format!("linking {} -> {}", link_path.display(), target.display()))?;

    let rel = link_path
        .strip_prefix(game_dir)
        .unwrap_or(link_path)
        .to_string_lossy()
        .to_string();
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
        // Source's own directory name disappears; each child is linked
        // directly into the target directory.
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
        // Exact path, possibly renaming.
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
            // Only what's listed gets deployed. Anything else under
            // mod_root is ignored, per the README's "files specified ->
            // everything else ignored" rule.
            for rule in rules {
                for (link_path, target_path) in
                    resolve_rule(&mod_root, game_dir, &rule.source, &rule.target)?
                {
                    link(&target_path, &link_path, created, game_dir)?;
                }
            }
        }
        None => {
            // Whole folder, as a single unit, into modloader/<name>.
            let link_path = game_dir.join("modloader").join(&m.name);
            link(&mod_root, &link_path, created, game_dir)?;
        }
    }
    Ok(())
}

/// Deploy every active manifest. Symlinks straight from STAGING_DIR into
/// GAME_DIR; nothing is copied.
pub fn deploy(app: &AppManifest) -> Result<()> {
    let game_dir = &app.game_dir;
    let staging_dir = &app.staging_dir;

    teardown(game_dir)?;

    let mut created = Vec::new();
    for m in app.manifests.iter().filter(|m| m.active) {
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
