use anyhow::{Context, Result, anyhow};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use url::Url;

use crate::state::{
    ARTWORK_PAGE_SIZE, ArtworkGame, ArtworkGrid, FileOption, Paths, Provider, ReleaseOption,
    RepoItem,
};

const APP_USER_AGENT: &str = "RepoRocket/1.0 (+https://github.com/Aspenini/RepoRocket)";
const SEARCH_LIMIT: &str = "10";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS_EVERY: Duration = Duration::from_millis(50);
const COPY_BUF: usize = 64 * 1024;

const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'%');

static HTTP: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .user_agent(APP_USER_AGENT)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(HEADER_TIMEOUT))
        .https_only(true)
        .max_idle_connections(8)
        .build()
        .into()
});

pub fn search_repositories(provider: Provider, query: &str) -> Result<Vec<RepoItem>> {
    match provider {
        Provider::GitHub => search_github(query),
        Provider::GitLab => search_gitlab(query),
        Provider::InternetArchive => search_internet_archive(query),
    }
}

pub fn fetch_releases(repo: &RepoItem) -> Result<Vec<ReleaseOption>> {
    match repo.provider {
        Provider::GitHub => fetch_github_releases(repo),
        Provider::GitLab => fetch_gitlab_releases(repo),
        Provider::InternetArchive => fetch_internet_archive_files(repo),
    }
}

fn search_github(query: &str) -> Result<Vec<RepoItem>> {
    let mut url = Url::parse("https://api.github.com/search/repositories")?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("per_page", SEARCH_LIMIT);
    let response: GitHubSearchResponse =
        get_json(url.as_str(), &[("Accept", "application/vnd.github+json")])?;
    Ok(response
        .items
        .into_iter()
        .map(|repo| RepoItem {
            provider: Provider::GitHub,
            name: repo.name,
            owner: repo.owner.login,
            description: unwrap_desc(repo.description),
            project_id: None,
            identifier: None,
        })
        .collect())
}

fn search_gitlab(query: &str) -> Result<Vec<RepoItem>> {
    let mut url = Url::parse("https://gitlab.com/api/v4/projects")?;
    url.query_pairs_mut()
        .append_pair("search", query)
        .append_pair("per_page", SEARCH_LIMIT);
    let response: Vec<GitLabProject> = get_json(url.as_str(), &[])?;
    Ok(response
        .into_iter()
        .map(|project| {
            let owner = project
                .namespace
                .and_then(|namespace| namespace.name.or(namespace.full_path))
                .unwrap_or_else(|| "GitLab".into());
            RepoItem {
                provider: Provider::GitLab,
                name: project.name,
                owner,
                description: unwrap_desc(project.description),
                project_id: Some(project.id),
                identifier: None,
            }
        })
        .collect())
}

fn search_internet_archive(query: &str) -> Result<Vec<RepoItem>> {
    let mut url = Url::parse("https://archive.org/advancedsearch.php")?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("fl[]", "identifier")
        .append_pair("fl[]", "title")
        .append_pair("fl[]", "creator")
        .append_pair("fl[]", "description")
        .append_pair("rows", SEARCH_LIMIT)
        .append_pair("output", "json");
    let response: InternetArchiveSearch = get_json(url.as_str(), &[])?;
    Ok(response
        .response
        .docs
        .into_iter()
        .map(|doc| {
            let title = doc.title.unwrap_or_else(|| doc.identifier.clone());
            RepoItem {
                provider: Provider::InternetArchive,
                name: title,
                owner: value_to_single_line(doc.creator.as_ref())
                    .unwrap_or_else(|| "Internet Archive".into()),
                description: value_to_single_line(doc.description.as_ref())
                    .unwrap_or_else(|| "No description available.".into()),
                project_id: None,
                identifier: Some(doc.identifier),
            }
        })
        .collect())
}

fn fetch_github_releases(repo: &RepoItem) -> Result<Vec<ReleaseOption>> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases",
        encode_segment(&repo.owner),
        encode_segment(&repo.name)
    );
    let releases: Vec<GitHubRelease> =
        get_json(&url, &[("Accept", "application/vnd.github+json")])?;
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
    let releases: Vec<GitLabRelease> = get_json(&url, &[])?;
    Ok(releases
        .into_iter()
        .map(|release| {
            let mut files = Vec::new();
            files.extend(release.assets.sources.into_iter().map(|source| FileOption {
                name: source.format.unwrap_or_else(|| "source archive".into()),
                url: source.url,
            }));
            files.extend(release.assets.links.into_iter().map(|link| FileOption {
                name: link.name.unwrap_or_else(|| file_name_from_url(&link.url)),
                url: link.url,
            }));
            ReleaseOption {
                title: release
                    .tag_name
                    .or(release.name)
                    .unwrap_or_else(|| "Release".into()),
                subtitle: format!("{} file(s)", files.len()),
                files,
            }
        })
        .collect())
}

