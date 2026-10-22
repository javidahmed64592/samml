//! The deploy receipt: a record in `GAME_DIR` of exactly what a deploy
//! changed, so teardown can undo it and nothing else.
//!
//! Owns reading, writing and removing the receipt file. Knows nothing about
//! how links are made (that is `deploy`).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub(crate) const RECEIPT_FILE: &str = ".samml_receipt.json";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Receipt {
    /// Slug of the profile this deploy came from.
    #[serde(default)]
    pub profile: Option<String>,
    /// Every symlink path (relative to GAME_DIR) this tool created.
    pub links: Vec<String>,
    /// Every directory (relative to GAME_DIR) this tool had to create to
    /// make room for a symlink, that did NOT already exist beforehand.
    /// Deepest-first, matching creation order.
    #[serde(default)]
    pub created_dirs: Vec<String>,
}

fn receipt_path(game_dir: &Path) -> PathBuf {
    game_dir.join(RECEIPT_FILE)
}

/// Returns `None` when nothing is currently deployed.
pub fn read(game_dir: &Path) -> Result<Option<Receipt>> {
    let path = receipt_path(game_dir);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let receipt = serde_json::from_str(&text)
        .with_context(|| format!("parsing receipt {}", path.display()))?;
    Ok(Some(receipt))
}

pub fn write(game_dir: &Path, receipt: &Receipt) -> Result<()> {
    let path = receipt_path(game_dir);
    fs::write(&path, serde_json::to_string_pretty(receipt)?)
        .with_context(|| format!("writing {}", path.display()))
}

pub fn remove(game_dir: &Path) -> Result<()> {
    let path = receipt_path(game_dir);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}
