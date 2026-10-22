use crate::manifest::AppManifest;
use anyhow::{Context, Result};
use std::process::Command;

pub fn launch(app: &AppManifest) -> Result<()> {
    let app_id = app
        .steam_app_id
        .context("steam_app_id is not set in app_manifest.json")?;
    Command::new("steam")
        .arg("-applaunch")
        .arg(app_id.to_string())
        .spawn()
        .context("failed to run 'steam' - is it on PATH?")?;
    Ok(())
}
