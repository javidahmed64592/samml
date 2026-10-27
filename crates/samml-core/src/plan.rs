//! What deploying a profile would do, worked out without changing anything:
//! every mod's links, the conflicts between mods, and whether the game
//! directory already matches.
//!
//! Joins `resolve` (names and problems), `links` (which symlinks each mod
//! needs) and `receipt` (what is deployed now). Read-only: the UI shows the
//! plan, and `deploy` then does it.

use crate::links::{self, LinkPair, MODLOADER_DIR};
use crate::manifest::{
    AppConfig, DeployDrift, DeployMod, DeployPlan, DeployState, PlanConflict, PlannedLink,
    PlannedMod, Profile, ProfileModView,
};
use crate::receipt::{self, Receipt};
use crate::{library, resolve};
use anyhow::Result;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

fn relative(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// True if a file or folder that deploy did not create is at `rel`.
fn replaces_game_file(game_dir: &Path, rel: &str, receipt: Option<&Receipt>) -> bool {
    let Ok(meta) = fs::symlink_metadata(game_dir.join(rel)) else {
        return false;
    };
    let ours = receipt.is_some_and(|r| r.created_dirs.iter().any(|d| d == rel));
    !meta.file_type().is_symlink() && !ours
}

/// Whether `modloader/` would exist before the first mod is deployed: it
/// already exists in the game directory and is not something a previous
/// deploy made (that would be removed first).
fn modloader_in_vanilla_game(game_dir: &Path, receipt: Option<&Receipt>) -> bool {
    let made_by_us = receipt.is_some_and(|r| {
        r.links.iter().any(|l| l == MODLOADER_DIR)
            || r.created_dirs.iter().any(|d| d == MODLOADER_DIR)
    });
    game_dir.join(MODLOADER_DIR).exists() && !made_by_us
}

fn touches_modloader(rel_link: &str) -> bool {
    Path::new(rel_link)
        .components()
        .next()
        .is_some_and(|c| c.as_os_str() == MODLOADER_DIR)
}

fn describe_links(
    pairs: Vec<LinkPair>,
    app: &AppConfig,
    receipt: Option<&Receipt>,
) -> Vec<PlannedLink> {
    pairs
        .into_iter()
        .map(|(link, target)| {
            let path = relative(&app.game_dir, &link);
            PlannedLink {
                replaces_game_file: replaces_game_file(&app.game_dir, &path, receipt),
                path,
                source: relative(&app.staging_dir(), &target),
            }
        })
        .collect()
}

/// One profile entry's links, or the reason it has none.
fn plan_mod(
    view: &ProfileModView,
    rules: Option<&DeployMod>,
    app: &AppConfig,
    receipt: Option<&Receipt>,
) -> PlannedMod {
    let planned = |links, problem| PlannedMod {
        id: view.id.clone(),
        name: view.name.clone(),
        active: view.active,
        links,
        problem,
    };
    if let Some(problem) = &view.problem {
        return planned(Vec::new(), Some(problem.clone()));
    }
    let Some(m) = rules else {
        return planned(Vec::new(), Some("it is not in the library".to_string()));
    };
    match links::mod_links(m, &app.staging_dir(), &app.game_dir) {
        Ok(pairs) => planned(describe_links(pairs, app, receipt), None),
        Err(err) => planned(Vec::new(), Some(format!("{err:#}"))),
    }
}

/// Mods without file rules (`needs_modloader`, one flag per entry) need
/// `modloader/` to exist by the time they are linked. Mirrors the check
/// `deploy` makes, so the plan can report it before anything is touched.
fn check_modloader_order(
    mods: &mut [PlannedMod],
    needs_modloader: &[bool],
    app: &AppConfig,
    receipt: Option<&Receipt>,
) {
    let mut available = modloader_in_vanilla_game(&app.game_dir, receipt);
    for (m, needs) in mods.iter_mut().zip(needs_modloader) {
        if !m.active || m.problem.is_some() {
            continue;
        }
        if *needs && !available {
            let name = m.name.as_deref().unwrap_or(&m.id);
            m.problem = Some(links::modloader_missing_message(name));
            m.links.clear();
            continue;
        }
        if m.links.iter().any(|l| touches_modloader(&l.path)) {
            available = true;
        }
    }
}

fn find_conflicts(mods: &[PlannedMod]) -> Vec<PlanConflict> {
    let mut writers: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for m in mods.iter().filter(|m| m.active) {
        for link in &m.links {
            writers
                .entry(&link.path)
                .or_default()
                .push(m.name.clone().unwrap_or_else(|| m.id.clone()));
        }
    }
    writers
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(path, mods)| PlanConflict {
            path: path.to_string(),
            mods,
        })
        .collect()
}