fn fetch_internet_archive_files(repo: &RepoItem) -> Result<Vec<ReleaseOption>> {
    let identifier = repo
        .identifier
        .as_deref()
        .ok_or_else(|| anyhow!("missing Internet Archive identifier"))?;
    let url = format!(
        "https://archive.org/metadata/{}",
        encode_segment(identifier)
    );
    let metadata: InternetArchiveMetadata = get_json(&url, &[])?;
    let files = metadata
        .files
        .into_iter()
        .filter(|file| {
            !matches!(
                file.format.as_deref().unwrap_or_default(),
                "Metadata" | "Text" | "Item Image"
            )
        })
        .map(|file| {
            let encoded_name = file
                .name
                .split('/')
                .map(encode_segment)
                .collect::<Vec<_>>()
                .join("/");
            FileOption {
                name: file.name,
                url: format!("https://archive.org/download/{identifier}/{encoded_name}"),
            }
        })
        .collect::<Vec<_>>();

    Ok(vec![ReleaseOption {
        title: "Internet Archive files".into(),
        subtitle: format!("{} downloadable file(s)", files.len()),
        files,
    }])
}

pub fn search_artwork_games(api_key: &str, query: &str) -> Result<Vec<ArtworkGame>> {
    let url = format!(
        "https://www.steamgriddb.com/api/v2/search/autocomplete/{}",
        encode_segment(query.trim())
    );
    let response: SgdbList<SgdbGame> = get_json(&url, &[("Authorization", &bearer(api_key))])?;
    ensure_sgdb_ok(&response)?;
    Ok(response
        .data
        .into_iter()
        .map(|game| {
            let kind = if game.verified {
                "verified"
            } else {
                "community"
            };
            let subtitle = match game.release_date {
                Some(release) => format!("{kind} · released {release}"),
                None => kind.to_string(),
            };
            ArtworkGame {
                id: game.id,
                name: game.name,
                subtitle,
            }
        })
        .collect())
}

pub fn load_artwork_grids(
    paths: &Paths,
    api_key: &str,
    game_id: usize,
    page: usize,
) -> Result<Vec<ArtworkGrid>> {
    let url = format!("https://www.steamgriddb.com/api/v2/grids/game/{game_id}");
    let response: SgdbList<SgdbGrid> = get_json(&url, &[("Authorization", &bearer(api_key))])?;
    ensure_sgdb_ok(&response)?;
    let grids = response
        .data
        .into_iter()
        .filter(|grid| grid.width == 0 || grid.height == 0 || grid.width > grid.height)
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

pub fn prepare_artwork_page_previews(
    paths: &Paths,
    mut grids: Vec<ArtworkGrid>,
    page: usize,
) -> Vec<ArtworkGrid> {
    let start = page * ARTWORK_PAGE_SIZE;
    let end = (start + ARTWORK_PAGE_SIZE).min(grids.len());
    if start >= end {
        return grids;
    }

    std::thread::scope(|scope| {
        let jobs = grids[start..end]
            .iter()
            .enumerate()
            .filter(|(_, grid)| grid.preview_path.is_none())
            .map(|(index, grid)| {
                let id = grid.id;
                let thumb = grid.thumb.clone();
                (
                    index,
                    scope.spawn(move || download_preview(paths, id, &thumb).ok()),
                )
            })
            .collect::<Vec<_>>();

        for (index, job) in jobs {
            if let Some(path) = job.join().unwrap_or(None) {
                grids[start + index].preview_path = Some(path);
            }
        }
    });
    grids
}

pub fn download_artwork(paths: &Paths, app_name: &str, url: &str) -> Result<()> {
    fs::create_dir_all(&paths.artwork)?;
    crate::ops::remove_existing_artwork(paths, app_name)?;
    let extension = image_extension_from_url(url);
    let path = paths.artwork.join(format!("{app_name}.{extension}"));
    download_to_path(url, &path, |_, _| {})
}

fn download_preview(paths: &Paths, id: u32, url: &str) -> Result<PathBuf> {
    fs::create_dir_all(&paths.cache)?;
    let extension = image_extension_from_url(url);
    let path = paths.cache.join(format!("sgdb-{id}.{extension}"));
    if path.is_file() {
        return Ok(path);
    }
    download_to_path(url, &path, |_, _| {})?;
    Ok(path)
}

pub fn download_to_path<F>(url: &str, path: &Path, mut progress: F) -> Result<()>
where
    F: FnMut(u64, Option<u64>),
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut response = HTTP
        .get(url)
        .call()
        .map_err(|err| anyhow!("download failed: {err}"))?;
    let total = content_length(&response);
    let mut reader = response.body_mut().as_reader();
    let mut output = BufWriter::with_capacity(COPY_BUF, File::create(path)?);
    copy_with_progress(&mut reader, &mut output, total, &mut progress)?;
    output.flush()?;
    Ok(())
}

