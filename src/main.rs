mod deploy;
mod manifest;

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (manifest_path, command) = match args.len() {
        2 => (PathBuf::from("app-manifest.json"), args[1].clone()),
        3 => (PathBuf::from(&args[1]), args[2].clone()),
        _ => bail!("usage: samml [app-manifest.json] <deploy|launch|run>"),
    };

    let app = manifest::load_app_manifest(&manifest_path)?;

    println!("{} mod(s) in app manifest:", app.manifests.len());
    for m in &app.manifests {
        println!(
            "  [{}] {} ({})",
            if m.active { "x" } else { " " },
            m.name,
            m.mod_path
        );
    }

    match command.as_str() {
        "deploy" => deploy(&app)?,
        "launch" => launch(&app)?,
        "run" => {
            deploy(&app)?;
            launch(&app)?;
        }
        other => bail!("unknown command: {other}"),
    }

    Ok(())
}

fn deploy(app: &manifest::AppManifest) -> Result<()> {
    deploy::deploy(app)?;
    println!("Deployed.");
    Ok(())
}

fn launch(app: &manifest::AppManifest) -> Result<()> {
    let app_id = app
        .steam_app_id
        .context("steam_app_id is not set in app-manifest.json")?;
    println!("Launching Steam App ID {app_id}...");
    Command::new("steam")
        .arg("-applaunch")
        .arg(app_id.to_string())
        .spawn()
        .context("failed to run 'steam' - is it on PATH?")?;
    Ok(())
}