fn points_at(game_dir: &Path, rel: &str, expected: &Path) -> bool {
    fs::read_link(game_dir.join(rel)).is_ok_and(|actual| actual == expected)
}

/// Compares what is deployed (the receipt and the links on disk) with the
/// links the profile's active mods need.
fn deploy_state(
    app: &AppConfig,
    profile_slug: &str,
    mods: &[PlannedMod],
    receipt: Option<&Receipt>,
) -> DeployState {
    let Some(receipt) = receipt else {
        return DeployState::NotDeployed;
    };
    if receipt.profile.as_deref() != Some(profile_slug) {
        return DeployState::OtherProfile {
            slug: receipt.profile.clone(),
        };
    }

    let deployed: HashSet<&str> = receipt.links.iter().map(String::as_str).collect();
    let mut wanted: HashMap<&str, PathBuf> = HashMap::new();
    let mut order = Vec::new();
    for link in mods.iter().filter(|m| m.active).flat_map(|m| &m.links) {
        if wanted
            .insert(&link.path, app.staging_dir().join(&link.source))
            .is_none()
        {
            order.push(link.path.as_str());
        }
    }

    let mut drift = DeployDrift::default();
    for path in order {
        if !deployed.contains(path) {
            drift.missing.push(path.to_string());
        } else if !points_at(&app.game_dir, path, &wanted[path]) {
            drift.changed.push(path.to_string());
        }
    }
    drift.stale = receipt
        .links
        .iter()
        .filter(|l| !wanted.contains_key(l.as_str()))
        .cloned()
        .collect();

    if drift == DeployDrift::default() {
        DeployState::UpToDate
    } else {
        DeployState::OutOfDate(drift)
    }
}

