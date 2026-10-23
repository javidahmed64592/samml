//! SAMML core: everything the CLI and the GUI do, with no printing and no
//! argument parsing.
//!
//! Data and files
//! - `manifest`: data types (app config, library, profiles)
//! - `config`: loading the app manifest
//! - `library`: the mods in staging and their deploy rules
//! - `profiles`: profile files on disk (list, create, save, rename, ...)
//! - `receipt`: record of what a deploy changed
//!
//! Operations
//! - `import`: copying files and folders into a mod's folder
//! - `browse`: listing what is inside a mod's folder
//! - `profile_mods`: editing which mods a profile uses (in memory)
//! - `usage`: renaming or deleting a mod across the library and all profiles
//! - `resolve`: a profile's active mods joined with their deploy rules
//! - `links`: which symlinks a mod needs (shared by `plan` and `deploy`)
//! - `plan`: what deploying a profile would do, and whether the game matches
//! - `deploy`: symlink mods into the game directory and undo it
//! - `launch`: start the game through Steam
//!
//! Helpers
//! - `rules`: validating deploy rules
//! - `paths`: validating and joining user-supplied relative paths
//! - `slug`: display name -> filename-safe handle

pub mod browse;
pub mod config;
pub mod deploy;
pub mod import;
pub mod launch;
pub mod library;
pub mod links;
pub mod manifest;
pub mod paths;
pub mod plan;
pub mod profile_mods;
pub mod profiles;
pub mod receipt;
pub mod resolve;
pub mod rules;
pub mod slug;
pub mod usage;
