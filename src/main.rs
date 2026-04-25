use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client as HttpClient;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::SystemTime;
use steamgriddb_api::query_parameters::QueryType::Grid;
use walkdir::WalkDir;

slint::include_modules!();

const DEFAULT_API_KEY_ENV: &str = "STEAMGRIDDB_API_KEY";
const APP_USER_AGENT: &str = "RepoRocket-Rust/0.1";

type SharedState = Arc<Mutex<AppState>>;
type Config = BTreeMap<String, AppConfig>;

#[derive(Clone)]
struct Paths {
    root: PathBuf,
    applications: PathBuf,
    themes: PathBuf,
    rr_saves: PathBuf,
    artwork: PathBuf,
    cache: PathBuf,
    settings: PathBuf,
    config: PathBuf,
    errors: PathBuf,
}

impl Paths {
    fn new() -> Result<Self> {
        let root = std::env::current_dir().context("failed to read current directory")?;
        let applications = root.join("applications");
        let themes = root.join("themes");
        let rr_saves = root.join("saves").join("reporocket");
        let artwork = rr_saves.join("artwork");
        let cache = rr_saves.join("cache");
        Ok(Self {
            root,
            applications,
            themes,
            settings: rr_saves.join("settings.json"),
            config: rr_saves.join("config.json"),
            errors: rr_saves.join("errorlogs.json"),
            rr_saves,
            artwork,
            cache,
        })
    }
}

#[derive(Default)]
struct AppState {
    settings: Settings,
    config: Config,
    repos: Vec<RepoItem>,
    releases: Vec<ReleaseOption>,
    files: Vec<FileOption>,
    library: Vec<LibraryEntry>,
    executable_choices: Vec<ExecutableChoice>,
    themes: Vec<String>,
    current_repo: Option<RepoItem>,
    current_app_name: Option<String>,
    artwork_games: Vec<ArtworkGame>,
    artwork_grids: Vec<ArtworkGrid>,
    artwork_page: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    theme: String,
    fullscreen: String,
    repo_source: String,
    steamgriddb_api_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "Default Dark".to_string(),
            fullscreen: "Windowed".to_string(),
            repo_source: "GitHub".to_string(),
            steamgriddb_api_key: std::env::var(DEFAULT_API_KEY_ENV).unwrap_or_default(),
        }
    }
}

impl Settings {
    fn fullscreen_enabled(&self) -> bool {
        self.fullscreen.eq_ignore_ascii_case("fullscreen")
    }

    fn set_fullscreen_enabled(&mut self, enabled: bool) {
        self.fullscreen = if enabled { "Fullscreen" } else { "Windowed" }.to_string();
    }

