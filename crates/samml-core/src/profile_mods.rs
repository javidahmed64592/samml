//! Editing which mods a `Profile` uses, in memory: add, remove, reorder and
//! toggle entries.
//!
//! A profile entry only holds a library mod's id and whether it is active;
//! the order of the entries is the deploy order. Pure edits and no disk
//! access: callers load a profile (`profiles`), edit it here, then save it.
//! Whether an id really names a mod in the library is checked when the
//! profile is resolved (`resolve`).

use crate::manifest::{Profile, ProfileMod};
use crate::paths;
use anyhow::{Context, Result, anyhow, bail};

fn position(profile: &Profile, id: &str) -> Result<usize> {
    profile
        .mods
        .iter()
        .position(|m| m.id == id)
        .with_context(|| format!("mod '{id}' is not in this profile"))
}

/// Appends the mod (active) at the end of the deploy order.
pub fn add_mod(profile: &mut Profile, id: &str) -> Result<()> {
    paths::validate_component(id)
        .map_err(|reason| anyhow!("'{id}' is not a valid mod id: {reason}"))?;
    if profile.mods.iter().any(|m| m.id == id) {
        bail!("this profile already uses mod '{id}'");
    }
    profile.mods.push(ProfileMod {
        id: id.to_string(),
        active: true,
    });
    Ok(())
}

/// Takes the mod out of this profile only; the library is untouched.
pub fn remove_mod(profile: &mut Profile, id: &str) -> Result<ProfileMod> {
    let index = position(profile, id)?;
    Ok(profile.mods.remove(index))
}

pub fn set_mod_active(profile: &mut Profile, id: &str, active: bool) -> Result<()> {
    let index = position(profile, id)?;
    profile.mods[index].active = active;
    Ok(())
}

/// Moves the mod so it ends up at `new_index` in deploy order.
pub fn move_mod(profile: &mut Profile, id: &str, new_index: usize) -> Result<()> {
    if new_index >= profile.mods.len() {
        bail!(
            "position {new_index} is out of range for {} mod(s)",
            profile.mods.len()
        );
    }
    let index = position(profile, id)?;
    let moved = profile.mods.remove(index);
    profile.mods.insert(new_index, moved);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_with(ids: &[&str]) -> Profile {
        let mut p = Profile {
            name: "P".into(),
            mods: Vec::new(),
        };
        for id in ids {
            add_mod(&mut p, id).unwrap();
        }
        p
    }

    fn ids(profile: &Profile) -> Vec<&str> {
        profile.mods.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn add_appends_active_mods_in_order() {
        let p = profile_with(&["b-0001", "a-0002"]);
        assert_eq!(ids(&p), ["b-0001", "a-0002"]);
        assert!(p.mods.iter().all(|m| m.active));
    }

    #[test]
    fn add_rejects_invalid_and_duplicate_ids() {
        let mut p = profile_with(&["mod-0001"]);

        let err = add_mod(&mut p, "a/b").unwrap_err();
        assert!(err.to_string().contains("cannot contain '/'"));
        let err = add_mod(&mut p, "mod-0001").unwrap_err();
        assert!(err.to_string().contains("already uses"));
        assert_eq!(ids(&p), ["mod-0001"]);
    }

    #[test]
    fn remove_returns_the_entry_and_keeps_order() {
        let mut p = profile_with(&["a", "b", "c"]);

        let removed = remove_mod(&mut p, "b").unwrap();

        assert_eq!(removed.id, "b");
        assert_eq!(ids(&p), ["a", "c"]);
        assert!(remove_mod(&mut p, "b").is_err());
    }

    #[test]
    fn set_active_toggles_the_entry() {
        let mut p = profile_with(&["m"]);

        set_mod_active(&mut p, "m", false).unwrap();
        assert!(!p.mods[0].active);
        assert!(set_mod_active(&mut p, "ghost", true).is_err());
    }

    #[test]
    fn move_reorders() {
        let mut p = profile_with(&["a", "b", "c", "d"]);

        move_mod(&mut p, "d", 0).unwrap();
        assert_eq!(ids(&p), ["d", "a", "b", "c"]);
        move_mod(&mut p, "d", 3).unwrap();
        assert_eq!(ids(&p), ["a", "b", "c", "d"]);
        move_mod(&mut p, "a", 2).unwrap();
        assert_eq!(ids(&p), ["b", "c", "a", "d"]);

        assert!(move_mod(&mut p, "a", 4).is_err());
        assert!(move_mod(&mut p, "ghost", 0).is_err());
    }
}
