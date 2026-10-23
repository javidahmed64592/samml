//! Looking inside a mod's folder, one level at a time, for the rule picker
//! (choosing a `source` for a deploy rule).
//!
//! Read-only. The folder being listed is always inside the mod's own
//! folder: `..`, absolute paths and symlinks that lead elsewhere are
//! refused.

use crate::manifest::FileEntry;
use crate::{library, paths};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

/// The items directly inside `relative` (a folder path inside the mod;
/// empty or `.` for the mod's root), folders first, then by name.
pub fn list_mod_files(staging_dir: &Path, id: &str, relative: &str) -> Result<Vec<FileEntry>> {
    let library = library::load_library(staging_dir)?;
    let name = &library::entry(&library, id)?.name;
    let root = library::mod_dir(staging_dir, name)?;
    if !root.is_dir() {
        bail!(
            "the folder '{name}' is missing from {}",
            staging_dir.display()
        );
    }

    let dir = if relative.is_empty() {
        root.clone()
    } else {
        paths::validate_relative(relative)
            .with_context(|| format!("invalid folder '{relative}'"))?;
        paths::join_normalized(&root, relative)
    };
    let resolved = dir
        .canonicalize()
        .with_context(|| format!("'{relative}' is not a folder in this mod"))?;
    if !resolved.starts_with(root.canonicalize()?) {
        bail!("'{relative}' leads outside this mod");
    }
    if !resolved.is_dir() {
        bail!("'{relative}' is not a folder in this mod");
    }

    let mut entries = Vec::new();
    for entry in
        fs::read_dir(&resolved).with_context(|| format!("reading {}", resolved.display()))?
    {
        let entry = entry?;
        let meta = entry.metadata()?;
        entries.push(FileEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: (!meta.is_dir()).then_some(meta.len()),
        });
    }
    entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase(), e.name.clone()));
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    struct Env {
        _root: TempDir,
        staging: std::path::PathBuf,
        id: String,
    }

    impl Env {
        fn new() -> Self {
            let root = TempDir::new().unwrap();
            let staging = root.path().join("staging");
            let (id, dir) = library::add_mod(&staging, "Mod", None).unwrap();
            fs::create_dir_all(dir.join("data/models")).unwrap();
            fs::create_dir_all(dir.join(".hidden_dir")).unwrap();
            fs::write(dir.join("b.asi"), "bb").unwrap();
            fs::write(dir.join("A.cfg"), "a").unwrap();
            fs::write(dir.join("data/file.dat"), "12345").unwrap();
            Env {
                _root: root,
                staging,
                id,
            }
        }

        fn list(&self, rel: &str) -> Result<Vec<FileEntry>> {
            list_mod_files(&self.staging, &self.id, rel)
        }

        fn names(&self, rel: &str) -> Vec<String> {
            self.list(rel)
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect()
        }
    }

    #[test]
    fn lists_the_root_with_folders_first_then_by_name() {
        let env = Env::new();
        assert_eq!(env.names(""), [".hidden_dir", "data", "A.cfg", "b.asi"]);
        assert_eq!(env.names("."), env.names(""));
        assert_eq!(env.names("./"), env.names(""));
    }

    #[test]
    fn lists_a_subfolder_with_sizes() {
        let env = Env::new();

        let entries = env.list("data").unwrap();

        assert_eq!(
            entries,
            [
                FileEntry {
                    name: "models".into(),
                    is_dir: true,
                    size: None
                },
                FileEntry {
                    name: "file.dat".into(),
                    is_dir: false,
                    size: Some(5)
                },
            ]
        );
        assert_eq!(env.names("data/"), env.names("data"));
    }

    #[test]
    fn refuses_to_look_outside_the_mod() {
        let env = Env::new();
        let outside = env.staging.join("Mod/../Elsewhere");
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, env.staging.join("Mod/escape")).unwrap();

        for bad in ["..", "../", "data/../..", "/etc"] {
            assert!(env.list(bad).is_err(), "{bad}");
        }
        let err = env.list("escape").unwrap_err();
        assert!(err.to_string().contains("outside this mod"));
    }

    #[test]
    fn missing_folders_files_and_mods_are_errors() {
        let env = Env::new();

        assert!(env.list("nope").is_err());
        let err = env.list("b.asi").unwrap_err();
        assert!(err.to_string().contains("not a folder"));
        assert!(list_mod_files(&env.staging, "ghost-0000", "").is_err());
    }
}
