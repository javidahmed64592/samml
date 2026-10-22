//! SAMML core: everything the CLI and the GUI do, with no printing and no
//! argument parsing.
//!
//! - `manifest`: data types (app config, profiles, mods)
//! - `config`: loading the app manifest
//! - `slug`, `paths`: name -> handle conversion, and safe relative paths
//! - `mods`: editing a profile's mod list in memory
//! - `profiles`: profile files on disk (list, create, save, rename, ...)
//! - `receipt`: record of what a deploy changed
//! - `deploy`: symlink mods into the game directory and undo it
//! - `launch`: start the game through Steam

pub mod config;
pub mod deploy;
pub mod launch;
pub mod manifest;
pub mod mods;
pub mod paths;
pub mod profiles;
pub mod receipt;
pub mod slug;