    fn provider_index(&self) -> i32 {
        match self.repo_source.as_str() {
            "GitLab" => 1,
            "Internet Archive" => 2,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct AppConfig {
    executable: Option<String>,
    cloud_save_location: Option<String>,
}

#[derive(Debug, Clone)]
struct RepoItem {
    provider: Provider,
    name: String,
    owner: String,
    description: String,
    project_id: Option<u64>,
    identifier: Option<String>,
}

#[derive(Debug, Clone)]
struct ReleaseOption {
    title: String,
    subtitle: String,
    files: Vec<FileOption>,
}

#[derive(Debug, Clone)]
struct FileOption {
    name: String,
    url: String,
}

#[derive(Debug, Clone)]
struct LibraryEntry {
    name: String,
    executable: Option<String>,
}

#[derive(Debug, Clone)]
struct ExecutableChoice {
    display: String,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct ArtworkGame {
    id: usize,
    name: String,
    subtitle: String,
}

#[derive(Debug, Clone)]
struct ArtworkGrid {
    id: u32,
    url: String,
    thumb: String,
    width: u32,
    height: u32,
    preview_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
enum Provider {
    GitHub,
    GitLab,
    InternetArchive,
}

impl Provider {
    fn from_index(index: i32) -> Self {
        match index {
            1 => Self::GitLab,
            2 => Self::InternetArchive,
            _ => Self::GitHub,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::InternetArchive => "Internet Archive",
        }
    }
}

#[derive(Deserialize)]
struct GitHubSearchResponse {
    #[serde(default)]
    items: Vec<GitHubRepo>,
}

#[derive(Deserialize)]
struct GitHubRepo {
    name: String,
    #[serde(default)]
    description: Option<String>,
    owner: GitHubOwner,
}

#[derive(Deserialize)]
struct GitHubOwner {
    login: String,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct GitLabProject {
    id: u64,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    namespace: Option<GitLabNamespace>,
}

#[derive(Deserialize)]
struct GitLabNamespace {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    full_path: Option<String>,
}

#[derive(Deserialize)]
struct GitLabRelease {
    #[serde(default)]
    tag_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    assets: GitLabAssets,
}

#[derive(Default, Deserialize)]
struct GitLabAssets {
    #[serde(default)]
    sources: Vec<GitLabSource>,
    #[serde(default)]
    links: Vec<GitLabLink>,
}

#[derive(Deserialize)]
struct GitLabSource {
    #[serde(default)]
    format: Option<String>,
    url: String,
}

#[derive(Deserialize)]
struct GitLabLink {
    #[serde(default)]
    name: Option<String>,
    url: String,
}

#[derive(Deserialize)]
struct InternetArchiveSearch {
    response: InternetArchiveSearchResponse,
}

#[derive(Deserialize)]
struct InternetArchiveSearchResponse {
    #[serde(default)]
    docs: Vec<InternetArchiveDoc>,
}

#[derive(Deserialize)]
struct InternetArchiveDoc {
    identifier: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    creator: Option<Value>,
    #[serde(default)]
    description: Option<Value>,
}

#[derive(Deserialize)]
struct InternetArchiveMetadata {
    #[serde(default)]
    files: Vec<InternetArchiveFile>,
}

#[derive(Deserialize)]
struct InternetArchiveFile {
    name: String,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ErrorEntry {
    error: String,
    timestamp: String,
}

#[derive(Clone, Copy)]
struct ThemeColors {
    background: slint::Color,
    panel: slint::Color,
    surface: slint::Color,
    text: slint::Color,
    muted: slint::Color,
    accent: slint::Color,
    border: slint::Color,
    hover_surface: slint::Color,
    selected_surface: slint::Color,
    placeholder: slint::Color,
    progress_track: slint::Color,
    overlay: slint::Color,
    modal: slint::Color,
    danger: slint::Color,
    danger_text: slint::Color,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            background: slint::Color::from_rgb_u8(16, 20, 25),
            panel: slint::Color::from_rgb_u8(11, 15, 20),
            surface: slint::Color::from_rgb_u8(23, 32, 43),
            text: slint::Color::from_rgb_u8(246, 248, 251),
            muted: slint::Color::from_rgb_u8(174, 184, 197),
            accent: slint::Color::from_rgb_u8(47, 95, 143),
            border: slint::Color::from_rgb_u8(40, 50, 65),
            hover_surface: slint::Color::from_rgb_u8(32, 42, 55),
            selected_surface: slint::Color::from_rgb_u8(40, 75, 114),
            placeholder: slint::Color::from_rgb_u8(34, 43, 54),
            progress_track: slint::Color::from_rgb_u8(32, 40, 50),
            overlay: slint::Color::from_rgb_u8(5, 6, 7),
            modal: slint::Color::from_rgb_u8(27, 34, 44),
            danger: slint::Color::from_rgb_u8(214, 64, 64),
            danger_text: slint::Color::from_rgb_u8(255, 90, 90),
        }
    }
}

impl ThemeColors {
    fn light() -> Self {
        Self {
            background: slint::Color::from_rgb_u8(244, 247, 251),
            panel: slint::Color::from_rgb_u8(29, 39, 52),
            surface: slint::Color::from_rgb_u8(255, 255, 255),
            text: slint::Color::from_rgb_u8(28, 36, 48),
            muted: slint::Color::from_rgb_u8(95, 107, 123),
            accent: slint::Color::from_rgb_u8(37, 109, 179),
            border: slint::Color::from_rgb_u8(209, 218, 229),
            hover_surface: slint::Color::from_rgb_u8(232, 239, 248),
            selected_surface: slint::Color::from_rgb_u8(212, 231, 250),
            placeholder: slint::Color::from_rgb_u8(226, 233, 242),
            progress_track: slint::Color::from_rgb_u8(219, 226, 236),
            overlay: slint::Color::from_argb_u8(220, 10, 14, 20),
            modal: slint::Color::from_rgb_u8(255, 255, 255),
            danger: slint::Color::from_rgb_u8(196, 43, 43),
            danger_text: slint::Color::from_rgb_u8(196, 43, 43),
        }
    }
}

fn main() -> Result<()> {
    let paths = Paths::new()?;
    create_folder_structure(&paths)?;

    let settings: Settings = read_json_or_default(&paths.settings);
    let config: Config = read_json_or_default(&paths.config);

    let state = Arc::new(Mutex::new(AppState {
        settings,
        config,
        ..AppState::default()
    }));

    let ui = AppWindow::new().context("failed to create UI")?;
    initialize_ui(&ui, &paths, &state);
    install_callbacks(&ui, paths.clone(), state.clone());

    ui.run().context("UI exited with an error")
}

fn initialize_ui(ui: &AppWindow, paths: &Paths, state: &SharedState) {
    {
        let mut state = state.lock().expect("state lock poisoned");
        state.themes = load_themes(paths);
        state.library = load_library(paths, &state.config);
        let settings = state.settings.clone();
        ui.set_provider_index(settings.provider_index());
        ui.set_fullscreen_enabled(settings.fullscreen_enabled());
        ui.window().set_fullscreen(settings.fullscreen_enabled());
        ui.set_steamgriddb_api_key(settings.steamgriddb_api_key.into());
    }

    refresh_themes(ui, state);
    refresh_library(ui, paths, state);
    apply_selected_theme(ui, paths, state);
    set_status(ui, "Ready");
}

fn install_callbacks(ui: &AppWindow, paths: Paths, state: SharedState) {
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_search(move |query, provider_index| {
            let query = query.to_string();
            if query.trim().is_empty() {
                if let Some(ui) = ui_weak.upgrade() {
                    set_status(&ui, "Type a search query first.");
                }
                return;
            }

            let provider = Provider::from_index(provider_index);
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_busy(true);
                ui.set_repo_results(empty_model());
                ui.set_status_text(format!("Searching {}...", provider.as_str()).into());
            }

            let ui_weak = ui_weak.clone();
            let state = state.clone();
            let paths = paths.clone();
            thread::spawn(move || {
                let result = search_repositories(provider, &query);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_busy(false);
                        match result {
                            Ok(repos) => {
                                let count = repos.len();
                                {
                                    let mut state = state.lock().expect("state lock poisoned");
                                    state.settings.repo_source = provider.as_str().to_string();
                                    state.repos = repos;
                                    let _ = write_json_pretty(&paths.settings, &state.settings);
                                }
                                refresh_repo_results(&ui, &state);
                                set_status(&ui, format!("Found {count} result(s)."));
                            }
                            Err(err) => {
                                log_error(&paths, &format!("Search failed: {err:?}"));
                                set_status(&ui, format!("Search failed: {err}"));
                            }
                        }
                    }
                });
            });
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_repo_details(move |index| {
            let repo = {
                let state = state.lock().expect("state lock poisoned");
                state.repos.get(index as usize).cloned()
            };

            let Some(repo) = repo else {
                return;
            };

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_busy(true);
                ui.set_selected_release(-1);
                ui.set_selected_file(-1);
                ui.set_release_options(empty_model());
                ui.set_file_options(empty_model());
                ui.set_repo_title(repo.name.clone().into());
                ui.set_repo_description(repo.description.clone().into());
                ui.set_page(1);
                set_status(&ui, "Fetching releases...");
            }

            {
                let mut state = state.lock().expect("state lock poisoned");
                state.current_repo = Some(repo.clone());
            }

            let ui_weak = ui_weak.clone();
            let state = state.clone();
            let paths = paths.clone();
            thread::spawn(move || {
                let result = fetch_releases(&repo);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_busy(false);
                        match result {
                            Ok(releases) => {
                                let count = releases.len();
                                {
                                    let mut state = state.lock().expect("state lock poisoned");
                                    state.releases = releases;
                                    state.files.clear();
                                }
                                refresh_releases(&ui, &state);
                                set_status(&ui, format!("Loaded {count} release option(s)."));
                            }
                            Err(err) => {
                                log_error(&paths, &format!("Release fetch failed: {err:?}"));
                                set_status(&ui, format!("Could not load releases: {err}"));
                            }
                        }
                    }
                });
            });
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        ui.on_request_release_selected(move |index| {
            {
                let mut state = state.lock().expect("state lock poisoned");
                state.files = state
                    .releases
                    .get(index as usize)
                    .map(|release| release.files.clone())
                    .unwrap_or_default();
            }
            if let Some(ui) = ui_weak.upgrade() {
                refresh_files(&ui, &state);
            }
        });
    }

