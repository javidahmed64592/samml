//! Copying files and folders from anywhere on disk into a mod's folder in
//! staging.
//!
//! Sources are native paths (from a file dialog or drag-and-drop), copied on
//! the Rust side so file contents never pass through the UI. An import is
//! planned first, checked, then carried out; `preview_import` stops after
//! the plan so the UI can show what would happen (and which existing files
//! would be replaced) before asking. If copying fails part-way, everything
//! the import created is removed again; files it replaced stay replaced.
//!
//! Symlinks are refused rather than followed, and a source can never be the
//! mod's own folder or contain it.

use crate::library;
use crate::manifest::{ConflictPolicy, ImportPreview, ImportProgress, ImportReport, Placement};
use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

struct PlannedFile {
    source: PathBuf,
    dest: PathBuf,
    size: u64,
}

struct Plan {
    root: PathBuf,
    dirs: Vec<PathBuf>,
    files: Vec<PlannedFile>,
    conflicts: Vec<PathBuf>,
}

impl Plan {
    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    fn conflict_names(&self) -> Vec<String> {
        self.conflicts.iter().map(|c| self.relative(c)).collect()
    }
}

// --- Planning ------------------------------------------------------------

/// The mod's existing folder in staging.
fn mod_root(staging_dir: &Path, id: &str) -> Result<PathBuf> {
    let library = library::load_library(staging_dir)?;
    let name = &library::entry(&library, id)?.name;
    let root = library::mod_dir(staging_dir, name)?;
    if !root.is_dir() {
        bail!(
            "the folder '{name}' is missing from {}",
            staging_dir.display()
        );
    }
    Ok(root)
}

/// Adds `source` (a file, or a folder and everything in it) to the plan as
/// `dest`.
fn plan_path(source: &Path, dest: PathBuf, plan: &mut Plan) -> Result<()> {
    let meta =
        fs::symlink_metadata(source).with_context(|| format!("reading {}", source.display()))?;
    if meta.file_type().is_symlink() {
        bail!("symbolic links are not supported: {}", source.display());
    }
    if meta.is_dir() {
        plan.dirs.push(dest.clone());
        let mut children: Vec<_> = fs::read_dir(source)
            .with_context(|| format!("reading {}", source.display()))?
            .collect::<std::io::Result<_>>()?;
        children.sort_by_key(|c| c.file_name());
        for child in children {
            plan_path(&child.path(), dest.join(child.file_name()), plan)?;
        }
    } else if meta.is_file() {
        plan.files.push(PlannedFile {
            source: source.to_path_buf(),
            dest,
            size: meta.len(),
        });
    } else {
        bail!("not a regular file or folder: {}", source.display());
    }
    Ok(())
}

/// Refuses sources that are, contain, or sit inside the mod's own folder.
fn check_not_overlapping(root: &Path, source: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let source = source
        .canonicalize()
        .with_context(|| format!("reading {}", source.display()))?;
    if root.starts_with(&source) {
        bail!(
            "{} contains the mod's own folder, so it cannot be imported into it",
            source.display()
        );
    }
    if source.starts_with(&root) {
        bail!("{} is already inside this mod", source.display());
    }
    Ok(())
}

fn build_plan(
    staging_dir: &Path,
    id: &str,
    sources: &[PathBuf],
    placement: Placement,
) -> Result<Plan> {
    let root = mod_root(staging_dir, id)?;
    if sources.is_empty() {
        bail!("there is nothing to import");
    }
    let mut plan = Plan {
        root: root.clone(),
        dirs: Vec::new(),
        files: Vec::new(),
        conflicts: Vec::new(),
    };

    for source in sources {
        check_not_overlapping(&root, source)?;
    }
    match placement {
        Placement::Items => {
            for source in sources {
                let name = source
                    .file_name()
                    .with_context(|| format!("{} has no name to import under", source.display()))?;
                plan_path(source, root.join(name), &mut plan)?;
            }
        }
        Placement::Contents => {
            let [source] = sources else {
                bail!("choose exactly one folder to import the contents of");
            };
            if !fs::symlink_metadata(source)?.is_dir() {
                bail!("{} is not a folder", source.display());
            }
            let mut children: Vec<_> = fs::read_dir(source)
                .with_context(|| format!("reading {}", source.display()))?
                .collect::<std::io::Result<_>>()?;
            children.sort_by_key(|c| c.file_name());
            for child in children {
                plan_path(&child.path(), root.join(child.file_name()), &mut plan)?;
            }
        }
    }

    check_destinations(&mut plan)?;
    Ok(plan)
}

