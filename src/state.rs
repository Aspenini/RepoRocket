use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub const STEAMGRIDDB_KEY_ENV: &str = "STEAMGRIDDB_API_KEY";
pub const ARTWORK_PAGE_SIZE: usize = 12;
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];

pub type SharedState = Arc<Mutex<AppState>>;
pub type Config = BTreeMap<String, AppConfig>;

#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub applications: PathBuf,
    pub themes: PathBuf,
    pub rr_saves: PathBuf,
    pub artwork: PathBuf,
    pub cache: PathBuf,
    pub settings: PathBuf,
    pub config: PathBuf,
    pub errors: PathBuf,
}

impl Paths {
    pub fn new() -> Result<Self> {
        let root = std::env::current_dir()?;
        let applications = root.join("applications");
        let themes = root.join("themes");
        let rr_saves = root.join("saves").join("reporocket");
        let artwork = rr_saves.join("artwork");
        let cache = rr_saves.join("cache");
        Ok(Self {
            settings: rr_saves.join("settings.json"),
            config: rr_saves.join("config.json"),
            errors: rr_saves.join("errorlogs.json"),
            root,
            applications,
            themes,
            rr_saves,
            artwork,
            cache,
        })
    }

    pub fn app_dir(&self, name: &str) -> PathBuf {
        self.applications.join(name)
    }

    pub fn saves_dir(&self, name: &str) -> PathBuf {
        self.root.join("saves").join(name)
    }
}

#[derive(Default)]
pub struct AppState {
    pub settings: Settings,
    pub config: Config,
    pub repos: Vec<RepoItem>,
    pub releases: Vec<ReleaseOption>,
    pub files: Vec<FileOption>,
    pub library: Vec<LibraryEntry>,
    pub executable_choices: Vec<ExecutableChoice>,
    pub themes: Vec<String>,
    pub current_repo: Option<RepoItem>,
    pub current_app_name: Option<String>,
    pub artwork_games: Vec<ArtworkGame>,
    pub artwork_grids: Vec<ArtworkGrid>,
    pub artwork_page: usize,
}

pub fn lock(state: &SharedState) -> MutexGuard<'_, AppState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub fullscreen: String,
    pub repo_source: String,
    pub steamgriddb_api_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "Default Dark".into(),
            fullscreen: "Windowed".into(),
            repo_source: "GitHub".into(),
            steamgriddb_api_key: std::env::var(STEAMGRIDDB_KEY_ENV).unwrap_or_default(),
        }
    }
}

impl Settings {
    pub fn fullscreen_enabled(&self) -> bool {
        self.fullscreen.eq_ignore_ascii_case("fullscreen")
    }

    pub fn set_fullscreen_enabled(&mut self, enabled: bool) {
        self.fullscreen = if enabled { "Fullscreen" } else { "Windowed" }.into();
    }

    pub fn provider_index(&self) -> i32 {
        Provider::from_label(&self.repo_source).as_index()
    }

    pub fn steamgriddb_key(&self) -> Option<&str> {
        let configured = self.steamgriddb_api_key.trim();
        if !configured.is_empty() {
            return Some(configured);
        }
        None
    }
}

pub fn steamgriddb_key(settings: &Settings) -> Option<String> {
    if let Some(key) = settings.steamgriddb_key() {
        return Some(key.to_string());
    }
    std::env::var(STEAMGRIDDB_KEY_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub executable: Option<String>,
    pub cloud_save_location: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RepoItem {
    pub provider: Provider,
    pub name: String,
    pub owner: String,
    pub description: String,
    pub project_id: Option<u64>,
    pub identifier: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReleaseOption {
    pub title: String,
    pub subtitle: String,
    pub files: Vec<FileOption>,
}

#[derive(Debug, Clone)]
pub struct FileOption {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct LibraryEntry {
    pub name: String,
    pub executable: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExecutableChoice {
    pub display: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ArtworkGame {
    pub id: usize,
    pub name: String,
    pub subtitle: String,
}

#[derive(Debug, Clone)]
pub struct ArtworkGrid {
    pub id: u32,
    pub url: String,
    pub thumb: String,
    pub width: u32,
    pub height: u32,
    pub preview_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    GitHub,
    GitLab,
    InternetArchive,
}

impl Provider {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::GitLab,
            2 => Self::InternetArchive,
            _ => Self::GitHub,
        }
    }

    pub fn from_label(label: &str) -> Self {
        match label {
            "GitLab" => Self::GitLab,
            "Internet Archive" => Self::InternetArchive,
            _ => Self::GitHub,
        }
    }

    pub fn as_index(self) -> i32 {
        match self {
            Self::GitHub => 0,
            Self::GitLab => 1,
            Self::InternetArchive => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::InternetArchive => "Internet Archive",
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct ErrorEntry {
    pub error: String,
    pub timestamp: String,
}

#[derive(Clone)]
pub struct DownloadOutcome {
    pub app_name: String,
    pub file_name: String,
    pub executables: Vec<ExecutableChoice>,
    pub opened_html: bool,
}

pub fn ensure_inside(path: &Path, root: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if path.starts_with(&root) {
        Ok(())
    } else {
        Err(anyhow!("path is outside the expected directory"))
    }
}
