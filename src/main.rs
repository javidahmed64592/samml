mod deploy;
mod manifest;

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // samml [app-manifest.json] clean
    // samml [app-manifest.json] list
    // samml [app-manifest.json] <deploy|launch|run> <profile-slug>
    let (manifest_path, command, profile_slug) = match args.len() {
        2 if args[1] == "clean" || args[1] == "list" => {
            (PathBuf::from("app-manifest.json"), args[1].clone(), None)
        }
        3 if args[2] == "clean" || args[2] == "list" => {
            (PathBuf::from(&args[1]), args[2].clone(), None)
        }
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
            "usage: samml [app-manifest.json] <deploy|launch|run> <profile-slug>\n       samml [app-manifest.json] new <profile name>\n       samml [app-manifest.json] list\n       samml [app-manifest.json] clean"
        ),
    };

    let app = manifest::load_app_manifest(&manifest_path)?;

    match command.as_str() {
        "deploy" | "launch" | "run" => {
            let slug = profile_slug.as_deref().unwrap();
            let profile =
                manifest::find_profile_by_slug(&app.profiles_dir, slug)?.with_context(|| {
                    format!(
                        "profile '{slug}' not found in {}",
                        app.profiles_dir.display()
                    )
                })?;

            println!(
                "Profile '{}' ({slug}) - {} mod(s):",
                profile.name,
                profile.manifests.len()
            );
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
        "new" => {
            let name = profile_slug
                .as_deref()
                .context("usage: samml [app-manifest.json] new <profile name>")?;
            let (slug, path) = manifest::create_profile(&app.profiles_dir, name)?;
            println!("Created profile '{name}' ({slug}) at {}", path.display());
        }
        "list" => {
            let profiles = manifest::list_profiles(&app.profiles_dir)?;
            if profiles.is_empty() {
                println!("No profiles found in {}", app.profiles_dir.display());
            }
            for p in profiles {
                println!("{} - {}", p.slug, p.name);
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