/// Finds files that would be replaced, and refuses imports that would put a
/// file where a folder is (or the reverse) or write the same path twice.
fn check_destinations(plan: &mut Plan) -> Result<()> {
    let mut seen = HashSet::new();
    for path in plan.dirs.iter().chain(plan.files.iter().map(|f| &f.dest)) {
        if !seen.insert(path.clone()) {
            bail!(
                "two of the items to import would both be written to '{}'",
                plan.relative(path)
            );
        }
    }
    for dir in &plan.dirs {
        if dir.exists() && !dir.is_dir() {
            bail!(
                "'{}' is a file in this mod, but a folder with that name is being imported",
                plan.relative(dir)
            );
        }
    }
    let mut conflicts = Vec::new();
    for file in &plan.files {
        if file.dest.is_dir() {
            bail!(
                "'{}' is a folder in this mod, but a file with that name is being imported",
                plan.relative(&file.dest)
            );
        }
        if file.dest.exists() {
            conflicts.push(file.dest.clone());
        }
    }
    plan.conflicts = conflicts;
    Ok(())
}

/// What importing `sources` into the mod would do, without changing
/// anything.
pub fn preview_import(
    staging_dir: &Path,
    id: &str,
    sources: &[PathBuf],
    placement: Placement,
) -> Result<ImportPreview> {
    let plan = build_plan(staging_dir, id, sources, placement)?;
    Ok(ImportPreview {
        files: plan.files.len(),
        bytes: plan.total_bytes(),
        conflicts: plan.conflict_names(),
    })
}

// --- Copying -------------------------------------------------------------