    {
        ui.on_request_file_selected(move |_index| {});
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_download(move |release_index, file_index| {
            let (repo_name, file) = selected_download(&state, release_index, file_index);

            let Some(file) = file else {
                if let Some(ui) = ui_weak.upgrade() {
                    set_status(&ui, "Choose a downloadable file first.");
                }
                return;
            };

            let app_name = sanitize_name(&repo_name);
            if paths.applications.join(&app_name).exists() {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_pending_download_release(release_index);
                    ui.set_pending_download_file(file_index);
                    ui.set_pending_download_original(app_name.clone().into());
                    ui.set_pending_download_name(unique_app_name(&paths, &app_name).into());
                    ui.set_pending_download_visible(true);
                    set_status(
                        &ui,
                        format!("{app_name} already exists. Choose a new folder name."),
                    );
                }
                return;
            }

            begin_download(
                ui_weak.clone(),
                state.clone(),
                paths.clone(),
                file,
                app_name,
            );
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_download_named(move |release_index, file_index, folder_name| {
            let (_, file) = selected_download(&state, release_index, file_index);

            let Some(file) = file else {
                if let Some(ui) = ui_weak.upgrade() {
                    set_status(&ui, "Choose a downloadable file first.");
                }
                return;
            };

            let app_name = sanitize_name(folder_name.as_str());
            if paths.applications.join(&app_name).exists() {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_pending_download_name(unique_app_name(&paths, &app_name).into());
                    set_status(
                        &ui,
                        format!("{app_name} already exists. Pick a different folder name."),
                    );
                }
                return;
            }

            begin_download(
                ui_weak.clone(),
                state.clone(),
                paths.clone(),
                file,
                app_name,
            );
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_library_refresh(move || {
            if let Some(ui) = ui_weak.upgrade() {
                {
                    let mut state = state.lock().expect("state lock poisoned");
                    state.library = load_library(&paths, &state.config);
                }
                refresh_library(&ui, &paths, &state);
                set_status(&ui, "Library refreshed.");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_launch_app(move |index| {
            let entry = {
                let state = state.lock().expect("state lock poisoned");
                state.library.get(index as usize).cloned()
            };
            if let Some(entry) = entry {
                match launch_app(&entry) {
                    Ok(()) => {
                        if let Some(ui) = ui_weak.upgrade() {
                            set_status(&ui, format!("Launched {}.", entry.name));
                        }
                    }
                    Err(err) => {
                        log_error(&paths, &format!("Launch failed: {err:?}"));
                        if let Some(ui) = ui_weak.upgrade() {
                            set_status(&ui, format!("Could not launch {}: {err}", entry.name));
                        }
                    }
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_delete_app(move |index| {
            let app_name = {
                let state = state.lock().expect("state lock poisoned");
                state
                    .library
                    .get(index as usize)
                    .map(|entry| entry.name.clone())
            };
            let Some(app_name) = app_name else {
                return;
            };

            match delete_application(&paths, &app_name) {
                Ok(()) => {
                    {
                        let mut state = state.lock().expect("state lock poisoned");
                        state.config.remove(&app_name);
                        let _ = write_json_pretty(&paths.config, &state.config);
                        state.library = load_library(&paths, &state.config);
                    }
                    if let Some(ui) = ui_weak.upgrade() {
                        refresh_library(&ui, &paths, &state);
                        set_status(&ui, format!("Deleted {app_name}."));
                    }
                }
                Err(err) => {
                    log_error(&paths, &format!("Delete failed: {err:?}"));
                    if let Some(ui) = ui_weak.upgrade() {
                        set_status(&ui, format!("Could not delete {app_name}: {err}"));
                    }
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        ui.on_request_change_artwork(move |index| {
            let app_name = {
                let mut state = state.lock().expect("state lock poisoned");
                let app_name = state
                    .library
                    .get(index as usize)
                    .map(|entry| entry.name.clone());
                state.current_app_name = app_name.clone();
                state.artwork_games.clear();
                state.artwork_grids.clear();
                state.artwork_page = 0;
                app_name
            };

            if let (Some(ui), Some(app_name)) = (ui_weak.upgrade(), app_name) {
                ui.set_current_app_name(app_name.clone().into());
                ui.set_artwork_query(app_name.into());
                ui.set_artwork_games(empty_model());
                ui.set_artwork_images(empty_model());
                ui.set_artwork_page(0);
                ui.set_page(5);
                set_status(&ui, "Search SteamGridDB for matching artwork.");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_choose_cloud_save(move |index| {
            let app_name = {
                let state = state.lock().expect("state lock poisoned");
                state
                    .library
                    .get(index as usize)
                    .map(|entry| entry.name.clone())
            };
            let Some(app_name) = app_name else {
                return;
            };

            if let Some(folder) = rfd::FileDialog::new()
                .set_directory(&paths.root)
                .pick_folder()
            {
                let result = (|| -> Result<()> {
                    {
                        let mut state = state.lock().expect("state lock poisoned");
                        state
                            .config
                            .entry(app_name.clone())
                            .or_default()
                            .cloud_save_location = Some(folder.to_string_lossy().to_string());
                        write_json_pretty(&paths.config, &state.config)?;
                    }
                    sync_cloud_save(&paths, &app_name, &folder)?;
                    Ok(())
                })();

                if let Some(ui) = ui_weak.upgrade() {
                    match result {
                        Ok(()) => set_status(&ui, format!("Cloud save synced for {app_name}.")),
                        Err(err) => {
                            log_error(&paths, &format!("Cloud save setup failed: {err:?}"));
                            set_status(&ui, format!("Cloud save failed: {err}"));
                        }
                    }
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_sync_cloud_save(move |index| {
            let (app_name, location) = {
                let state = state.lock().expect("state lock poisoned");
                let app_name = state
                    .library
                    .get(index as usize)
                    .map(|entry| entry.name.clone());
                let location = app_name
                    .as_ref()
                    .and_then(|name| state.config.get(name))
                    .and_then(|config| config.cloud_save_location.clone());
                (app_name, location)
            };

            match (app_name, location) {
                (Some(app_name), Some(location)) => {
                    let result = sync_cloud_save(&paths, &app_name, Path::new(&location));
                    if let Some(ui) = ui_weak.upgrade() {
                        match result {
                            Ok(()) => set_status(&ui, format!("Cloud save synced for {app_name}.")),
                            Err(err) => {
                                log_error(&paths, &format!("Cloud save sync failed: {err:?}"));
                                set_status(&ui, format!("Cloud save sync failed: {err}"));
                            }
                        }
                    }
                }
                (Some(app_name), None) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        set_status(
                            &ui,
                            format!("No cloud save folder is configured for {app_name}."),
                        );
                    }
                }
                _ => {}
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_artwork_search(move |query| {
            let query = query.to_string();
            let api_key = {
                let state = state.lock().expect("state lock poisoned");
                effective_steamgriddb_key(&state.settings)
            };

            let Some(api_key) = api_key else {
                if let Some(ui) = ui_weak.upgrade() {
                    set_status(&ui, "Add a SteamGridDB API key in Settings first.");
                }
                return;
            };

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_busy(true);
                ui.set_artwork_games(empty_model());
                set_status(&ui, "Searching SteamGridDB...");
            }

            let ui_weak = ui_weak.clone();
            let state = state.clone();
            let paths = paths.clone();
            thread::spawn(move || {
                let result = search_artwork_games(&api_key, &query);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_busy(false);
                        match result {
                            Ok(games) => {
                                let count = games.len();
                                {
                                    let mut state = state.lock().expect("state lock poisoned");
                                    state.artwork_games = games;
                                    state.artwork_grids.clear();
                                    state.artwork_page = 0;
                                }
                                refresh_artwork_games(&ui, &state);
                                set_status(&ui, format!("Found {count} SteamGridDB game(s)."));
                            }
                            Err(err) => {
                                log_error(&paths, &format!("SteamGridDB search failed: {err:?}"));
                                set_status(&ui, format!("SteamGridDB search failed: {err}"));
                            }
                        }
                    }
                });
            });
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_artwork_game(move |index| {
            let game = {
                let state = state.lock().expect("state lock poisoned");
                state.artwork_games.get(index as usize).cloned()
            };
            let Some(game) = game else {
                return;
            };

            let api_key = {
                let state = state.lock().expect("state lock poisoned");
                effective_steamgriddb_key(&state.settings)
            };
            let Some(api_key) = api_key else {
                if let Some(ui) = ui_weak.upgrade() {
                    set_status(&ui, "Add a SteamGridDB API key in Settings first.");
                }
                return;
            };

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_busy(true);
                ui.set_artwork_images(empty_model());
                ui.set_artwork_page(0);
                ui.set_page(6);
                set_status(&ui, format!("Loading artwork for {}...", game.name));
            }

            let ui_weak = ui_weak.clone();
            let state = state.clone();
            let paths = paths.clone();
            thread::spawn(move || {
                let result = load_artwork_grids(&paths, &api_key, game.id, 0);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_busy(false);
                        match result {
                            Ok(grids) => {
                                let count = grids.len();
                                {
                                    let mut state = state.lock().expect("state lock poisoned");
                                    state.artwork_grids = grids;
                                    state.artwork_page = 0;
                                }
                                ui.set_artwork_page(0);
                                refresh_artwork_images(&ui, &state);
                                set_status(
                                    &ui,
                                    format!("Loaded {count} landscape artwork option(s)."),
                                );
                            }
                            Err(err) => {
                                log_error(&paths, &format!("Artwork load failed: {err:?}"));
                                set_status(&ui, format!("Could not load artwork: {err}"));
                            }
                        }
                    }
                });
            });
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_artwork_delta(move |delta| {
            let (page, grids) = {
                let mut state = state.lock().expect("state lock poisoned");
                let max_page = state.artwork_grids.len().saturating_sub(1) / 12;
                let next = if delta < 0 {
                    state.artwork_page.saturating_sub(1)
                } else {
                    (state.artwork_page + 1).min(max_page)
                };
                state.artwork_page = next;
                (next, state.artwork_grids.clone())
            };

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_busy(true);
                ui.set_artwork_page(page as i32);
                set_status(&ui, "Preparing artwork previews...");
            }

            let ui_weak = ui_weak.clone();
            let state = state.clone();
            let paths = paths.clone();
            thread::spawn(move || {
                let updated = prepare_artwork_page_previews(&paths, grids, page);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        {
                            let mut state = state.lock().expect("state lock poisoned");
                            state.artwork_grids = updated;
                        }
                        ui.set_busy(false);
                        refresh_artwork_images(&ui, &state);
                        set_status(&ui, format!("Artwork page {}.", page + 1));
                    }
                });
            });
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_apply_artwork(move |index| {
            let (app_name, grid) = {
                let state = state.lock().expect("state lock poisoned");
                let page = state.artwork_page;
                let grid = state.artwork_grids.get(page * 12 + index as usize).cloned();
                (state.current_app_name.clone(), grid)
            };

            let (Some(app_name), Some(grid)) = (app_name, grid) else {
                return;
            };

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_busy(true);
                set_status(&ui, "Applying artwork...");
            }

            let ui_weak = ui_weak.clone();
            let state = state.clone();
            let paths = paths.clone();
            thread::spawn(move || {
                let result = download_artwork(&paths, &app_name, &grid.url);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_busy(false);
                        match result {
                            Ok(()) => {
                                {
                                    let mut state = state.lock().expect("state lock poisoned");
                                    state.library = load_library(&paths, &state.config);
                                }
                                refresh_library(&ui, &paths, &state);
                                ui.set_page(2);
                                set_status(&ui, format!("Artwork applied to {app_name}."));
                            }
                            Err(err) => {
                                log_error(&paths, &format!("Artwork apply failed: {err:?}"));
                                set_status(&ui, format!("Could not apply artwork: {err}"));
                            }
                        }
                    }
                });
            });
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_select_executable(move |index| {
            let (app_name, choice) = {
                let state = state.lock().expect("state lock poisoned");
                (
                    state.current_app_name.clone(),
                    state.executable_choices.get(index as usize).cloned(),
                )
            };

            let (Some(app_name), Some(choice)) = (app_name, choice) else {
                return;
            };

            {
                let mut state = state.lock().expect("state lock poisoned");
                state.config.entry(app_name.clone()).or_default().executable =
                    Some(choice.path.to_string_lossy().to_string());
                let _ = write_json_pretty(&paths.config, &state.config);
                state.library = load_library(&paths, &state.config);
            }

            if let Some(ui) = ui_weak.upgrade() {
                refresh_library(&ui, &paths, &state);
                ui.set_page(2);
                set_status(&ui, format!("Set executable for {app_name}."));
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_page(move |page| {
            if let Some(ui) = ui_weak.upgrade() {
                if page == 2 {
                    {
                        let mut state = state.lock().expect("state lock poisoned");
                        state.library = load_library(&paths, &state.config);
                    }
                    refresh_library(&ui, &paths, &state);
                } else if page == 3 {
                    {
                        let mut state = state.lock().expect("state lock poisoned");
                        state.themes = load_themes(&paths);
                    }
                    refresh_themes(&ui, &state);
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_save_settings(move |api_key, fullscreen| {
            {
                let mut state = state.lock().expect("state lock poisoned");
                state.settings.steamgriddb_api_key = api_key.to_string();
                state.settings.set_fullscreen_enabled(fullscreen);
                state.settings.repo_source = Provider::from_index(
                    ui_weak
                        .upgrade()
                        .map(|ui| ui.get_provider_index())
                        .unwrap_or_default(),
                )
                .as_str()
                .to_string();
                if let Err(err) = write_json_pretty(&paths.settings, &state.settings) {
                    log_error(&paths, &format!("Settings save failed: {err:?}"));
                }
            }
            if let Some(ui) = ui_weak.upgrade() {
                ui.window().set_fullscreen(fullscreen);
                set_status(&ui, "Settings saved.");
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let paths = paths.clone();
        ui.on_request_theme(move |index| {
            {
                let mut state = state.lock().expect("state lock poisoned");
                if let Some(theme) = state.themes.get(index as usize).cloned() {
                    state.settings.theme = theme;
                    let _ = write_json_pretty(&paths.settings, &state.settings);
                }
            }
            if let Some(ui) = ui_weak.upgrade() {
                apply_selected_theme(&ui, &paths, &state);
                set_status(&ui, "Theme applied.");
            }
        });
    }
}

fn selected_download(
    state: &SharedState,
    release_index: i32,
    file_index: i32,
) -> (String, Option<FileOption>) {
    let state = state.lock().expect("state lock poisoned");
    let repo_name = state
        .current_repo
        .as_ref()
        .map(|repo| repo.name.clone())
        .unwrap_or_else(|| "Application".to_string());

    if release_index < 0 || file_index < 0 {
        return (repo_name, None);
    }

    let file = state
        .releases
        .get(release_index as usize)
        .and_then(|release| release.files.get(file_index as usize))
        .cloned()
        .or_else(|| state.files.get(file_index as usize).cloned());
    (repo_name, file)
}

fn begin_download(
    ui_weak: slint::Weak<AppWindow>,
    state: SharedState,
    paths: Paths,
    file: FileOption,
    app_name: String,
) {
    if let Some(ui) = ui_weak.upgrade() {
        ui.set_pending_download_visible(false);
        ui.set_pending_download_release(-1);
        ui.set_pending_download_file(-1);
        ui.set_pending_download_name("".into());
        ui.set_pending_download_original("".into());
        ui.set_busy(true);
        ui.set_progress(0.0);
        ui.set_progress_visible(true);
        set_status(&ui, format!("Downloading {}...", file.name));
    }

    let ui_progress = ui_weak.clone();
    let ui_done = ui_weak;
    thread::spawn(move || {
        let result = download_file(&paths, &file, &app_name, move |downloaded, total| {
            if let Some(total) = total {
                if total > 0 {
                    let progress = (downloaded as f32 / total as f32).clamp(0.0, 1.0);
                    let ui_progress = ui_progress.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_progress.upgrade() {
                            ui.set_progress(progress);
                        }
                    });
                }
            }
        });

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_done.upgrade() {
                ui.set_busy(false);
                ui.set_progress_visible(false);
                ui.set_progress(0.0);
                match result {
                    Ok(outcome) => {
                        if outcome.opened_html {
                            set_status(
                                &ui,
                                format!("Downloaded and opened {}.", outcome.file_name),
                            );
                        } else {
                            let app_name = outcome.app_name.clone();
                            {
                                let mut state = state.lock().expect("state lock poisoned");
                                state.current_app_name = Some(app_name.clone());
                                state.executable_choices = outcome.executables.clone();
                                state.library = load_library(&paths, &state.config);
                            }
                            refresh_executables(&ui, &state);
                            ui.set_current_app_name(app_name.into());
                            ui.set_page(4);
                            set_status(&ui, "Download complete. Choose the executable to launch.");
                        }
                    }
                    Err(err) => {
                        log_error(&paths, &format!("Download failed: {err:?}"));
                        set_status(&ui, format!("Download failed: {err}"));
                    }
                }
            }
        });
    });
}

fn create_folder_structure(paths: &Paths) -> Result<()> {
    fs::create_dir_all(&paths.applications)?;
    fs::create_dir_all(&paths.rr_saves)?;
    fs::create_dir_all(&paths.artwork)?;
    fs::create_dir_all(&paths.cache)?;
    fs::create_dir_all(&paths.themes)?;

    if !paths.errors.exists() {
        write_json_pretty(&paths.errors, &Vec::<ErrorEntry>::new())?;
    }
    if !paths.settings.exists() {
        write_json_pretty(&paths.settings, &Settings::default())?;
    }
    if !paths.config.exists() {
        write_json_pretty(&paths.config, &Config::new())?;
    }
    Ok(())
}

fn read_json_or_default<T>(path: &Path) -> T
where
    T: DeserializeOwned + Default,
{
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn write_json_pretty<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(value)?;
    fs::write(path, text)?;
    Ok(())
}

fn log_error(paths: &Paths, error: &str) {
    let mut errors: Vec<ErrorEntry> = read_json_or_default(&paths.errors);
    errors.push(ErrorEntry {
        error: error.to_string(),
        timestamp: format!("{:?}", SystemTime::now()),
    });
    let _ = write_json_pretty(&paths.errors, &errors);
}

fn search_repositories(provider: Provider, query: &str) -> Result<Vec<RepoItem>> {
    match provider {
        Provider::GitHub => search_github(query),
        Provider::GitLab => search_gitlab(query),
        Provider::InternetArchive => search_internet_archive(query),
    }
}

fn search_github(query: &str) -> Result<Vec<RepoItem>> {
    let url = format!(
        "https://api.github.com/search/repositories?q={}&per_page=10",
        urlencoding::encode(query)
    );
    let response: GitHubSearchResponse =
        http_client().get(url).send()?.error_for_status()?.json()?;
    Ok(response
        .items
        .into_iter()
        .map(|repo| RepoItem {
            provider: Provider::GitHub,
            name: repo.name,
            owner: repo.owner.login,
            description: repo
                .description
                .unwrap_or_else(|| "No description available.".to_string()),
            project_id: None,
            identifier: None,
        })
        .collect())
}

fn search_gitlab(query: &str) -> Result<Vec<RepoItem>> {
    let url = format!(
        "https://gitlab.com/api/v4/projects?search={}&per_page=10",
        urlencoding::encode(query)
    );
    let response: Vec<GitLabProject> = http_client().get(url).send()?.error_for_status()?.json()?;
    Ok(response
        .into_iter()
        .map(|project| {
            let owner = project
                .namespace
                .and_then(|namespace| namespace.name.or(namespace.full_path))
                .unwrap_or_else(|| "GitLab".to_string());
            RepoItem {
                provider: Provider::GitLab,
                name: project.name,
                owner,
                description: project
                    .description
                    .unwrap_or_else(|| "No description available.".to_string()),
                project_id: Some(project.id),
                identifier: None,
            }
        })
        .collect())
}

fn search_internet_archive(query: &str) -> Result<Vec<RepoItem>> {
    let url = format!(
        "https://archive.org/advancedsearch.php?q={}&fl[]=identifier&fl[]=title&fl[]=creator&fl[]=description&rows=10&output=json",
        urlencoding::encode(query)
    );
    let response: InternetArchiveSearch =
        http_client().get(url).send()?.error_for_status()?.json()?;
    Ok(response
        .response
        .docs
        .into_iter()
        .map(|doc| {
            let title = doc.title.unwrap_or_else(|| doc.identifier.clone());
            let owner = value_to_single_line(doc.creator.as_ref())
                .unwrap_or_else(|| "Internet Archive".to_string());
            let description = value_to_single_line(doc.description.as_ref())
                .unwrap_or_else(|| "No description available.".to_string());
            RepoItem {
                provider: Provider::InternetArchive,
                name: title,
                owner,
                description,
                project_id: None,
                identifier: Some(doc.identifier),
            }
        })
        .collect())
}

fn fetch_releases(repo: &RepoItem) -> Result<Vec<ReleaseOption>> {
    match repo.provider {
        Provider::GitHub => fetch_github_releases(repo),
        Provider::GitLab => fetch_gitlab_releases(repo),
        Provider::InternetArchive => fetch_internet_archive_files(repo),
    }
}

fn fetch_github_releases(repo: &RepoItem) -> Result<Vec<ReleaseOption>> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases",
        repo.owner, repo.name
    );
    let releases: Vec<GitHubRelease> = http_client().get(url).send()?.error_for_status()?.json()?;
    Ok(releases
        .into_iter()
        .filter(|release| !release.draft)
        .map(|release| {
            let files = release
                .assets
                .into_iter()
                .map(|asset| FileOption {
                    name: asset.name,
                    url: asset.browser_download_url,
                })
                .collect::<Vec<_>>();
            let suffix = if release.prerelease {
                "pre-release"
            } else {
                "release"
            };
            ReleaseOption {
                title: release.tag_name,
                subtitle: format!("{} file(s), {suffix}", files.len()),
                files,
            }
        })
        .collect())
}

fn fetch_gitlab_releases(repo: &RepoItem) -> Result<Vec<ReleaseOption>> {
    let id = repo
        .project_id
        .ok_or_else(|| anyhow!("missing GitLab project id"))?;
    let url = format!("https://gitlab.com/api/v4/projects/{id}/releases");
    let releases: Vec<GitLabRelease> = http_client().get(url).send()?.error_for_status()?.json()?;
    Ok(releases
        .into_iter()
        .map(|release| {
            let mut files = Vec::new();
            for source in release.assets.sources {
                files.push(FileOption {
                    name: source
                        .format
                        .unwrap_or_else(|| "source archive".to_string()),
                    url: source.url,
                });
            }
            for link in release.assets.links {
                files.push(FileOption {
                    name: link.name.unwrap_or_else(|| file_name_from_url(&link.url)),
                    url: link.url,
                });
            }
            let title = release
                .tag_name
                .or(release.name)
                .unwrap_or_else(|| "Release".to_string());
            ReleaseOption {
                title,
                subtitle: format!("{} file(s)", files.len()),
                files,
            }
        })
        .collect())
}

fn fetch_internet_archive_files(repo: &RepoItem) -> Result<Vec<ReleaseOption>> {
    let identifier = repo
        .identifier
        .as_ref()
        .ok_or_else(|| anyhow!("missing Internet Archive identifier"))?;
    let url = format!("https://archive.org/metadata/{identifier}");
    let metadata: InternetArchiveMetadata =
        http_client().get(url).send()?.error_for_status()?.json()?;
    let files = metadata
        .files
        .into_iter()
        .filter(|file| {
            let format = file.format.as_deref().unwrap_or_default();
            !matches!(format, "Metadata" | "Text" | "Item Image")
        })
        .map(|file| {
            let encoded_name = file
                .name
                .split('/')
                .map(urlencoding::encode)
                .collect::<Vec<_>>()
                .join("/");
            FileOption {
                name: file.name.clone(),
                url: format!("https://archive.org/download/{identifier}/{encoded_name}"),
            }
        })
        .collect::<Vec<_>>();

    Ok(vec![ReleaseOption {
        title: "Internet Archive files".to_string(),
        subtitle: format!("{} downloadable file(s)", files.len()),
        files,
    }])
}

fn download_file<F>(
    paths: &Paths,
    file: &FileOption,
    repo_name: &str,
    progress: F,
) -> Result<DownloadOutcome>
where
    F: Fn(u64, Option<u64>),
{
    let app_name = sanitize_name(repo_name);
    let parent_folder = paths.applications.join(&app_name);
    let child_folder = parent_folder.join(&app_name);
    fs::create_dir_all(&child_folder)?;

    let file_name = file_name_from_url(&file.url);
    let save_path = child_folder.join(&file_name);
    let mut response = http_client().get(&file.url).send()?.error_for_status()?;
    let total = response.content_length();
    let mut output = File::create(&save_path)?;
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 8192];

    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        downloaded += read as u64;
        progress(downloaded, total);
    }

    let mut opened_html = false;
    if is_zip_file(&save_path) {
        unzip_and_clean(&save_path, &child_folder)?;
    } else if save_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"))
    {
        opened_html = true;
        let _ = open::that(&save_path);
    }

    let executables = scan_executables(&child_folder)?;
    Ok(DownloadOutcome {
        app_name,
        file_name,
        executables,
        opened_html,
    })
}

#[derive(Clone)]
struct DownloadOutcome {
    app_name: String,
    file_name: String,
    executables: Vec<ExecutableChoice>,
    opened_html: bool,
}

fn unzip_and_clean(zip_path: &Path, extract_path: &Path) -> Result<()> {
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let Some(enclosed_name) = file.enclosed_name() else {
            continue;
        };
        let out_path = extract_path.join(enclosed_name);
        if file.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = File::create(&out_path)?;
            std::io::copy(&mut file, &mut output)?;
        }
    }

    fs::remove_file(zip_path)?;
    flatten_single_nested_dir(extract_path)?;
    Ok(())
}

fn flatten_single_nested_dir(extract_path: &Path) -> Result<()> {
    let entries = fs::read_dir(extract_path)?.collect::<std::result::Result<Vec<_>, _>>()?;
    if entries.len() != 1 || !entries[0].file_type()?.is_dir() {
        return Ok(());
    }

    let nested = entries[0].path();
    for entry in fs::read_dir(&nested)? {
        let entry = entry?;
        let destination = extract_path.join(entry.file_name());
        if destination.exists() {
            if destination.is_dir() {
                fs::remove_dir_all(&destination)?;
            } else {
                fs::remove_file(&destination)?;
            }
        }
        fs::rename(entry.path(), destination)?;
    }
    fs::remove_dir_all(nested)?;
    Ok(())
}

fn scan_executables(app_folder: &Path) -> Result<Vec<ExecutableChoice>> {
    let mut choices = Vec::new();
    for entry in WalkDir::new(app_folder)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());

        let executable = if entry.file_type().is_dir() {
            extension.as_deref() == Some("app")
        } else {
            matches!(
                extension.as_deref(),
                Some("exe") | Some("bat") | Some("cmd") | Some("sh") | Some("appimage")
            )
        };

        if executable {
            choices.push(ExecutableChoice {
                display: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Executable")
                    .to_string(),
                path: path.to_path_buf(),
            });
        }
    }
    choices.sort_by(|a, b| a.display.to_lowercase().cmp(&b.display.to_lowercase()));
    Ok(choices)
}

fn load_library(paths: &Paths, config: &Config) -> Vec<LibraryEntry> {
    let mut library = fs::read_dir(&paths.applications)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let executable = config.get(&name).and_then(|entry| entry.executable.clone());
            LibraryEntry { name, executable }
        })
        .collect::<Vec<_>>();

    library.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    library
}

fn launch_app(entry: &LibraryEntry) -> Result<()> {
    let executable = entry
        .executable
        .as_ref()
        .ok_or_else(|| anyhow!("no executable selected"))?;
    let path = Path::new(executable);
    if !path.exists() {
        return Err(anyhow!("selected executable does not exist"));
    }
    ensure_unix_executable(path);

    #[cfg(windows)]
    {
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        if ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd") {
            Command::new("cmd")
                .arg("/C")
                .arg(path)
                .current_dir(path.parent().unwrap_or_else(|| Path::new(".")))
                .spawn()?;
        } else {
            Command::new(path)
                .current_dir(path.parent().unwrap_or_else(|| Path::new(".")))
                .spawn()?;
        }
    }

    #[cfg(target_os = "macos")]
    {
        if path.extension().and_then(|ext| ext.to_str()) == Some("app") {
            Command::new("open").arg(path).spawn()?;
        } else {
            Command::new(path)
                .current_dir(path.parent().unwrap_or_else(|| Path::new(".")))
                .spawn()?;
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new(path)
            .current_dir(path.parent().unwrap_or_else(|| Path::new(".")))
            .spawn()?;
    }

    Ok(())
}

#[cfg(unix)]
fn ensure_unix_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        let mode = permissions.mode();
        if mode & 0o111 == 0 {
            permissions.set_mode(mode | 0o755);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

#[cfg(not(unix))]
fn ensure_unix_executable(_path: &Path) {}

fn delete_application(paths: &Paths, app_name: &str) -> Result<()> {
    let app_folder = paths.applications.join(app_name);
    if app_folder.exists() {
        ensure_inside(&app_folder, &paths.applications)?;
        fs::remove_dir_all(app_folder)?;
    }
    Ok(())
}

fn sync_cloud_save(paths: &Paths, app_name: &str, source: &Path) -> Result<()> {
    if !source.exists() {
        return Err(anyhow!("cloud save folder does not exist"));
    }
    let destination = paths.root.join("saves").join(app_name);
    fs::create_dir_all(&destination)?;
    copy_dir_contents(source, &destination)?;
    Ok(())
}

fn copy_dir_contents(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fs::create_dir_all(&destination_path)?;
            copy_dir_contents(&source_path, &destination_path)?;
        } else {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

fn search_artwork_games(api_key: &str, query: &str) -> Result<Vec<ArtworkGame>> {
    let client = steamgriddb_api::Client::new(api_key);
    let games = client.search(query).map_err(|err| anyhow!("{err}"))?;
    Ok(games
        .into_iter()
        .map(|game| ArtworkGame {
            id: game.id,
            name: game.name,
            subtitle: format!(
                "{}{}",
                if game.verified {
                    "verified"
                } else {
                    "community"
                },
                game.release_date
                    .map(|release| format!(" · released {release}"))
                    .unwrap_or_default()
            ),
        })
        .collect())
}

fn load_artwork_grids(
    paths: &Paths,
    api_key: &str,
    game_id: usize,
    page: usize,
) -> Result<Vec<ArtworkGrid>> {
    let client = steamgriddb_api::Client::new(api_key);
    let grids = client
        .get_images_for_id(game_id, &Grid(None))
        .map_err(|err| anyhow!("{err}"))?;
    let grids = grids
        .into_iter()
        .filter(|grid| grid.width > grid.height)
        .map(|grid| ArtworkGrid {
            id: grid.id,
            url: grid.url,
            thumb: grid.thumb,
            width: grid.width,
            height: grid.height,
            preview_path: None,
        })
        .collect::<Vec<_>>();
    Ok(prepare_artwork_page_previews(paths, grids, page))
}

fn prepare_artwork_page_previews(
    paths: &Paths,
    mut grids: Vec<ArtworkGrid>,
    page: usize,
) -> Vec<ArtworkGrid> {
    let start = page * 12;
    let end = (start + 12).min(grids.len());
    for grid in grids.iter_mut().take(end).skip(start) {
        if grid.preview_path.is_none() {
            grid.preview_path = download_preview(paths, grid.id, &grid.thumb).ok();
        }
    }
    grids
}

fn download_preview(paths: &Paths, id: u32, url: &str) -> Result<PathBuf> {
    fs::create_dir_all(&paths.cache)?;
    let extension = image_extension_from_url(url);
    let path = paths.cache.join(format!("sgdb-{id}.{extension}"));
    if path.exists() {
        return Ok(path);
    }
    let mut response = http_client().get(url).send()?.error_for_status()?;
    let mut file = File::create(&path)?;
    std::io::copy(&mut response, &mut file)?;
    Ok(path)
}

fn download_artwork(paths: &Paths, app_name: &str, url: &str) -> Result<()> {
    fs::create_dir_all(&paths.artwork)?;
    remove_existing_artwork(paths, app_name)?;
    let extension = image_extension_from_url(url);
    let path = paths.artwork.join(format!("{app_name}.{extension}"));
    let mut response = http_client().get(url).send()?.error_for_status()?;
    let mut file = File::create(path)?;
    std::io::copy(&mut response, &mut file)?;
    Ok(())
}

fn effective_steamgriddb_key(settings: &Settings) -> Option<String> {
    let configured = settings.steamgriddb_api_key.trim();
    if !configured.is_empty() {
        return Some(configured.to_string());
    }
    std::env::var(DEFAULT_API_KEY_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn load_themes(paths: &Paths) -> Vec<String> {
    let mut themes = vec!["Default Dark".to_string(), "Default Light".to_string()];
    if let Ok(entries) = fs::read_dir(&paths.themes) {
        let mut custom = entries
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
            .filter(|entry| entry.path().join("theme.yaml").exists())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        custom.sort();
        themes.extend(custom);
    }
    themes
}

fn apply_selected_theme(ui: &AppWindow, paths: &Paths, state: &SharedState) {
    let theme = {
        let state = state.lock().expect("state lock poisoned");
        state.settings.theme.clone()
    };
    let colors = match theme.as_str() {
        "Default Dark" => ThemeColors::default(),
        "Default Light" => ThemeColors::light(),
        _ => load_theme_colors(paths, &theme).unwrap_or_default(),
    };
    ui.set_background_color(colors.background);
    ui.set_panel_color(colors.panel);
    ui.set_surface_color(colors.surface);
    ui.set_text_color(colors.text);
    ui.set_muted_text_color(colors.muted);
    ui.set_accent_color(colors.accent);
    ui.set_border_color(colors.border);
    ui.set_hover_surface_color(colors.hover_surface);
    ui.set_selected_surface_color(colors.selected_surface);
    ui.set_placeholder_color(colors.placeholder);
    ui.set_progress_track_color(colors.progress_track);
    ui.set_overlay_color(colors.overlay);
    ui.set_modal_color(colors.modal);
    ui.set_danger_color(colors.danger);
    ui.set_danger_text_color(colors.danger_text);
}

fn load_theme_colors(paths: &Paths, theme_name: &str) -> Result<ThemeColors> {
    let path = paths.themes.join(theme_name).join("theme.yaml");
    let value: serde_yaml::Value = serde_yaml::from_str(&fs::read_to_string(path)?)?;
    let mut colors = ThemeColors::default();

    if let Some(map) = value.as_mapping() {
        for (key, value) in map {
            let Some(key) = key.as_str() else {
                continue;
            };
            let Some(value) = value.as_str().and_then(parse_color) else {
                continue;
            };
            match key {
                "main-background" => colors.background = value,
                "panel-background" => colors.panel = value,
                "button-color" => colors.surface = value,
                "button-hover-color" => colors.hover_surface = value,
                "text-color" => colors.text = value,
                "accent-color" => colors.accent = value,
                "muted-text-color" => colors.muted = value,
                "border-color" => colors.border = value,
                "selected-surface-color" => colors.selected_surface = value,
                "placeholder-color" => colors.placeholder = value,
                "progress-track-color" => colors.progress_track = value,
                "overlay-color" => colors.overlay = value,
                "modal-color" => colors.modal = value,
                "danger-color" => colors.danger = value,
                "danger-text-color" => colors.danger_text = value,
                _ => {}
            }
        }
    }
    Ok(colors)
}

fn parse_color(value: &str) -> Option<slint::Color> {
    let hex = value.trim().strip_prefix('#')?;
    let (r, g, b) = match hex.len() {
        6 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        ),
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            (r, g, b)
        }
        _ => return None,
    };
    Some(slint::Color::from_rgb_u8(r, g, b))
}

fn refresh_repo_results(ui: &AppWindow, state: &SharedState) {
    let rows = {
        let state = state.lock().expect("state lock poisoned");
        state
            .repos
            .iter()
            .map(|repo| RepoRow {
                title: repo.name.clone().into(),
                subtitle: format!("{} by {}", repo.provider.as_str(), repo.owner).into(),
                description: repo.description.clone().into(),
            })
            .collect::<Vec<_>>()
    };
    ui.set_repo_results(model_from_vec(rows));
}

fn refresh_releases(ui: &AppWindow, state: &SharedState) {
    let rows = {
        let state = state.lock().expect("state lock poisoned");
        state
            .releases
            .iter()
            .map(|release| OptionRow {
                title: release.title.clone().into(),
                subtitle: release.subtitle.clone().into(),
            })
            .collect::<Vec<_>>()
    };
    ui.set_release_options(model_from_vec(rows));
}

fn refresh_files(ui: &AppWindow, state: &SharedState) {
    let rows = {
        let state = state.lock().expect("state lock poisoned");
        state
            .files
            .iter()
            .map(|file| OptionRow {
                title: file.name.clone().into(),
                subtitle: file.url.clone().into(),
            })
            .collect::<Vec<_>>()
    };
    ui.set_file_options(model_from_vec(rows));
}

fn refresh_library(ui: &AppWindow, paths: &Paths, state: &SharedState) {
    let rows = {
        let state = state.lock().expect("state lock poisoned");
        state
            .library
            .iter()
            .map(|entry| {
                let artwork =
                    find_artwork_path(paths, &entry.name).and_then(|path| load_slint_image(&path));
                LibraryRow {
                    name: entry.name.clone().into(),
                    executable: entry
                        .executable
                        .clone()
                        .unwrap_or_else(|| "No executable selected".to_string())
                        .into(),
                    has_artwork: artwork.is_some(),
                    artwork: artwork.unwrap_or_default(),
                }
            })
            .collect::<Vec<_>>()
    };
    ui.set_library_apps(model_from_vec(rows));
}

fn refresh_executables(ui: &AppWindow, state: &SharedState) {
    let rows = {
        let state = state.lock().expect("state lock poisoned");
        state
            .executable_choices
            .iter()
            .map(|choice| OptionRow {
                title: choice.display.clone().into(),
                subtitle: choice.path.to_string_lossy().to_string().into(),
            })
            .collect::<Vec<_>>()
    };
    ui.set_executable_options(model_from_vec(rows));
}

fn refresh_artwork_games(ui: &AppWindow, state: &SharedState) {
    let rows = {
        let state = state.lock().expect("state lock poisoned");
        state
            .artwork_games
            .iter()
            .map(|game| ArtworkGameRow {
                name: game.name.clone().into(),
                subtitle: game.subtitle.clone().into(),
            })
            .collect::<Vec<_>>()
    };
    ui.set_artwork_games(model_from_vec(rows));
}

fn refresh_artwork_images(ui: &AppWindow, state: &SharedState) {
    let rows = {
        let state = state.lock().expect("state lock poisoned");
        let start = state.artwork_page * 12;
        let end = (start + 12).min(state.artwork_grids.len());
        state.artwork_grids[start..end]
            .iter()
            .map(|grid| {
                let preview = grid.preview_path.as_deref().and_then(load_slint_image);
                ArtworkImageRow {
                    url: grid.url.clone().into(),
                    dimensions: format!("{} x {}", grid.width, grid.height).into(),
                    has_preview: preview.is_some(),
                    preview: preview.unwrap_or_default(),
                }
            })
            .collect::<Vec<_>>()
    };
    ui.set_artwork_images(model_from_vec(rows));
}

fn refresh_themes(ui: &AppWindow, state: &SharedState) {
    let rows = {
        let state = state.lock().expect("state lock poisoned");
        state
            .themes
            .iter()
            .map(|theme| ThemeRow {
                name: theme.clone().into(),
            })
            .collect::<Vec<_>>()
    };
    ui.set_themes(model_from_vec(rows));
}

fn model_from_vec<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(Rc::new(VecModel::from(rows)))
}

fn empty_model<T: Clone + 'static>() -> ModelRc<T> {
    model_from_vec(Vec::new())
}

fn set_status<S: AsRef<str>>(ui: &AppWindow, status: S) {
    ui.set_status_text(SharedString::from(status.as_ref()));
}

fn http_client() -> HttpClient {
    HttpClient::builder()
        .user_agent(APP_USER_AGENT)
        .build()
        .expect("valid HTTP client")
}

fn value_to_single_line(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        other => Some(other.to_string()),
    }
    .map(|text| text.replace(['\n', '\r'], " ").trim().to_string())
    .filter(|text| !text.is_empty())
}

fn sanitize_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string();
    if sanitized.is_empty() {
        "Application".to_string()
    } else {
        sanitized
    }
}

fn unique_app_name(paths: &Paths, base_name: &str) -> String {
    let base_name = sanitize_name(base_name);
    if !paths.applications.join(&base_name).exists() {
        return base_name;
    }

    for index in 2.. {
        let candidate = format!("{base_name} {index}");
        if !paths.applications.join(&candidate).exists() {
            return candidate;
        }
    }

    unreachable!("unbounded unique app name search should always return")
}

fn file_name_from_url(url: &str) -> String {
    let without_query = url.split('?').next().unwrap_or(url);
    let raw = without_query.rsplit('/').next().unwrap_or("download.bin");
    urlencoding::decode(raw)
        .map(|decoded| decoded.to_string())
        .unwrap_or_else(|_| raw.to_string())
        .trim()
        .to_string()
        .if_empty("download.bin")
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

fn is_zip_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
}

fn image_extension_from_url(url: &str) -> String {
    let file_name = file_name_from_url(url);
    let extension = Path::new(&file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_else(|| "png".to_string());
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => extension,
        _ => "png".to_string(),
    }
}

fn find_artwork_path(paths: &Paths, app_name: &str) -> Option<PathBuf> {
    ["png", "jpg", "jpeg", "webp", "gif", "bmp"]
        .iter()
        .map(|extension| paths.artwork.join(format!("{app_name}.{extension}")))
        .find(|path| path.exists())
}

fn remove_existing_artwork(paths: &Paths, app_name: &str) -> Result<()> {
    for extension in ["png", "jpg", "jpeg", "webp", "gif", "bmp"] {
        let path = paths.artwork.join(format!("{app_name}.{extension}"));
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn load_slint_image(path: &Path) -> Option<Image> {
    if path.exists() {
        Image::load_from_path(path).ok()
    } else {
        None
    }
}

fn ensure_inside(path: &Path, root: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if path.starts_with(&root) {
        Ok(())
    } else {
        Err(anyhow!("path is outside the expected directory"))
    }
}
