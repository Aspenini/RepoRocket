# RepoRocket

[![GitHub release](https://img.shields.io/github/v/release/Aspenini/RepoRocket?style=flat-square&logo=github)](https://github.com/Aspenini/RepoRocket/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/Aspenini/RepoRocket/total?style=flat-square)](https://github.com/Aspenini/RepoRocket/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/Aspenini/RepoRocket/build.yml?branch=main&style=flat-square&label=build)](https://github.com/Aspenini/RepoRocket/actions/workflows/build.yml)
[![License](https://img.shields.io/github/license/Aspenini/RepoRocket?style=flat-square)](https://github.com/Aspenini/RepoRocket/blob/main/LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.88+-CE412B?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)

RepoRocket is a native Rust and Slint desktop launcher for finding, downloading, organizing, and launching apps from GitHub, GitLab, and Internet Archive.

## Features

- Search GitHub, GitLab, and Internet Archive.
- Browse releases and downloadable files.
- Download files into isolated `applications/<app>/<app>` folders.
- Prompt for a new folder name when a download would collide with an existing app.
- Extract `.zip` downloads and flatten the common single-folder archive layout.
- Pick an executable after download and launch apps from the library.
- Delete apps with a confirmation dialog.
- Add and sync per-app cloud save folders into `saves/<app>`.
- Search SteamGridDB and apply custom library artwork using the `steamgriddb_api` crate.
- Use built-in Default Dark and Default Light themes, plus optional custom themes.

## Requirements

- Rust 1.88 or newer.
- A SteamGridDB API key for artwork search.

SteamGridDB keys can be set in the Settings screen or through the `STEAMGRIDDB_API_KEY` environment variable.

## Running

```bash
cargo run
```

## Building

```bash
cargo build --release
```

The release binary is created under `target/release`.

The scripts in `scripts/` package the release binary and the `img/` assets into `dist/`:

```bash
scripts/compile_linux.sh
scripts\compile_windows.bat
```

## Runtime Folders

- `applications/`: downloaded applications.
- `saves/reporocket/config.json`: executable paths and cloud save folders.
- `saves/reporocket/settings.json`: UI settings and SteamGridDB key.
- `saves/reporocket/artwork/`: selected library artwork.
- `saves/reporocket/errorlogs.json`: recoverable runtime errors.
- `themes/`: optional custom `theme.yaml` folders.

## Custom Themes

Custom themes are read from `themes/<theme-name>/theme.yaml`. Supported keys include:

```yaml
main-background: "#101419"
panel-background: "#0b0f14"
button-color: "#17202b"
button-hover-color: "#202a37"
text-color: "#f6f8fb"
muted-text-color: "#aeb8c5"
accent-color: "#2f5f8f"
border-color: "#283241"
selected-surface-color: "#284b72"
placeholder-color: "#222b36"
progress-track-color: "#202832"
overlay-color: "#050607"
modal-color: "#1b222c"
danger-color: "#d64040"
danger-text-color: "#ff5a5a"
```

## Disclaimer

If you see a retired app on Steam by the same name, that's from when I tried to put this on Steam for cloud saves. Lets just say they didn't like it very much and gave it the boot. appID is `3513390`

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE) for details.
