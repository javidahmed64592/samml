mod deploy;
mod manifest;

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // samml [app-manifest.json] clean
    // samml [app-manifest.json] <deploy|launch|run> <profile>
    let (manifest_path, command, profile_name) = match args.len() {
        2 if args[1] == "clean" => (PathBuf::from("app-manifest.json"), args[1].clone(), None),
        3 if args[2] == "clean" => (PathBuf::from(&args[1]), args[2].clone(), None),
        3 => (
            PathBuf::from("app-manifest.json"),
            args[1].clone(),
            Some(args[2].clone()),
        ),
        4 => (
            PathBuf::from(&args[1]),
            args[2].clone(),
            Some(args[3].clone()),
        ),
        _ => bail!(
            "usage: samml [app-manifest.json] <deploy|launch|run> <profile>\n       samml [app-manifest.json] clean"
        ),
    };

    let app = manifest::load_app_manifest(&manifest_path)?;

    match command.as_str() {
        "deploy" | "launch" | "run" => {
            let name = profile_name.as_deref().unwrap();
            let profiles = manifest::list_profiles(&app.profiles_dir)?;
            if !manifest::check_profile_exists(&profiles, name) {
                bail!(
                    "profile '{name}' not found in {}",
                    app.profiles_dir.display()
                );
            }
            let profile_path = app.profiles_dir.join(format!("{name}.json"));
            let profile = manifest::load_profile(&profile_path)?;

            println!("Profile '{}' — {} mod(s):", name, profile.manifests.len());
            for m in &profile.manifests {
                println!(
                    "  [{}] {} ({})",
                    if m.active { "x" } else { " " },
                    m.name,
                    m.mod_path
                );
            }

            match command.as_str() {
                "deploy" => deploy(&app, &profile.manifests)?,
                "launch" => launch(&app)?,
                "run" => {
                    deploy(&app, &profile.manifests)?;
                    launch(&app)?;
                }
                _ => unreachable!(),
            }
        }
        "clean" => {
            deploy::clean(&app)?;
            println!("Restored to vanilla.");
        }
        other => bail!("unknown command: {other}"),
    }

    Ok(())
}

fn deploy(app: &manifest::AppManifest, mods: &[manifest::ModManifest]) -> Result<()> {
    deploy::deploy(app, mods)?;
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
