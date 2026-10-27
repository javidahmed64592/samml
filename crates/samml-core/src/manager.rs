//! The one object the CLI and the UI talk to: every action a player can
//! take, as a single method.
//!
//! `Manager` owns the `AppConfig` and strings the other modules together
//! (load a profile, edit it, save it; plan, resolve, deploy; and so on), so
//! a Tauri command is a one-line call and the CLI is only argument parsing
//! and printing. It adds no rules of its own beyond what composing the
//! modules needs.
//!
//! Anything that changes the library, a profile, or the game directory runs
//! under one lock, so two UI actions at once (e.g. adding two mods to the
//! same profile) cannot overwrite each other's changes. Reads are not
//! locked, and copying files into a mod (`import_into_mod`) is not either,
//! so a long import never freezes the rest of the app.
//!
//! Deleting or renaming things never touches the game directory: a deployed
//! profile that is deleted stays deployed (see `deployed`) until `clean` or
//! the next deploy, and a renamed or deleted mod shows up as an out-of-date
//! deployment in the plan.

use crate::manifest::{
    AppConfig, ConflictPolicy, DeployPlan, DeployedInfo, FileEntry, FileRule, ImportPreview,
    ImportProgress, ImportReport, LibraryMod, Placement, Profile, ProfileInfo, ProfileView,
};
use crate::{
    browse, config, deploy, import, launch, library, plan, profile_mods, profiles, resolve, usage,
};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub struct Manager {
    config: AppConfig,
    /// Held while changing the library, a profile or the game directory.
    write_lock: Mutex<()>,
}

impl Manager {
    pub fn new(config: AppConfig) -> Self {
        Manager {
            config,
            write_lock: Mutex::new(()),
        }
    }