fn copy_with_progress<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    total: Option<u64>,
    progress: &mut impl FnMut(u64, Option<u64>),
) -> Result<u64> {
    let mut buf = vec![0_u8; COPY_BUF];
    let mut downloaded = 0_u64;
    let mut last = Instant::now()
        .checked_sub(PROGRESS_EVERY)
        .unwrap_or_else(Instant::now);
    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buf[..read])?;
        downloaded += read as u64;
        let now = Instant::now();
        if now.duration_since(last) >= PROGRESS_EVERY {
            last = now;
            progress(downloaded, total);
        }
    }
    progress(downloaded, total);
    Ok(downloaded)
}

fn content_length(response: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    response
        .headers()
        .get(ureq::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .filter(|len| *len > 0)
}

fn get_json<T: DeserializeOwned>(url: &str, headers: &[(&str, &str)]) -> Result<T> {
    let mut request = HTTP.get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request
        .call()
        .map_err(|err| anyhow!("{err}"))
        .with_context(|| format!("GET {url}"))?;
    response
        .body_mut()
        .read_json()
        .map_err(|err| anyhow!("invalid JSON from {url}: {err}"))
}

fn bearer(api_key: &str) -> String {
    format!("Bearer {api_key}")
}

fn ensure_sgdb_ok<T>(response: &SgdbList<T>) -> Result<()> {
    if response.success || !response.data.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("SteamGridDB request failed"))
    }
}

fn unwrap_desc(description: Option<String>) -> String {
    description
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| "No description available.".into())
}

fn encode_segment(value: &str) -> String {
    utf8_percent_encode(value, PATH_SEGMENT).to_string()
}

pub fn file_name_from_url(url: &str) -> String {
    let decoded = if let Ok(parsed) = Url::parse(url) {
        parsed
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .unwrap_or("download.bin")
            .to_string()
    } else {
        let without_query = url.split('?').next().unwrap_or(url);
        without_query
            .rsplit('/')
            .next()
            .unwrap_or("download.bin")
            .to_string()
    };
    let name = percent_encoding::percent_decode_str(&decoded)
        .decode_utf8_lossy()
        .trim()
        .to_string();
    if name.is_empty() {
        "download.bin".into()
    } else {
        name
    }
}

pub fn image_extension_from_url(url: &str) -> &'static str {
    let file_name = file_name_from_url(url);
    let extension = Path::new(&file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    match extension.as_deref() {
        Some("png") => "png",
        Some("jpg" | "jpeg") => "jpg",
        Some("webp") => "webp",
        Some("gif") => "gif",
        Some("bmp") => "bmp",
        _ => "png",
    }
}

fn value_to_single_line(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        other => Some(other.to_string()),
    }
    .map(|text| text.replace(['\n', '\r'], " ").trim().to_string())
    .filter(|text| !text.is_empty())
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

#[derive(Deserialize)]
struct SgdbList<T> {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    data: Vec<T>,
}

#[derive(Default, Deserialize)]
struct SgdbGame {
    #[serde(default)]
    id: usize,
    #[serde(default)]
    name: String,
    #[serde(default)]
    verified: bool,
    #[serde(default)]
    release_date: Option<u64>,
}

#[derive(Default, Deserialize)]
struct SgdbGrid {
    #[serde(default)]
    id: u32,
    #[serde(default)]
    url: String,
    #[serde(default)]
    thumb: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}
