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
- `${STAGING_DIR}`: Add your mods as separate directories within this staging area before deploying them to the game directory.

## App Manifest

The app manifest is a JSON file named `app-manifest.json` that defines the game directory, staging directory, and the list of mod manifests.

```json
{
  "game_dir": "/path/to/steamapps/common/Grand Theft Auto San Andreas/", // This is ${GAME_DIR}
  "staging_dir": "/path/to/samml/staging/", // This is ${STAGING_DIR}
  "steam_app_id": 12120,
  "manifests": [] // See below section for adding manifests
}
```

## Manifest

The manifests are JSON objects that define the deployment paths for your mods within the game directory.

- `name`: The name of the mod.
- `mod_path`: The relative path to the mod within the staging directory.
- `active`: A boolean indicating whether the mod is active. Set to false to not deploy the mod.
- `files`: An array of objects specifying the source and target paths for files within the mod.

The mods will get deployed to the `modloader` directory by default if the `files` array is omitted.
However, some mods require files to be deployed to specific locations within the game directory so this can be specified in the manifest files using the `files` array.

### Trailing Slash Convention

The trailing slash convention mirrors `rsync`:
- `source` ends with '/'  => deploy the CONTENTS of that directory, merging them into `target` (the directory itself is not recreated).
- `source` has no trailing slash => deploy the item itself (file or whole directory) as a unit.
- `target` ends with '/'  => place the item INSIDE that directory, keeping the source's own name.
- `target` has no trailing slash => the item is placed at that EXACT path (i.e. may rename it).

### Deploying CLEO + Modloader

The below examples show how to deploy `CLEO 5.4.0` and `modloader` (use your preferred CLEO version).
The manifest entries for these **must come first** in the `app-manifest.json` file to ensure that the necessary directories are created before deploying other mods.
First, add these two mods to the staging directory so you have the following structure:

```
${STAGING_DIR}/
└── system/
    ├── cleo-5.4.0/
    │   ├── cleo/
    │   ├── CLEO.asi
    │   ├── bass.dll
    │   ├── vorbisFile.dll
    │   └── vorbisHooked.dll
    └── modloader/
        ├── modloader/
        │   ├── .data/
        │   └── .profiles/
        └── modloader.asi
```

These need to get deployed to the game directory with the following structure:

```
${GAME_DIR}/
├── cleo/
├── modloader/
│   ├── .data/
│   └── .profiles/
├── CLEO.asi
├── bass.dll
├── modloader.asi
├── vorbisFile.dll
└── vorbisHooked.dll
```

The included `app-manifest.json` file has manifest entries for this deployment.
Update it to reflect your actual mod setup in the staging directory.

### Adding New Mods

#### Deploying directly to modloader/

Adding new mods is similar to adding mods to the `modloader` directory.
For example, if you have a mod named "Mod 1" stored in `${STAGING_DIR}/mods/mod-1` and you would like this to be deployed to `${GAME_DIR}/modloader/mod-1`, include the following manifest entry in your `app-manifest.json` file:

```
${STAGING_DIR}/
└── mods/
    └── mod-1/
        ├── new-mod-files/
        └── new-mod.asi
```

```
${GAME_DIR}/
└── modloader/
    └── mod-1/
        ├── new-mod-files/
        └── new-mod.asi
```


```json
{
  "manifests": [
    {
      "name": "Mod 1",
      "mod_path": "mods/mod-1",
      "active": true
    }
  ]
}
```

#### Deploying to specific game directories

However, not all mods can be deployed directly to the `modloader` directory.
Some mods require files to be placed in specific locations within the game directory.
In such cases, you need to specify the `files` array in the manifest to define the source and target paths for each file.

```
${STAGING_DIR}/
└── mods/
    └── mod-2/
        ├── cleo/
        │   └── new-script.cs
        └── new-mod.asi
```

```
${GAME_DIR}/
├── cleo/
│   └── new-script.cs
└── new-mod.asi
```

```json
{
  "manifests": [
    {
      "name": "Mod 2",
      "mod_path": "mods/mod-2",
      "active": true,
      "files": [
        { "source": "cleo/new-script.cs", "target": "cleo/new-script.cs" },
        { "source": "new-mod.asi", "target": "new-mod.asi" }
      ]
    }
  ]
}
```

Any file/folder not included in the `files` array will be ignored if the `files` array is specified.
If an empty array is specified, no files from that mod will be deployed to the game directory.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