    /// Opens the config file, refusing one whose paths are not usable (use
    /// `config::config_status` first to show setup), and makes sure the
    /// SAMML folder exists.
    pub fn open(config_path: &Path) -> Result<Self> {
        let loaded = config::load_config(config_path)?;
        let problems = config::check_config(&loaded);
        if !problems.is_empty() {
            let messages: Vec<_> = problems
                .iter()
                .map(|p| format!("{:?}: {}", p.field, p.message))
                .collect();
            bail!(
                "the config at {} is not usable: {}",
                config_path.display(),
                messages.join("; ")
            );
        }
        config::ensure_dirs(&loaded)?;
        Ok(Self::new(loaded))
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    fn lock(&self) -> MutexGuard<'_, ()> {
        // A panic in another action must not wedge the app: the files it
        // was writing are replaced atomically, so the state is still sound.
        self.write_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // --- Profiles ---------------------------------------------------------

    pub fn list_profiles(&self) -> Result<Vec<ProfileInfo>> {
        profiles::list_profiles(&self.config.profiles_dir())
    }

    fn load_profile(&self, slug: &str) -> Result<Profile> {
        profiles::find_profile_by_slug(&self.config.profiles_dir(), slug)?
            .with_context(|| format!("profile '{slug}' not found"))
    }

    fn info_for(&self, slug: String) -> Result<ProfileInfo> {
        let name = self.load_profile(&slug)?.name;
        Ok(ProfileInfo { slug, name })
    }

    /// A profile with each entry's mod name and any problem.
    pub fn profile(&self, slug: &str) -> Result<ProfileView> {
        let profile = self.load_profile(slug)?;
        let mods = resolve::describe_profile(&self.config.staging_dir(), &profile)?;
        Ok(ProfileView {
            slug: slug.to_string(),
            name: profile.name,
            mods,
        })
    }

    pub fn create_profile(&self, name: &str) -> Result<ProfileInfo> {
        let _guard = self.lock();
        let (slug, _) = profiles::create_profile(&self.config.profiles_dir(), name)?;
        self.info_for(slug)
    }

    pub fn rename_profile(&self, slug: &str, new_name: &str) -> Result<()> {
        let _guard = self.lock();
        profiles::rename_profile(&self.config.profiles_dir(), slug, new_name)
    }

    pub fn duplicate_profile(&self, slug: &str, new_name: &str) -> Result<ProfileInfo> {
        let _guard = self.lock();
        let (new_slug, _) =
            profiles::duplicate_profile(&self.config.profiles_dir(), slug, new_name)?;
        self.info_for(new_slug)
    }

    /// Deletes the profile file only. If it is the deployed profile, the
    /// game directory is left as it is (see `deployed`).
    pub fn delete_profile(&self, slug: &str) -> Result<()> {
        let _guard = self.lock();
        profiles::delete_profile(&self.config.profiles_dir(), slug)
    }

    /// Loads the profile, applies `edit`, and saves it, all under the lock.
    fn edit_profile(
        &self,
        slug: &str,
        edit: impl FnOnce(&mut Profile) -> Result<()>,
    ) -> Result<()> {
        let _guard = self.lock();
        let mut profile = self.load_profile(slug)?;
        edit(&mut profile)?;
        profiles::save_profile(&self.config.profiles_dir(), slug, &profile)
    }

    /// Adds a library mod to the end of the profile (active).
    pub fn add_mod_to_profile(&self, slug: &str, id: &str) -> Result<()> {
        let staging = self.config.staging_dir();
        self.edit_profile(slug, |profile| {
            library::entry(&library::load_library(&staging)?, id)?;
            profile_mods::add_mod(profile, id)
        })
    }

    /// Takes the mod out of the profile; the mod stays in the library.
    pub fn remove_mod_from_profile(&self, slug: &str, id: &str) -> Result<()> {
        self.edit_profile(slug, |profile| {
            profile_mods::remove_mod(profile, id).map(|_| ())
        })
    }

    pub fn set_profile_mod_active(&self, slug: &str, id: &str, active: bool) -> Result<()> {
        self.edit_profile(slug, |profile| {
            profile_mods::set_mod_active(profile, id, active)
        })
    }

    pub fn move_profile_mod(&self, slug: &str, id: &str, new_index: usize) -> Result<()> {
        self.edit_profile(slug, |profile| {
            profile_mods::move_mod(profile, id, new_index)
        })
    }

    // --- Library ----------------------------------------------------------

    /// Every mod in the library. Folders dropped into staging get their id
    /// here.
    pub fn list_mods(&self) -> Result<Vec<LibraryMod>> {
        let _guard = self.lock();
        library::list_mods(&self.config.staging_dir())
    }

    /// The profiles that use the mod, e.g. to show before deleting it.
    pub fn profiles_using_mod(&self, id: &str) -> Result<Vec<ProfileInfo>> {
        usage::profiles_using_mod(&self.config.profiles_dir(), id)
    }

    /// Creates a mod named `name` with the given deploy rules and copies
    /// `sources` into it. If the copy fails, the new mod is removed again.
    /// Returns the new mod's id.
    pub fn create_mod(
        &self,
        name: &str,
        files: Option<Vec<FileRule>>,
        sources: &[PathBuf],
        placement: Placement,
        progress: impl FnMut(&ImportProgress),
    ) -> Result<String> {
        let staging = self.config.staging_dir();
        let (id, _) = {
            let _guard = self.lock();
            library::add_mod(&staging, name, files)?
        };
        let imported = import::import_into_mod(
            &staging,
            &id,
            sources,
            placement,
            ConflictPolicy::Fail,
            progress,
        );
        if let Err(err) = imported {
            let _guard = self.lock();
            let _ = library::delete_mod(&staging, &id);
            return Err(err);
        }
        Ok(id)
    }

    /// Renames the mod. Profiles are unaffected (they use ids); the game
    /// folder follows on the next deploy.
    pub fn rename_mod(&self, id: &str, new_name: &str) -> Result<()> {
        let _guard = self.lock();
        library::rename_mod(&self.config.staging_dir(), id, new_name)
    }

    pub fn set_mod_files(&self, id: &str, files: Option<Vec<FileRule>>) -> Result<()> {
        let _guard = self.lock();
        library::set_mod_files(&self.config.staging_dir(), id, files)
    }

    /// Deletes the mod's files and removes it from every profile that uses
    /// it, returning those profiles.
    pub fn delete_mod(&self, id: &str) -> Result<Vec<ProfileInfo>> {
        let _guard = self.lock();
        usage::delete_mod_everywhere(&self.config, id)
    }

    pub fn list_mod_files(&self, id: &str, relative: &str) -> Result<Vec<FileEntry>> {
        browse::list_mod_files(&self.config.staging_dir(), id, relative)
    }

    pub fn preview_import(
        &self,
        id: &str,
        sources: &[PathBuf],
        placement: Placement,
    ) -> Result<ImportPreview> {
        import::preview_import(&self.config.staging_dir(), id, sources, placement)
    }

    /// Copies `sources` into an existing mod. Not locked, so a long copy
    /// does not block other actions.
    pub fn import_into_mod(
        &self,
        id: &str,
        sources: &[PathBuf],
        placement: Placement,
        policy: ConflictPolicy,
        progress: impl FnMut(&ImportProgress),
    ) -> Result<ImportReport> {
        import::import_into_mod(
            &self.config.staging_dir(),
            id,
            sources,
            placement,
            policy,
            progress,
        )
    }

    // --- Deploying --------------------------------------------------------

    /// What deploying the profile would do, and whether the game directory
    /// already matches.
    pub fn plan(&self, slug: &str) -> Result<DeployPlan> {
        let profile = self.load_profile(slug)?;
        plan::plan_deploy(&self.config, slug, &profile)
    }

    /// Deploys the profile, replacing whatever is deployed. All or nothing.
    pub fn deploy(&self, slug: &str) -> Result<()> {
        let _guard = self.lock();
        let profile = self.load_profile(slug)?;
        let mods = resolve::resolve_profile(&self.config.staging_dir(), &profile)?;
        deploy::deploy(&self.config, slug, &mods)
    }

    /// Removes everything deployed and restores backed-up game files.
    pub fn clean(&self) -> Result<()> {
        let _guard = self.lock();
        deploy::clean(&self.config)
    }

    /// The profile deployed right now, if any.
    pub fn deployed(&self) -> Result<Option<DeployedInfo>> {
        let Some(slug) = deploy::deployed_profile(&self.config)? else {
            return Ok(None);
        };
        let name = profiles::find_profile_by_slug(&self.config.profiles_dir(), &slug)
            .ok()
            .flatten()
            .map(|p| p.name);
        Ok(Some(DeployedInfo { slug, name }))
    }

    /// Starts the game through Steam.
    pub fn launch(&self) -> Result<()> {
        launch::launch(&self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{DeployState, FileRule};
    use std::fs;
    use std::sync::Arc;
    use tempfile::TempDir;

    struct Env {
        root: TempDir,
        manager: Manager,
    }

    fn rule(source: &str, target: &str) -> FileRule {
        FileRule {
            source: source.into(),
            target: target.into(),
        }
    }

    impl Env {
        fn new() -> Self {
            let root = TempDir::new().unwrap();
            let game = root.path().join("Game");
            fs::create_dir_all(&game).unwrap();
            fs::write(game.join("gta-sa.exe"), "").unwrap();
            let config = AppConfig {
                game_dir: game,
                samml_dir: root.path().join("samml"),
                steam_app_id: Some(12120),
            };
            config::ensure_dirs(&config).unwrap();
            Env {
                root,
                manager: Manager::new(config),
            }
        }

        /// A folder of downloaded files to import from.
        fn download(&self, name: &str, files: &[&str]) -> PathBuf {
            let dir = self.root.path().join("downloads").join(name);
            for f in files {
                let path = dir.join(f);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, f).unwrap();
            }
            dir
        }

        fn create_mod(&self, name: &str, files: &[&str], rules: Option<Vec<FileRule>>) -> String {
            let source = self.download(name, files);
            self.manager
                .create_mod(name, rules, &[source], Placement::Contents, |_| {})
                .unwrap()
        }

        fn game(&self, rel: &str) -> PathBuf {
            self.manager.config().game_dir.join(rel)
        }

        /// Modloader first, as a real profile has it.
        fn profile_with_modloader(&self, name: &str) -> (String, String) {
            let modloader = self.create_mod(
                "Modloader",
                &["modloader/.data/x", "modloader.asi"],
                Some(vec![
                    rule("modloader", "modloader"),
                    rule("modloader.asi", "modloader.asi"),
                ]),
            );
            let profile = self.manager.create_profile(name).unwrap();
            self.manager
                .add_mod_to_profile(&profile.slug, &modloader)
                .unwrap();
            (profile.slug, modloader)
        }
    }

    // --- opening ---

    #[test]
    fn open_refuses_an_unusable_config_and_creates_the_folders_for_a_good_one() {
        let env = Env::new();
        let path = env.root.path().join("cfg/config.json");
        let mut config = env.manager.config().clone();
        config.game_dir = env.root.path().join("nope");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serde_json::to_string(&config).unwrap()).unwrap();
        let err = Manager::open(&path).err().unwrap();
        assert!(err.to_string().contains("not usable"), "{err}");

        config.game_dir = env.manager.config().game_dir.clone();
        config.samml_dir = env.root.path().join("fresh");
        config::save_config(&path, &config).unwrap();
        let manager = Manager::open(&path).unwrap();
        assert!(manager.config().staging_dir().is_dir());
        assert!(manager.config().profiles_dir().is_dir());
    }

    // --- profiles ---

    #[test]
    fn profile_lifecycle_create_rename_duplicate_delete() {
        let env = Env::new();
        let m = &env.manager;

        let created = m.create_profile("  My Profile ").unwrap();
        assert_eq!(
            (created.slug.as_str(), created.name.as_str()),
            ("my_profile", "My Profile")
        );

        m.rename_profile(&created.slug, "Renamed").unwrap();
        let copy = m.duplicate_profile(&created.slug, "Copy").unwrap();
        let names: Vec<_> = m
            .list_profiles()
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["Copy", "Renamed"]);

        m.delete_profile(&copy.slug).unwrap();
        assert_eq!(m.list_profiles().unwrap().len(), 1);
        assert!(m.profile(&copy.slug).is_err());
        assert!(m.profile("../escape").is_err());
    }

