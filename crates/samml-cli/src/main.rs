use anyhow::{Context, Result, bail};
use samml_core::config;
use samml_core::manager::Manager;
use samml_core::manifest::{self, ConfigStatus, DeployPlan, DeployState, ProfileView};
use samml_core::platform::PlatformDirs;
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // samml [config.json] init [game_dir]
    // samml [config.json] clean
    // samml [config.json] list
    // samml [config.json] mods
    // samml [config.json] <show|deploy|launch|run> <profile_slug>
    let (config_path, command, argument) = match args.len() {
        2 if matches!(args[1].as_str(), "init" | "clean" | "list" | "mods") => {
            (None, args[1].clone(), None)
        }
        3 if matches!(args[2].as_str(), "init" | "clean" | "list" | "mods") => {
            (Some(PathBuf::from(&args[1])), args[2].clone(), None)
        }
        3 => (None, args[1].clone(), Some(args[2].clone())),
        4 => (
            Some(PathBuf::from(&args[1])),
            args[2].clone(),
            Some(args[3].clone()),
        ),
        _ => bail!(
            "usage: samml [config.json] init [game_dir]\n       samml [config.json] <show|deploy|launch|run> <profile_slug>\n       samml [config.json] new <profile name>\n       samml [config.json] list\n       samml [config.json] mods\n       samml [config.json] clean"
        ),
    };
    let profile_slug = argument;

    let dirs = PlatformDirs::from_env()?;
    if command == "init" {
        return init(
            &config_path.unwrap_or_else(|| dirs.config_file()),
            profile_slug.as_deref(),
            &dirs,
        );
    }
    let config_path = config_path.unwrap_or_else(|| dirs.config_file());
    if !config_path.exists() {
        bail!(
            "no config found at {}; run `samml init` to set up",
            config_path.display()
        );
    }

    let manager = Manager::open(&config_path).with_context(|| {
        format!(
            "could not open {}; fix it, or run `samml init` to set up again",
            config_path.display()
        )
    })?;

    match command.as_str() {
        "show" | "deploy" | "launch" | "run" => {
            let slug = profile_slug.as_deref().unwrap();
            print_profile(&manager.profile(slug)?);

            match command.as_str() {
                "show" => print_plan(&manager.plan(slug)?),
                "deploy" => deploy(&manager, slug)?,
                "launch" => launch(&manager)?,
                "run" => {
                    deploy(&manager, slug)?;
                    launch(&manager)?;
                }
                _ => unreachable!(),
            }
        }
        "new" => {
            let name = profile_slug
                .as_deref()
                .context("usage: samml [config.json] new <profile name>")?;
            let profile = manager.create_profile(name)?;
            println!("Created profile '{}' ({})", profile.name, profile.slug);
        }
        "list" => {
            let profiles = manager.list_profiles()?;
            if profiles.is_empty() {
                println!(
                    "No profiles found in {}",
                    manager.config().profiles_dir().display()
                );
            }
            for p in profiles {
                println!("{} - {}", p.slug, p.name);
            }
        }
        "mods" => {
            let mods = manager.list_mods()?;
            if mods.is_empty() {
                println!(
                    "No mods found in {}",
                    manager.config().staging_dir().display()
                );
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
            manager.clean()?;
            println!("Restored to vanilla.");
        }
        other => bail!("unknown command: {other}"),
    }

    Ok(())
}

fn deploy(manager: &Manager, slug: &str) -> Result<()> {
    manager.deploy(slug)?;
    println!("Deployed.");
    Ok(())
}

fn launch(manager: &Manager) -> Result<()> {
    manager.launch()?;
    println!(
        "Launched Steam App ID {}.",
        manager.config().steam_app_id.unwrap_or_default()
    );
    Ok(())
}

fn print_profile(profile: &ProfileView) {
    println!(
        "Profile '{}' ({}) - {} mod(s):",
        profile.name,
        profile.slug,
        profile.mods.len()
    );
    for m in &profile.mods {
        println!(
            "  [{}] {}{}",
            if m.active { "x" } else { " " },
            m.name.as_deref().unwrap_or(&m.id),
            m.problem
                .as_ref()
                .map(|p| format!("  (PROBLEM: {p})"))
                .unwrap_or_default()
        );
    }
}

fn print_plan(plan: &DeployPlan) {
    let links: usize = plan
        .mods
        .iter()
        .filter(|m| m.active)
        .map(|m| m.links.len())
        .sum();
    println!(
        "\nWould create {links} link(s). Game directory: {}",
        match &plan.state {
            DeployState::NotDeployed => "nothing deployed".to_string(),
            DeployState::OtherProfile { slug } => format!(
                "another profile is deployed ({})",
                slug.as_deref().unwrap_or("unknown")
            ),
            DeployState::UpToDate => "up to date".to_string(),
            DeployState::OutOfDate(d) => format!(
                "OUT OF DATE ({} to add, {} to remove, {} changed) - redeploy",
                d.missing.len(),
                d.stale.len(),
                d.changed.len()
            ),
        }
    );
    for c in &plan.conflicts {
        println!("  CONFLICT {} <- {}", c.path, c.mods.join(", "));
    }
    let replaced = plan
        .mods
        .iter()
        .flat_map(|m| &m.links)
        .filter(|l| l.replaces_game_file)
        .count();
    if replaced > 0 {
        println!("  {replaced} existing game file(s) will be backed up and replaced");
    }
    if !plan.can_deploy {
        println!("  Cannot deploy until the problems above are fixed.");
    }
}

/// First-run setup: find the game (or take it from the command line), pick
/// the standard folders, and save the config. If a config exists but its
/// paths no longer work (e.g. the game moved), only the game folder is
/// replaced.
fn init(path: &Path, game_dir: Option<&str>, dirs: &PlatformDirs) -> Result<()> {
    let suggestion = config::suggest_config(dirs);
    let samml_dir = match config::config_status(path) {
        ConfigStatus::Ready { .. } => {
            println!("Already set up: {}", path.display());
            return Ok(());
        }
        ConfigStatus::Broken { reason } => bail!("{reason}"),
        ConfigStatus::NeedsAttention { config, .. } => config.samml_dir,
        ConfigStatus::Missing => suggestion.samml_dir,
    };
    let game_dir = match game_dir {
        Some(dir) => PathBuf::from(dir),
        None => suggestion.game_dir.context(
            "could not find GTA San Andreas in any Steam library; run `samml init <game folder>`",
        )?,
    };
    let new_config = manifest::AppConfig {
        game_dir,
        samml_dir,
        steam_app_id: Some(suggestion.steam_app_id),
    };
    config::save_config(path, &new_config)?;
    config::ensure_dirs(&new_config)?;
    println!("Saved {}", path.display());
    println!("  game:     {}", new_config.game_dir.display());
    println!("  samml:    {}", new_config.samml_dir.display());
    println!("            (mods in staging/, profiles in profiles/)");
    Ok(())
}
