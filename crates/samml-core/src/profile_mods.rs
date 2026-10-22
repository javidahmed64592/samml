//! Editing which mods a `Profile` uses, in memory: add, remove, reorder and
//! toggle entries.
//!
//! A profile entry only names a library mod and says whether it is active;
//! the order of the entries is the deploy order. Pure edits and no disk
//! access: callers load a profile (`profiles`), edit it here, then save it.
//! Whether a named mod really exists in staging is checked when the profile
//! is resolved for deploy (`resolve`).

use crate::manifest::{Profile, ProfileMod};
use crate::paths;
use anyhow::{Context, Result, anyhow, bail};

fn position(profile: &Profile, name: &str) -> Result<usize> {
    profile
        .mods
        .iter()
        .position(|m| m.name == name)
        .with_context(|| format!("'{name}' is not in this profile"))
}

/// Appends the mod (active) at the end of the deploy order.
pub fn add_mod(profile: &mut Profile, name: &str) -> Result<()> {
    paths::validate_component(name)
        .map_err(|reason| anyhow!("'{name}' is not a valid mod name: {reason}"))?;
    if profile
        .mods
        .iter()
        .any(|m| m.name.to_lowercase() == name.to_lowercase())
    {
        bail!("this profile already uses '{name}'");
    }
    profile.mods.push(ProfileMod {
        name: name.to_string(),
        active: true,
    });
    Ok(())
}

/// Takes the mod out of this profile only; the library is untouched.
pub fn remove_mod(profile: &mut Profile, name: &str) -> Result<ProfileMod> {
    let index = position(profile, name)?;
    Ok(profile.mods.remove(index))
}

pub fn set_mod_active(profile: &mut Profile, name: &str, active: bool) -> Result<()> {
    let index = position(profile, name)?;
    profile.mods[index].active = active;
    Ok(())
}

/// Moves the mod so it ends up at `new_index` in deploy order.
pub fn move_mod(profile: &mut Profile, name: &str, new_index: usize) -> Result<()> {
    if new_index >= profile.mods.len() {
        bail!(
            "position {new_index} is out of range for {} mod(s)",
            profile.mods.len()
        );
    }
    let index = position(profile, name)?;
    let moved = profile.mods.remove(index);
    profile.mods.insert(new_index, moved);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_with(names: &[&str]) -> Profile {
        let mut p = Profile {
            name: "P".into(),
            mods: Vec::new(),
        };
        for n in names {
            add_mod(&mut p, n).unwrap();
        }
        p
    }

    fn names(profile: &Profile) -> Vec<&str> {
        profile.mods.iter().map(|m| m.name.as_str()).collect()
    }

    #[test]
    fn add_appends_active_mods_in_order() {
        let p = profile_with(&["B", "A"]);
        assert_eq!(names(&p), ["B", "A"]);
        assert!(p.mods.iter().all(|m| m.active));
    }

    #[test]
    fn add_rejects_invalid_and_duplicate_names() {
        let mut p = profile_with(&["Mod"]);

        let err = add_mod(&mut p, "a/b").unwrap_err();
        assert!(err.to_string().contains("cannot contain '/'"));
        let err = add_mod(&mut p, "MOD").unwrap_err();
        assert!(err.to_string().contains("already uses"));
        assert_eq!(names(&p), ["Mod"]);
    }

    #[test]
    fn remove_returns_the_entry_and_keeps_order() {
        let mut p = profile_with(&["A", "B", "C"]);

        let removed = remove_mod(&mut p, "B").unwrap();

        assert_eq!(removed.name, "B");
        assert_eq!(names(&p), ["A", "C"]);
        assert!(remove_mod(&mut p, "B").is_err());
    }

    #[test]
    fn set_active_toggles_the_entry() {
        let mut p = profile_with(&["M"]);

        set_mod_active(&mut p, "M", false).unwrap();
        assert!(!p.mods[0].active);
        assert!(set_mod_active(&mut p, "ghost", true).is_err());
    }

    #[test]
    fn move_reorders() {
        let mut p = profile_with(&["A", "B", "C", "D"]);

        move_mod(&mut p, "D", 0).unwrap();
        assert_eq!(names(&p), ["D", "A", "B", "C"]);
        move_mod(&mut p, "D", 3).unwrap();
        assert_eq!(names(&p), ["A", "B", "C", "D"]);
        move_mod(&mut p, "A", 2).unwrap();
        assert_eq!(names(&p), ["B", "C", "A", "D"]);

        assert!(move_mod(&mut p, "A", 4).is_err());
        assert!(move_mod(&mut p, "ghost", 0).is_err());
    }
}