    #[test]
    fn editing_a_profile_adds_toggles_moves_and_removes_mods() {
        let env = Env::new();
        let m = &env.manager;
        let a = env.create_mod("A", &["f"], None);
        let b = env.create_mod("B", &["f"], None);
        let p = m.create_profile("P").unwrap().slug;

        m.add_mod_to_profile(&p, &a).unwrap();
        m.add_mod_to_profile(&p, &b).unwrap();
        m.move_profile_mod(&p, &b, 0).unwrap();
        m.set_profile_mod_active(&p, &a, false).unwrap();

        let view = m.profile(&p).unwrap();
        let shown: Vec<_> = view
            .mods
            .iter()
            .map(|v| (v.name.as_deref().unwrap(), v.active))
            .collect();
        assert_eq!(shown, [("B", true), ("A", false)]);

        m.remove_mod_from_profile(&p, &a).unwrap();
        assert_eq!(m.profile(&p).unwrap().mods.len(), 1);
        assert_eq!(m.list_mods().unwrap().len(), 2, "the library keeps it");
    }

    #[test]
    fn only_library_mods_can_be_added_to_a_profile() {
        let env = Env::new();
        let p = env.manager.create_profile("P").unwrap().slug;

        let err = env
            .manager
            .add_mod_to_profile(&p, "ghost-0000")
            .unwrap_err();

        assert!(err.to_string().contains("no mod with id"), "{err}");
        assert!(env.manager.profile(&p).unwrap().mods.is_empty());
    }