/// What deploying `profile` (whose slug is `profile_slug`) would do.
pub fn plan_deploy(app: &AppConfig, profile_slug: &str, profile: &Profile) -> Result<DeployPlan> {
    let views = resolve::describe_profile(&app.staging_dir(), profile)?;
    let library = library::load_library(&app.staging_dir())?;
    let receipt = receipt::read(&app.game_dir)?;

    let mut mods: Vec<PlannedMod> = views
        .iter()
        .map(|view| {
            let rules = library.mods.get(&view.id).map(|s| DeployMod {
                name: s.name.clone(),
                files: s.files.clone(),
            });
            plan_mod(view, rules.as_ref(), app, receipt.as_ref())
        })
        .collect();
    let needs_modloader: Vec<bool> = views
        .iter()
        .map(|v| library.mods.get(&v.id).is_some_and(|s| s.files.is_none()))
        .collect();
    check_modloader_order(&mut mods, &needs_modloader, app, receipt.as_ref());

    let conflicts = find_conflicts(&mods);
    let state = deploy_state(app, profile_slug, &mods, receipt.as_ref());
    let can_deploy = mods
        .iter()
        .filter(|m| m.active)
        .all(|m| m.problem.is_none());
    Ok(DeployPlan {
        mods,
        conflicts,
        state,
        can_deploy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{FileRule, ProfileMod};
    use crate::{deploy, profile_mods};
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    struct Env {
        _root: TempDir,
        app: AppConfig,
        profile: Profile,
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
            let app = AppConfig {
                game_dir: root.path().join("game"),
                samml_dir: root.path().join("samml"),
                steam_app_id: None,
            };
            fs::create_dir_all(&app.game_dir).unwrap();
            Env {
                _root: root,
                app,
                profile: Profile {
                    name: "P".into(),
                    mods: Vec::new(),
                },
            }
        }

        /// Adds a mod to the library (with the given files, each holding
        /// its own path as contents) and to the profile. Returns its id.
        fn add(&mut self, name: &str, files: &[&str], rules: Option<Vec<FileRule>>) -> String {
            let (id, dir) = library::add_mod(&self.app.staging_dir(), name, rules).unwrap();
            for f in files {
                let path = dir.join(f);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, f).unwrap();
            }
            profile_mods::add_mod(&mut self.profile, &id).unwrap();
            id
        }

        /// A Modloader mod whose rules create `modloader/`, as in real use.
        fn add_modloader(&mut self) -> String {
            self.add(
                "Modloader",
                &["modloader/.data/x", "modloader.asi"],
                Some(vec![
                    rule("modloader", "modloader"),
                    rule("modloader.asi", "modloader.asi"),
                ]),
            )
        }

        fn plan(&self) -> DeployPlan {
            plan_deploy(&self.app, "p", &self.profile).unwrap()
        }

        fn deploy(&self) {
            let mods = resolve::resolve_profile(&self.app.staging_dir(), &self.profile).unwrap();
            deploy::deploy(&self.app, "p", &mods).unwrap();
        }

        fn write_game(&self, rel: &str, contents: &str) {
            let path = self.app.game_dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    fn link_paths(m: &PlannedMod) -> Vec<&str> {
        m.links.iter().map(|l| l.path.as_str()).collect()
    }

    // --- links ---

    #[test]
    fn plans_each_mods_links_in_profile_order() {
        let mut env = Env::new();
        env.add_modloader();
        env.add("Cool Cars", &["car.dff"], None);
        env.add(
            "Ruled",
            &["a.txt", "dir/b.txt"],
            Some(vec![rule("a.txt", "x/"), rule("dir/", "merged/")]),
        );

        let plan = env.plan();

        assert!(plan.can_deploy);
        assert_eq!(link_paths(&plan.mods[0]), ["modloader", "modloader.asi"]);
        assert_eq!(link_paths(&plan.mods[1]), ["modloader/Cool Cars"]);
        assert_eq!(plan.mods[1].links[0].source, "Cool Cars");
        assert_eq!(link_paths(&plan.mods[2]), ["x/a.txt", "merged/b.txt"]);
        assert_eq!(plan.mods[2].links[1].source, "Ruled/dir/b.txt");
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn the_plan_matches_what_deploy_really_links() {
        let mut env = Env::new();
        env.add_modloader();
        env.add("Cool Cars", &["car.dff"], None);
        env.add(
            "Ruled",
            &["a.txt", "dir/b.txt", "dir/c.txt"],
            Some(vec![rule("a.txt", "x/"), rule("dir/", "merged/")]),
        );
        let plan = env.plan();

        env.deploy();

        let mut planned: Vec<_> = plan
            .mods
            .iter()
            .flat_map(|m| m.links.iter().map(|l| l.path.clone()))
            .collect();
        let mut deployed = receipt::read(&env.app.game_dir).unwrap().unwrap().links;
        planned.sort();
        deployed.sort();
        assert_eq!(planned, deployed);
    }

    #[test]
    fn inactive_mods_are_planned_but_do_not_count() {
        let mut env = Env::new();
        env.add_modloader();
        let off = env.add("Off", &["f.txt"], Some(vec![rule("f.txt", "f.txt")]));
        env.add("Other", &["f.txt"], Some(vec![rule("f.txt", "f.txt")]));
        profile_mods::set_mod_active(&mut env.profile, &off, false).unwrap();

        let plan = env.plan();

        assert!(!plan.mods[1].active);
        assert_eq!(link_paths(&plan.mods[1]), ["f.txt"], "still shown");
        assert!(plan.conflicts.is_empty(), "but not a conflict");
    }

    #[test]
    fn links_that_replace_game_files_are_flagged() {
        let mut env = Env::new();
        env.write_game("data/stock.dat", "original");
        env.add(
            "M",
            &["stock.dat", "new.dat"],
            Some(vec![
                rule("stock.dat", "data/stock.dat"),
                rule("new.dat", "data/new.dat"),
            ]),
        );

        let plan = env.plan();

        let flags: Vec<_> = plan.mods[0]
            .links
            .iter()
            .map(|l| l.replaces_game_file)
            .collect();
        assert_eq!(flags, [true, false]);
    }

    #[test]
    fn our_own_links_and_directories_are_not_stock_files() {
        let mut env = Env::new();
        env.add("M", &["a.txt"], Some(vec![rule("a.txt", "new_dir/a.txt")]));
        env.deploy();

        let plan = env.plan();

        assert!(!plan.mods[0].links[0].replaces_game_file);
    }

    // --- modloader ordering ---

    #[test]
    fn default_mods_need_modloader_earlier_in_the_profile() {
        let mut env = Env::new();
        env.add("Early", &["f"], None);
        env.add_modloader();
        env.add("Late", &["f"], None);

        let plan = env.plan();

        let problem = plan.mods[0].problem.as_ref().unwrap();
        assert!(problem.contains("modloader/ does not exist"), "{problem}");
        assert!(plan.mods[0].links.is_empty());
        assert!(plan.mods[2].problem.is_none());
        assert!(!plan.can_deploy);
    }

    #[test]
    fn a_rule_placing_files_in_modloader_also_creates_it() {
        let mut env = Env::new();
        env.add(
            "Ragdoll",
            &["CLEO/script.cs"],
            Some(vec![rule("CLEO/", "modloader/Ragdoll/")]),
        );
        env.add("After", &["f"], None);

        assert!(env.plan().can_deploy);
        env.deploy();
    }

    #[test]
    fn a_modloader_left_by_an_earlier_deploy_does_not_count() {
        let mut env = Env::new();
        env.add_modloader();
        env.add("Cool Cars", &["f"], None);
        env.deploy();
        // reorder so the mod comes first: the old deployment is removed
        // before deploying, so modloader/ will not be there for it
        let second = env.profile.mods[1].id.clone();
        profile_mods::move_mod(&mut env.profile, &second, 0).unwrap();

        let plan = env.plan();

        assert!(plan.mods[0].problem.is_some());
        assert!(env.app.game_dir.join("modloader").exists());
    }

    #[test]
    fn a_modloader_folder_that_was_already_in_the_game_counts() {
        let mut env = Env::new();
        fs::create_dir(env.app.game_dir.join("modloader")).unwrap();
        env.add("Cool Cars", &["f"], None);

        assert!(env.plan().can_deploy);
    }

    // --- conflicts ---

    #[test]
    fn two_mods_writing_the_same_path_are_a_conflict() {
        let mut env = Env::new();
        env.add("First", &["x.dat"], Some(vec![rule("x.dat", "data/x.dat")]));
        env.add(
            "Second",
            &["y.dat"],
            Some(vec![rule("y.dat", "data/x.dat")]),
        );
        env.add("Third", &["z.dat"], Some(vec![rule("z.dat", "data/z.dat")]));

        let plan = env.plan();

        assert_eq!(
            plan.conflicts,
            [PlanConflict {
                path: "data/x.dat".into(),
                mods: vec!["First".into(), "Second".into()],
            }]
        );
        assert!(plan.can_deploy, "conflicts warn, they do not block");
    }

    #[test]
    fn placing_files_inside_another_mods_folder_link_is_not_a_conflict() {
        let mut env = Env::new();
        env.add_modloader();
        env.add(
            "Ragdoll",
            &["CLEO/s.cs"],
            Some(vec![rule("CLEO/", "modloader/Ragdoll/")]),
        );
        env.add("Cars", &["f"], None);

        assert!(env.plan().conflicts.is_empty());
    }

    // --- problems ---

    #[test]
    fn broken_mods_get_a_problem_and_the_rest_are_still_planned() {
        let mut env = Env::new();
        env.add_modloader();
        let bad = env.add("BadRule", &["a"], None);
        // a hand-edited library can hold rules the app would have refused
        let mut lib = library::load_library(&env.app.staging_dir()).unwrap();
        lib.mods.get_mut(&bad).unwrap().files = Some(vec![rule("a", "../../evil")]);
        fs::write(
            env.app.staging_dir().join(library::LIBRARY_FILE),
            serde_json::to_string(&lib).unwrap(),
        )
        .unwrap();
        env.add("NoSource", &["a"], Some(vec![rule("missing.txt", "x")]));
        let gone = env.add("Gone", &["a"], None);
        fs::remove_dir_all(env.app.staging_dir().join("Gone")).unwrap();
        profile_mods::add_mod(&mut env.profile, "unknown-0000").unwrap();
        env.add("Fine", &["a"], None);

        let plan = env.plan();

        let problems: Vec<_> = plan.mods.iter().map(|m| m.problem.is_some()).collect();
        assert_eq!(problems, [false, true, true, true, true, false]);
        assert!(
            plan.mods[1]
                .problem
                .as_ref()
                .unwrap()
                .contains("invalid file rules")
        );
        assert!(
            plan.mods[2]
                .problem
                .as_ref()
                .unwrap()
                .contains("source does not exist")
        );
        assert_eq!(plan.mods[3].id, gone);
        assert!(
            plan.mods[4]
                .problem
                .as_ref()
                .unwrap()
                .contains("not in the library")
        );
        assert!(!plan.can_deploy);
    }

    #[test]
    fn problems_in_inactive_mods_do_not_block_deploying() {
        let mut env = Env::new();
        env.add_modloader();
        env.profile.mods.push(ProfileMod {
            id: "unknown-0000".into(),
            active: false,
        });

        let plan = env.plan();

        assert!(plan.mods[1].problem.is_some());
        assert!(plan.can_deploy);
    }

    // --- deployed state ---

    #[test]
    fn state_follows_the_deployment_lifecycle() {
        let mut env = Env::new();
        env.add_modloader();
        env.add("Cool Cars", &["f"], None);
        assert_eq!(env.plan().state, DeployState::NotDeployed);

        env.deploy();
        assert_eq!(env.plan().state, DeployState::UpToDate);

        let other = plan_deploy(&env.app, "other", &env.profile).unwrap();
        assert_eq!(
            other.state,
            DeployState::OtherProfile {
                slug: Some("p".into())
            }
        );

        deploy::clean(&env.app).unwrap();
        assert_eq!(env.plan().state, DeployState::NotDeployed);
    }

    #[test]
    fn renaming_a_mod_makes_the_deployment_out_of_date() {
        let mut env = Env::new();
        env.add_modloader();
        let id = env.add("Old Name", &["f"], None);
        env.deploy();

        library::rename_mod(&env.app.staging_dir(), &id, "New Name").unwrap();
        let plan = env.plan();

        assert_eq!(
            plan.state,
            DeployState::OutOfDate(DeployDrift {
                missing: vec!["modloader/New Name".into()],
                stale: vec!["modloader/Old Name".into()],
                changed: vec![],
            })
        );
        env.deploy();
        assert_eq!(env.plan().state, DeployState::UpToDate);
        assert!(env.app.game_dir.join("modloader/New Name").is_symlink());
    }

    #[test]
    fn adding_removing_and_re_ruling_mods_are_all_drift() {
        let mut env = Env::new();
        env.add_modloader();
        let a = env.add("A", &["a.txt"], Some(vec![rule("a.txt", "a.txt")]));
        env.deploy();

        env.add("B", &["b"], None);
        let DeployState::OutOfDate(drift) = env.plan().state else {
            panic!("adding a mod should be drift");
        };
        assert_eq!(drift.missing, ["modloader/B"]);

        env.deploy();
        library::set_mod_files(
            &env.app.staging_dir(),
            &a,
            Some(vec![rule("a.txt", "moved.txt")]),
        )
        .unwrap();
        let DeployState::OutOfDate(drift) = env.plan().state else {
            panic!("changed rules should be drift");
        };
        assert_eq!(
            (drift.missing, drift.stale),
            (vec!["moved.txt".into()], vec!["a.txt".into()])
        );

        env.deploy();
        profile_mods::set_mod_active(&mut env.profile, &a, false).unwrap();
        let DeployState::OutOfDate(drift) = env.plan().state else {
            panic!("deactivating a mod should be drift");
        };
        assert_eq!(drift.stale, ["moved.txt"]);
    }

    #[test]
    fn a_link_pointing_somewhere_else_is_reported_as_changed() {
        let mut env = Env::new();
        env.add("M", &["a.txt"], Some(vec![rule("a.txt", "a.txt")]));
        env.deploy();
        let link = env.app.game_dir.join("a.txt");
        fs::remove_file(&link).unwrap();
        symlink(env.app.staging_dir().join("M"), &link).unwrap();

        let DeployState::OutOfDate(drift) = env.plan().state else {
            panic!("a redirected link should be drift");
        };

        assert_eq!(drift.changed, ["a.txt"]);
    }

    #[test]
    fn planning_changes_nothing() {
        let mut env = Env::new();
        env.add_modloader();
        env.add("Cool Cars", &["f"], None);

        env.plan();

        assert_eq!(fs::read_dir(&env.app.game_dir).unwrap().count(), 0);
    }
}