/// Copies via a temporary file in the destination folder, then renames it
/// into place, so a file is never left half-written.
fn copy_file(source: &Path, dest: &Path) -> Result<()> {
    let name = dest.file_name().context("destination has no name")?;
    let tmp = dest.with_file_name(format!(".{}.samml_tmp", name.to_string_lossy()));
    let result = fs::copy(source, &tmp)
        .and_then(|_| fs::rename(&tmp, dest))
        .with_context(|| format!("copying {} to {}", source.display(), dest.display()));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn roll_back(created: &[PathBuf]) {
    for path in created.iter().rev() {
        if path.is_dir() {
            let _ = fs::remove_dir(path);
        } else {
            let _ = fs::remove_file(path);
        }
    }
}

/// Copies `sources` into the mod's folder. With `ConflictPolicy::Fail`
/// nothing is copied if any existing file would be replaced (use
/// `preview_import` to find out first). `progress` is called after each
/// file.
pub fn import_into_mod(
    staging_dir: &Path,
    id: &str,
    sources: &[PathBuf],
    placement: Placement,
    policy: ConflictPolicy,
    mut progress: impl FnMut(&ImportProgress),
) -> Result<ImportReport> {
    let plan = build_plan(staging_dir, id, sources, placement)?;
    if policy == ConflictPolicy::Fail && !plan.conflicts.is_empty() {
        let names = plan.conflict_names();
        let shown: Vec<_> = names.iter().take(10).cloned().collect();
        bail!(
            "{} file(s) already exist in this mod, e.g. {}",
            names.len(),
            shown.join(", ")
        );
    }

    let conflicts: HashSet<&PathBuf> = plan.conflicts.iter().collect();
    let mut created: Vec<PathBuf> = Vec::new();
    let mut report = ImportReport::default();
    let total = ImportProgress {
        files_done: 0,
        files_total: plan.files.len(),
        bytes_done: 0,
        bytes_total: plan.total_bytes(),
    };

    let result = (|| -> Result<()> {
        for dir in &plan.dirs {
            if !dir.exists() {
                fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
                created.push(dir.clone());
            }
        }
        for file in &plan.files {
            let replaced = conflicts.contains(&file.dest);
            copy_file(&file.source, &file.dest)?;
            if replaced {
                report.overwritten.push(plan.relative(&file.dest));
            } else {
                created.push(file.dest.clone());
            }
            report.files_copied += 1;
            report.bytes_copied += file.size;
            progress(&ImportProgress {
                files_done: report.files_copied,
                bytes_done: report.bytes_copied,
                ..total.clone()
            });
        }
        Ok(())
    })();

    if let Err(err) = result {
        roll_back(&created);
        return Err(err);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use tempfile::TempDir;

    struct Env {
        _root: TempDir,
        staging: PathBuf,
        source: PathBuf,
        id: String,
    }

    impl Env {
        /// A staging dir holding one empty mod "Mod", and a separate dir to
        /// import from.
        fn new() -> Self {
            let root = TempDir::new().unwrap();
            let staging = root.path().join("staging");
            let source = root.path().join("downloads");
            fs::create_dir_all(&source).unwrap();
            let (id, _) = library::add_mod(&staging, "Mod", None).unwrap();
            Env {
                _root: root,
                staging,
                source,
                id,
            }
        }

        fn src(&self, rel: &str, contents: &str) -> PathBuf {
            let path = self.source.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }

        fn in_mod(&self, rel: &str) -> PathBuf {
            self.staging.join("Mod").join(rel)
        }

        fn import(
            &self,
            sources: &[PathBuf],
            placement: Placement,
            policy: ConflictPolicy,
        ) -> Result<ImportReport> {
            import_into_mod(&self.staging, &self.id, sources, placement, policy, |_| {})
        }

        fn read_in_mod(&self, rel: &str) -> String {
            fs::read_to_string(self.in_mod(rel)).unwrap()
        }
    }

    // --- placement ---

    #[test]
    fn items_are_placed_inside_the_mod_under_their_own_names() {
        let env = Env::new();
        let file = env.src("readme.txt", "hi");
        env.src("pack/data/a.dat", "a");
        env.src("pack/b.asi", "b");

        let report = env
            .import(
                &[file, env.source.join("pack")],
                Placement::Items,
                ConflictPolicy::Fail,
            )
            .unwrap();

        assert_eq!(env.read_in_mod("readme.txt"), "hi");
        assert_eq!(env.read_in_mod("pack/data/a.dat"), "a");
        assert_eq!(env.read_in_mod("pack/b.asi"), "b");
        assert_eq!(report.files_copied, 3);
        assert_eq!(report.bytes_copied, 4);
        assert!(report.overwritten.is_empty());
    }

    #[test]
    fn contents_merges_the_chosen_folder_into_the_mod_root() {
        let env = Env::new();
        env.src("pack/data/a.dat", "a");
        env.src("pack/b.asi", "b");

        env.import(
            &[env.source.join("pack")],
            Placement::Contents,
            ConflictPolicy::Fail,
        )
        .unwrap();

        assert_eq!(env.read_in_mod("data/a.dat"), "a");
        assert_eq!(env.read_in_mod("b.asi"), "b");
        assert!(!env.in_mod("pack").exists());
    }

    #[test]
    fn contents_needs_exactly_one_folder() {
        let env = Env::new();
        let file = env.src("f.txt", "x");
        env.src("one/a", "a");
        env.src("two/b", "b");

        let two = [env.source.join("one"), env.source.join("two")];
        let err = env
            .import(&two, Placement::Contents, ConflictPolicy::Fail)
            .unwrap_err();
        assert!(err.to_string().contains("exactly one folder"));
        let err = env
            .import(&[file], Placement::Contents, ConflictPolicy::Fail)
            .unwrap_err();
        assert!(err.to_string().contains("not a folder"));
    }

    #[test]
    fn empty_folders_are_kept() {
        let env = Env::new();
        fs::create_dir_all(env.source.join("pack/empty")).unwrap();

        env.import(
            &[env.source.join("pack")],
            Placement::Items,
            ConflictPolicy::Fail,
        )
        .unwrap();

        assert!(env.in_mod("pack/empty").is_dir());
    }

    #[test]
    fn copies_keep_the_executable_bit_and_leave_the_originals() {
        let env = Env::new();
        let tool = env.src("tool.sh", "#!/bin/sh");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();

        env.import(
            std::slice::from_ref(&tool),
            Placement::Items,
            ConflictPolicy::Fail,
        )
        .unwrap();

        let mode = fs::metadata(env.in_mod("tool.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
        assert!(tool.is_file(), "the source is copied, not moved");
    }

    // --- preview and conflicts ---

    #[test]
    fn preview_counts_files_and_changes_nothing() {
        let env = Env::new();
        env.src("pack/a.dat", "aaa");
        env.src("pack/sub/b.dat", "bb");

        let preview = preview_import(
            &env.staging,
            &env.id,
            &[env.source.join("pack")],
            Placement::Items,
        )
        .unwrap();

        assert_eq!((preview.files, preview.bytes), (2, 5));
        assert!(preview.conflicts.is_empty());
        assert!(!env.in_mod("pack").exists());
    }

    #[test]
    fn existing_files_are_conflicts_that_fail_the_import_by_default() {
        let env = Env::new();
        fs::write(env.in_mod("a.dat"), "old").unwrap();
        fs::write(env.in_mod("keep.dat"), "keep").unwrap();
        let a = env.src("a.dat", "new");
        let b = env.src("b.dat", "b");
        let sources = [a, b];

        let preview = preview_import(&env.staging, &env.id, &sources, Placement::Items).unwrap();
        assert_eq!(preview.conflicts, ["a.dat"]);

        let err = env
            .import(&sources, Placement::Items, ConflictPolicy::Fail)
            .unwrap_err();
        assert!(err.to_string().contains("a.dat"), "{err}");
        assert_eq!(env.read_in_mod("a.dat"), "old", "nothing was copied");
        assert!(!env.in_mod("b.dat").exists());
    }

    #[test]
    fn overwrite_replaces_conflicts_and_reports_them() {
        let env = Env::new();
        fs::write(env.in_mod("a.dat"), "old").unwrap();
        fs::write(env.in_mod("keep.dat"), "keep").unwrap();
        let sources = [env.src("a.dat", "new"), env.src("b.dat", "b")];

        let report = env
            .import(&sources, Placement::Items, ConflictPolicy::Overwrite)
            .unwrap();

        assert_eq!(env.read_in_mod("a.dat"), "new");
        assert_eq!(env.read_in_mod("b.dat"), "b");
        assert_eq!(env.read_in_mod("keep.dat"), "keep", "other files stay");
        assert_eq!(report.overwritten, ["a.dat"]);
        assert_eq!(report.files_copied, 2);
    }

    #[test]
    fn files_and_folders_cannot_swap_places() {
        let env = Env::new();
        fs::create_dir(env.in_mod("thing")).unwrap();
        fs::write(env.in_mod("other"), "file").unwrap();
        let file_over_dir = env.src("thing", "i am a file");
        env.src("other/inner.txt", "x");

        let err = env
            .import(
                &[file_over_dir],
                Placement::Items,
                ConflictPolicy::Overwrite,
            )
            .unwrap_err();
        assert!(err.to_string().contains("is a folder in this mod"));
        let err = env
            .import(
                &[env.source.join("other")],
                Placement::Items,
                ConflictPolicy::Overwrite,
            )
            .unwrap_err();
        assert!(err.to_string().contains("is a file in this mod"));
    }

    #[test]
    fn two_sources_with_the_same_name_are_refused() {
        let env = Env::new();
        let one = env.src("one/same.txt", "1");
        let two = env.src("two/same.txt", "2");

        let err = env
            .import(&[one, two], Placement::Items, ConflictPolicy::Overwrite)
            .unwrap_err();

        assert!(err.to_string().contains("both be written to 'same.txt'"));
        assert!(!env.in_mod("same.txt").exists());
    }

    // --- safety ---

    #[test]
    fn symbolic_links_are_refused() {
        let env = Env::new();
        let target = env.src("real.txt", "x");
        let link = env.source.join("link.txt");
        symlink(&target, &link).unwrap();

        let err = env
            .import(&[link], Placement::Items, ConflictPolicy::Fail)
            .unwrap_err();

        assert!(err.to_string().contains("symbolic links"));
    }

    #[test]
    fn a_mod_cannot_be_imported_into_itself() {
        let env = Env::new();
        fs::write(env.in_mod("inside.txt"), "x").unwrap();

        for source in [
            env.staging.clone(),
            env.in_mod(""),
            env.in_mod("inside.txt"),
        ] {
            let err = env
                .import(&[source], Placement::Items, ConflictPolicy::Overwrite)
                .unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("contains the mod's own folder") || msg.contains("already inside"),
                "{msg}"
            );
        }
    }

    #[test]
    fn unknown_mods_missing_sources_and_empty_imports_are_errors() {
        let env = Env::new();
        let file = env.src("f.txt", "x");

        let err = import_into_mod(
            &env.staging,
            "ghost-0000",
            std::slice::from_ref(&file),
            Placement::Items,
            ConflictPolicy::Fail,
            |_| {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("no mod with id"));

        assert!(
            env.import(
                &[env.source.join("missing.txt")],
                Placement::Items,
                ConflictPolicy::Fail
            )
            .is_err()
        );
        let err = env
            .import(&[], Placement::Items, ConflictPolicy::Fail)
            .unwrap_err();
        assert!(err.to_string().contains("nothing to import"));
    }

    #[test]
    fn a_mod_whose_folder_is_missing_cannot_be_imported_into() {
        let env = Env::new();
        let file = env.src("f.txt", "x");
        fs::remove_dir(env.in_mod("")).unwrap();

        let err = env
            .import(&[file], Placement::Items, ConflictPolicy::Fail)
            .unwrap_err();

        assert!(err.to_string().contains("is missing"));
    }

    // --- progress and failure ---

    #[test]
    fn progress_is_reported_after_each_file() {
        let env = Env::new();
        env.src("pack/a", "aa");
        env.src("pack/b", "bbb");
        let mut seen = Vec::new();

        import_into_mod(
            &env.staging,
            &env.id,
            &[env.source.join("pack")],
            Placement::Items,
            ConflictPolicy::Fail,
            |p| seen.push((p.files_done, p.files_total, p.bytes_done, p.bytes_total)),
        )
        .unwrap();

        assert_eq!(seen, [(1, 2, 2, 5), (2, 2, 5, 5)]);
    }

    #[test]
    fn a_failed_copy_removes_what_the_import_created() {
        let env = Env::new();
        fs::write(env.in_mod("existing.txt"), "keep").unwrap();
        env.src("pack/a.txt", "a");
        let unreadable = env.src("pack/z.txt", "z");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();

        let result = env.import(
            &[env.source.join("pack")],
            Placement::Items,
            ConflictPolicy::Fail,
        );

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).unwrap();
        // Running as root can read the file anyway; only check the rollback
        // when the copy really failed.
        if result.is_err() {
            assert!(!env.in_mod("pack").exists(), "partial import is removed");
            assert!(!env.in_mod(".z.txt.samml_tmp").exists());
            assert_eq!(env.read_in_mod("existing.txt"), "keep");
        }
    }
}