    #[test]
    fn concurrent_edits_to_one_profile_do_not_lose_updates() {
        let env = Env::new();
        let ids: Vec<String> = (0..16)
            .map(|i| env.create_mod(&format!("Mod {i}"), &["f"], None))
            .collect();
        let slug = env.manager.create_profile("P").unwrap().slug;
        let manager = Arc::new(env.manager);

        let handles: Vec<_> = ids
            .iter()
            .map(|id| {
                let (manager, slug, id) = (Arc::clone(&manager), slug.clone(), id.clone());
                std::thread::spawn(move || manager.add_mod_to_profile(&slug, &id).unwrap())
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(manager.profile(&slug).unwrap().mods.len(), 16);
    }

    // --- library ---

    #[test]
    fn create_mod_copies_the_files_and_applies_the_rules() {
        let env = Env::new();
        let source = env.download("pack", &["a.txt", "dir/b.txt"]);
        let mut seen = Vec::new();

        let id = env
            .manager
            .create_mod(
                "Cool Pack",
                Some(vec![rule("a.txt", "a.txt")]),
                &[source],
                Placement::Contents,
                |p| seen.push(p.files_done),
            )
            .unwrap();

        let mods = env.manager.list_mods().unwrap();
        assert_eq!(
            (mods[0].id.as_str(), mods[0].name.as_str()),
            (id.as_str(), "Cool Pack")
        );
        assert_eq!(mods[0].files.as_ref().unwrap().len(), 1);
        let files = env.manager.list_mod_files(&id, "").unwrap();
        let names: Vec<_> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["dir", "a.txt"]);
        assert_eq!(seen, [1, 2]);
    }

    #[test]
    fn a_failed_create_mod_leaves_no_trace() {
        let env = Env::new();
        let source = env.download("pack", &["a.txt"]);
        std::os::unix::fs::symlink(source.join("a.txt"), source.join("link")).unwrap();

        let err = env
            .manager
            .create_mod("Broken", None, &[source], Placement::Contents, |_| {})
            .unwrap_err();

        assert!(err.to_string().contains("symbolic links"), "{err}");
        assert!(env.manager.list_mods().unwrap().is_empty());
        assert!(!env.manager.config().staging_dir().join("Broken").exists());
        // the name is free again
        env.create_mod("Broken", &["f"], None);
    }

    #[test]
    fn invalid_mod_names_are_refused_before_anything_is_copied() {
        let env = Env::new();
        let source = env.download("pack", &["a.txt"]);

        let err = env
            .manager
            .create_mod("a/b", None, &[source], Placement::Contents, |_| {})
            .unwrap_err();

        assert!(err.to_string().contains("cannot contain '/'"), "{err}");
        assert!(env.manager.list_mods().unwrap().is_empty());
    }

    #[test]
    fn importing_more_files_into_an_existing_mod_previews_conflicts_first() {
        let env = Env::new();
        let id = env.create_mod("M", &["a.txt"], None);
        let more = env.download("more", &["a.txt", "b.txt"]);

        let preview = env
            .manager
            .preview_import(&id, std::slice::from_ref(&more), Placement::Contents)
            .unwrap();
        assert_eq!(preview.conflicts, ["a.txt"]);
        assert!(
            env.manager
                .import_into_mod(
                    &id,
                    std::slice::from_ref(&more),
                    Placement::Contents,
                    ConflictPolicy::Fail,
                    |_| {}
                )
                .is_err()
        );

        let report = env
            .manager
            .import_into_mod(
                &id,
                &[more],
                Placement::Contents,
                ConflictPolicy::Overwrite,
                |_| {},
            )
            .unwrap();
        assert_eq!(report.overwritten, ["a.txt"]);
    }

    #[test]
    fn deleting_a_mod_reports_and_updates_the_profiles_that_used_it() {
        let env = Env::new();
        let a = env.create_mod("A", &["f"], None);
        let p = env.manager.create_profile("P").unwrap().slug;
        env.manager.add_mod_to_profile(&p, &a).unwrap();
        assert_eq!(env.manager.profiles_using_mod(&a).unwrap().len(), 1);

        let affected = env.manager.delete_mod(&a).unwrap();

        assert_eq!(affected[0].slug, p);
        assert!(env.manager.profile(&p).unwrap().mods.is_empty());
        assert!(env.manager.list_mods().unwrap().is_empty());
    }

    // --- deploying ---

    #[test]
    fn deploy_flow_plan_deploy_rename_redeploy_clean() {
        let env = Env::new();
        let m = &env.manager;
        let (p, _) = env.profile_with_modloader("P");
        let cars = env.create_mod("Old Name", &["car.dff"], None);
        m.add_mod_to_profile(&p, &cars).unwrap();

        let plan = m.plan(&p).unwrap();
        assert!(plan.can_deploy);
        assert_eq!(plan.state, DeployState::NotDeployed);
        assert!(m.deployed().unwrap().is_none());

        m.deploy(&p).unwrap();
        assert_eq!(m.plan(&p).unwrap().state, DeployState::UpToDate);
        assert!(env.game("modloader/Old Name").is_symlink());
        let deployed = m.deployed().unwrap().unwrap();
        assert_eq!(
            (deployed.slug.as_str(), deployed.name.as_deref()),
            (p.as_str(), Some("P"))
        );

        m.rename_mod(&cars, "New Name").unwrap();
        assert!(matches!(
            m.plan(&p).unwrap().state,
            DeployState::OutOfDate(_)
        ));
        m.deploy(&p).unwrap();
        assert_eq!(m.plan(&p).unwrap().state, DeployState::UpToDate);
        assert!(env.game("modloader/New Name").is_symlink());
        assert!(!env.game("modloader/Old Name").exists());

        m.clean().unwrap();
        assert!(!env.game("modloader").exists());
        assert!(m.deployed().unwrap().is_none());
    }

    #[test]
    fn deploying_a_profile_with_problems_is_refused_and_changes_nothing() {
        let env = Env::new();
        let (p, _) = env.profile_with_modloader("P");
        let gone = env.create_mod("Gone", &["f"], None);
        env.manager.add_mod_to_profile(&p, &gone).unwrap();
        fs::remove_dir_all(env.manager.config().staging_dir().join("Gone")).unwrap();

        let err = env.manager.deploy(&p).unwrap_err();

        assert!(err.to_string().contains("Gone"), "{err}");
        assert!(!env.manager.plan(&p).unwrap().can_deploy);
        assert!(env.manager.deployed().unwrap().is_none());
        assert!(!env.game("modloader").exists());
    }

    #[test]
    fn deleting_the_deployed_profile_leaves_the_game_alone_until_clean() {
        let env = Env::new();
        let m = &env.manager;
        let (p, _) = env.profile_with_modloader("P");
        m.deploy(&p).unwrap();

        m.delete_profile(&p).unwrap();

        let deployed = m.deployed().unwrap().unwrap();
        assert_eq!((deployed.slug.as_str(), deployed.name), (p.as_str(), None));
        assert!(env.game("modloader").is_symlink(), "still deployed");
        m.clean().unwrap();
        assert!(m.deployed().unwrap().is_none());
        assert!(!env.game("modloader").exists());
    }

    #[test]
    fn deleting_a_deployed_mod_leaves_dangling_links_that_redeploying_clears() {
        let env = Env::new();
        let m = &env.manager;
        let (p, _) = env.profile_with_modloader("P");
        let cars = env.create_mod("Cars", &["car.dff"], None);
        m.add_mod_to_profile(&p, &cars).unwrap();
        m.deploy(&p).unwrap();

        m.delete_mod(&cars).unwrap();

        let DeployState::OutOfDate(drift) = m.plan(&p).unwrap().state else {
            panic!("expected out of date");
        };
        assert_eq!(drift.stale, ["modloader/Cars"]);
        m.deploy(&p).unwrap();
        assert_eq!(m.plan(&p).unwrap().state, DeployState::UpToDate);
        assert!(env.game("modloader/Cars").symlink_metadata().is_err());
    }

    #[test]
    fn another_profile_deployed_is_reported_and_replaced_by_deploying() {
        let env = Env::new();
        let m = &env.manager;
        let (first, modloader) = env.profile_with_modloader("First");
        let second = m.create_profile("Second").unwrap().slug;
        m.add_mod_to_profile(&second, &modloader).unwrap();
        m.deploy(&first).unwrap();

        assert_eq!(
            m.plan(&second).unwrap().state,
            DeployState::OtherProfile {
                slug: Some(first.clone())
            }
        );
        m.deploy(&second).unwrap();
        assert_eq!(m.deployed().unwrap().unwrap().slug, second);
    }
}
