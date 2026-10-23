[![Rust](https://img.shields.io/badge/Rust-1.95.0-blue?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![CI](https://img.shields.io/github/actions/workflow/status/javidahmed64592/samml/ci.yml?branch=main&style=flat-square&label=CI&logo=github)](https://github.com/javidahmed64592/samml/actions/workflows/ci.yml)
[![Docs](https://img.shields.io/github/actions/workflow/status/javidahmed64592/samml/docs.yml?branch=main&style=flat-square&label=Docs&logo=github)](https://github.com/javidahmed64592/samml/actions/workflows/docs.yml)
[![Release](https://img.shields.io/github/actions/workflow/status/javidahmed64592/samml/release.yml?style=flat-square&label=Release&logo=github)](https://github.com/javidahmed64592/samml/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

<!-- omit from toc -->
# San Andreas Mod Manager Linux (SAMML)

A mod manager for Grand Theft Auto: San Andreas (v1.0 on Steam) on Linux, built in Rust.

## File Path Variables

In this guide, we will use the following variables for file paths:

- `${GAME_DIR}`: The root directory of the Grand Theft Auto: San Andreas game installation. This directory should contain the game executable.
- `${STAGING_DIR}`: Your mod library. Every folder directly inside it is a mod.
- `${PROFILES_DIR}`: The directory where mod profiles for the mod manager are stored.

## App Manifest

The app manifest is a JSON file named `app_manifest.json` that defines the game directory, staging directory, and the profiles directory.
Copy `app_manifest.example.json` to `app_manifest.json` and edit the paths.

```json
{
  "game_dir": "/path/to/steamapps/common/Grand Theft Auto San Andreas/",
  "staging_dir": "/path/to/samml/staging/",
  "profiles_dir": "/path/to/samml/profiles/",
  "steam_app_id": 12120
}
```

## Mods

Mods live in `${STAGING_DIR}`, one folder per mod, with no nesting:

```
${STAGING_DIR}/
├── .samml_library.json
├── CLEO 5.4.0/
├── Modloader/
├── Ragdoll/
└── Widescreen Fix/
```

The folder name is the mod's name.
It is also the folder the mod appears under in `modloader/` (and so the name modloader shows in game), so it must be a valid folder name: not empty, no leading or trailing spaces, no `/` or `\`, no control characters, and not starting with a `.`.
Names are unique ignoring case.
Folders starting with a `.` are ignored.

You can drop a new folder into `${STAGING_DIR}` at any time and it becomes available to every profile.
Renaming a mod renames its folder.
Profiles refer to mods by a stable ID (below), so no profile needs updating, but the in-game folder changes too, so redeploy afterwards.

### Library file

`${STAGING_DIR}/.samml_library.json` lists the mods, keyed by a stable **ID**, with each mod's name and, if it needs them, its deploy rules:

```json
{
  "mods": {
    "cleo_5_4_0-7287": {
      "name": "CLEO 5.4.0",
      "files": [
        { "source": "cleo", "target": "cleo" },
        { "source": "CLEO.asi", "target": "CLEO.asi" },
        { "source": "bass.dll", "target": "bass.dll" },
        { "source": "vorbisFile.dll", "target": "vorbisFile.dll" },
        { "source": "vorbisHooked.dll", "target": "vorbisHooked.dll" }
      ]
    },
    "widescreen_fix-9906": {
      "name": "Widescreen Fix"
    }
  }
}
```

- `name`: The mod's folder in `${STAGING_DIR}`. See above for what makes a valid name.
- `files`: Where the mod's files go. Omit it (or leave it empty) to deploy the whole mod to `${GAME_DIR}/modloader/<name>`.
  - `source`: a path inside the mod's folder.
  - `target`: a path inside `${GAME_DIR}`.
  - Paths must be relative and cannot contain `..`. Anything not listed is ignored when `files` is given.

The ID is made from the mod's name when it is first added (a slug plus four random characters) and never changes, even if the mod is renamed.
You do not need to write IDs yourself: a folder dropped into `${STAGING_DIR}` is given one the next time the library is listed.

Because the mods and their rules are all inside `${STAGING_DIR}`, zipping that folder backs up your whole library.

#### Trailing Slash Convention

The trailing slash convention mirrors `rsync`:
- `source` ends with '/'  => deploy the CONTENTS of that directory, merging them into `target` (the directory itself is not recreated).
- `source` has no trailing slash => deploy the item itself (file or whole directory) as a unit.
- `target` ends with '/'  => place the item INSIDE that directory, keeping the source's own name.
- `target` has no trailing slash => the item is placed at that EXACT path (i.e. may rename it).
- `target` of `./` => place the item directly in `${GAME_DIR}`.

For example, a mod called "Mod 2" with these rules:

```json
"Mod 2": {
  "files": [
    { "source": "data/", "target": "data/" },
    { "source": "cleo/new-script.cs", "target": "cleo/new-script.cs" },
    { "source": "new-mod.asi", "target": "new-mod.asi" }
  ]
}
```

is deployed like this:

```
${STAGING_DIR}/Mod 2/          ${GAME_DIR}/
├── cleo/                      ├── cleo/
│   └── new-script.cs          │   └── new-script.cs
├── data/                      ├── data/
│   └── file.dat               │   ├── ...
└── new-mod.asi                │   └── file.dat
                               └── new-mod.asi
```

## Profiles

A profile is a JSON file in `${PROFILES_DIR}` that picks mods from the library and puts them in deploy order.
It does not contain any deployment paths; those belong to the mods, so any number of profiles can share a mod.

```json
{
  "name": "Default",
  "mods": [
    { "id": "cleo_5_4_0-7287", "active": true },
    { "id": "modloader-bf47", "active": true },
    { "id": "widescreen_fix-9906", "active": true },
    { "id": "ragdoll-ddc0", "active": false }
  ]
}
```

- `name`: The profile's display name.
- `mods`: The mods to deploy, in order. Each has the `id` of a mod in the library and an `active` flag; set `active` to false to leave it out without losing its place.

Mods are deployed in the order listed.
**CLEO and Modloader must come first**, because other mods are placed inside the `modloader/` folder that Modloader creates and many need CLEO.
If a profile lists a mod that is not in the library, or whose folder is missing from `${STAGING_DIR}`, deploying fails and names the problem mods.

A mod without `files` is linked whole to `modloader/<name>`:

```
${STAGING_DIR}/Widescreen Fix/       ${GAME_DIR}/modloader/Widescreen Fix/
├── new-mod-files/                   ├── new-mod-files/
└── new-mod.asi                      └── new-mod.asi
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
