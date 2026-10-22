use anyhow::{Context, Result, bail};
use samml_core::{config, deploy, launch, library, manifest, profiles, resolve};
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // samml [app_manifest.json] clean
    // samml [app_manifest.json] list
    // samml [app_manifest.json] <show|deploy|launch|run> <profile_slug>
    let (manifest_path, command, profile_slug) = match args.len() {
        2 if matches!(args[1].as_str(), "clean" | "list" | "mods") => {
            (PathBuf::from("app_manifest.json"), args[1].clone(), None)
        }
        3 if matches!(args[2].as_str(), "clean" | "list" | "mods") => {
            (PathBuf::from(&args[1]), args[2].clone(), None)
        }
        3 => (
            PathBuf::from("app_manifest.json"),
            args[1].clone(),
            Some(args[2].clone()),
        ),
        4 => (
            PathBuf::from(&args[1]),
            args[2].clone(),
            Some(args[3].clone()),
        ),
        _ => bail!(
            "usage: samml [app_manifest.json] <show|deploy|launch|run> <profile_slug>\n       samml [app_manifest.json] new <profile name>\n       samml [app_manifest.json] list\n       samml [app_manifest.json] mods\n       samml [app_manifest.json] clean"
        ),
    };

    let app = config::load_app_manifest(&manifest_path)?;

    match command.as_str() {
        "show" | "deploy" | "launch" | "run" => {
            let slug = profile_slug.as_deref().unwrap();
            let profile =
                profiles::find_profile_by_slug(&app.profiles_dir, slug)?.with_context(|| {
                    format!(
                        "profile '{slug}' not found in {}",
                        app.profiles_dir.display()
                    )
                })?;

            println!(
                "Profile '{}' ({slug}) - {} mod(s):",
                profile.name,
                profile.mods.len()
            );
            for m in resolve::describe_profile(&app.staging_dir, &profile)? {
                println!(
                    "  [{}] {}{}",
                    if m.active { "x" } else { " " },
                    m.name.as_deref().unwrap_or(&m.id),
                    m.problem
                        .map(|p| format!("  (PROBLEM: {p})"))
                        .unwrap_or_default()
                );
            }

            match command.as_str() {
                "show" => {}
                "deploy" => deploy(&app, slug, &profile)?,
                "launch" => launch(&app)?,
                "run" => {
                    deploy(&app, slug, &profile)?;
                    launch(&app)?;
                }
                _ => unreachable!(),
            }
        }
        "new" => {
            let name = profile_slug
                .as_deref()
                .context("usage: samml [app_manifest.json] new <profile name>")?;
            let (slug, path) = profiles::create_profile(&app.profiles_dir, name)?;
            println!("Created profile '{name}' ({slug}) at {}", path.display());
        }
        "list" => {
            let profiles = profiles::list_profiles(&app.profiles_dir)?;
            if profiles.is_empty() {
                println!("No profiles found in {}", app.profiles_dir.display());
            }
            for p in profiles {
                println!("{} - {}", p.slug, p.name);
            }
        }
        "mods" => {
            let mods = library::list_mods(&app.staging_dir)?;
            if mods.is_empty() {
                println!("No mods found in {}", app.staging_dir.display());
            }
            for m in mods {
                let note = match (&m.problem, &m.files) {
                    (Some(problem), _) => format!("  (PROBLEM: {problem})"),
                    (None, Some(files)) => format!("  ({} file rule(s))", files.len()),
                    (None, None) => String::new(),
                };
                println!("{}  [{}]{note}", m.name, m.id);
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

fn deploy(app: &manifest::AppManifest, slug: &str, profile: &manifest::Profile) -> Result<()> {
    let mods = resolve::resolve_profile(&app.staging_dir, profile)?;
    deploy::deploy(app, slug, &mods)?;
    println!("Deployed.");
    Ok(())
}

fn launch(app: &manifest::AppManifest) -> Result<()> {
    launch::launch(app)?;
    println!(
        "Launched Steam App ID {}.",
        app.steam_app_id.unwrap_or_default()
    );
    Ok(())
}
