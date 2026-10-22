//! Turning display names into filesystem-safe handles, and keeping them
//! unique. Pure string logic; callers decide what "taken" means.

use std::time::{SystemTime, UNIX_EPOCH};

/// Lowercase, underscore-separated, filesystem-safe handle derived from a display
/// name. Falls back to a timestamp-based slug if nothing alphanumeric
/// survives (e.g. a name made entirely of emoji/punctuation).
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_separator = false;
        } else if (c.is_whitespace() || c == '-' || c == '_') && !last_was_separator {
            slug.push('_');
            last_was_separator = true;
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("profile_{ts}")
    } else {
        slug
    }
}

/// Returns `base`, or `base_2`, `base_3`, ... for the first one that
/// `is_taken` does not report as taken.
pub fn unique(base: &str, is_taken: impl Fn(&str) -> bool) -> String {
    let mut candidate = base.to_string();
    let mut n = 2;
    while is_taken(&candidate) {
        candidate = format!("{base}_{n}");
        n += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_gives_expected_slug() {
        assert_eq!(slugify("Test Profile"), "test_profile");
        assert_eq!(slugify("  My -- Cool_Profile!  "), "my_cool_profile");
        assert!(slugify("!!!").starts_with("profile_"));
    }

    #[test]
    fn unique_returns_base_when_free() {
        assert_eq!(unique("a", |_| false), "a");
    }

    #[test]
    fn unique_appends_the_first_free_number() {
        let taken = ["a", "a_2", "a_3"];
        assert_eq!(unique("a", |c| taken.contains(&c)), "a_4");
    }
}
