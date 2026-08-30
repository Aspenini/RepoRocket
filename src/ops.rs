use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde::de::DeserializeOwned;
use slint::Image;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::net::{download_to_path, file_name_from_url};
use crate::state::{
    Config, DownloadOutcome, ErrorEntry, ExecutableChoice, FileOption, IMAGE_EXTENSIONS,
    LibraryEntry, Paths, ensure_inside,
};

const MAX_ERROR_LOG: usize = 200;

pub fn create_folder_structure(paths: &Paths) -> Result<()> {
    for dir in [
        &paths.applications,
        &paths.rr_saves,
        &paths.artwork,
        &paths.cache,
        &paths.themes,
    ] {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

pub fn read_json_or_default<T>(path: &Path) -> T
where
    T: DeserializeOwned + Default,
{
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn write_json_pretty<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp, path).with_context(|| format!("failed to write {}", path.display()))
}

pub fn log_error(paths: &Paths, error: &str) {
    let mut errors: Vec<ErrorEntry> = read_json_or_default(&paths.errors);
    errors.push(ErrorEntry {
        error: error.to_string(),
        timestamp: format_rfc3339(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0),
        ),
    });
    let extra = errors.len().saturating_sub(MAX_ERROR_LOG);
    if extra > 0 {
        errors.drain(..extra);
    }
    let _ = write_json_pretty(&paths.errors, &errors);
}

pub fn download_file<F>(
    paths: &Paths,
    file: &FileOption,
    repo_name: &str,
    progress: F,
) -> Result<DownloadOutcome>
where
    F: FnMut(u64, Option<u64>),
{
    let app_name = sanitize_name(repo_name);
    let child_folder = paths.app_dir(&app_name).join(&app_name);
    fs::create_dir_all(&child_folder)?;

    let file_name = file_name_from_url(&file.url);
    let save_path = child_folder.join(&file_name);
    download_to_path(&file.url, &save_path, progress)?;

    let mut opened_html = false;
    if is_zip_file(&save_path) {
        unzip_and_clean(&save_path, &child_folder)?;
    } else if is_html_file(&save_path) {
        opened_html = true;
        let _ = open::that(&save_path);
    }

    Ok(DownloadOutcome {
        executables: scan_executables(&child_folder)?,
        app_name,
        file_name,
        opened_html,
    })
}

fn unzip_and_clean(zip_path: &Path, extract_path: &Path) -> Result<()> {
    let file = BufReader::new(File::open(zip_path)?);
    let mut archive = zip::ZipArchive::new(file)?;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let Some(enclosed_name) = file.enclosed_name() else {
            continue;
        };
        let out_path = extract_path.join(enclosed_name);
        if file.is_dir() {
            fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = BufWriter::new(File::create(&out_path)?);
        std::io::copy(&mut file, &mut output)?;
    }

    fs::remove_file(zip_path)?;
    flatten_single_nested_dir(extract_path)
}

fn flatten_single_nested_dir(extract_path: &Path) -> Result<()> {
    let mut entries = fs::read_dir(extract_path)?.collect::<std::io::Result<Vec<_>>>()?;
    if entries.len() != 1 || !entries[0].file_type()?.is_dir() {
        return Ok(());
    }

    let nested = entries.swap_remove(0).path();
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

pub fn scan_executables(app_folder: &Path) -> Result<Vec<ExecutableChoice>> {
    let mut choices = Vec::new();
    visit_executables(app_folder, &mut choices)?;
    choices.sort_by_cached_key(|choice| choice.display.to_ascii_lowercase());
    Ok(choices)
}

fn visit_executables(dir: &Path, choices: &mut Vec<ExecutableChoice>) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let is_dir = entry.file_type()?.is_dir();
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());
        let executable = if is_dir {
            extension.as_deref() == Some("app")
        } else {
            matches!(
                extension.as_deref(),
                Some("exe" | "bat" | "cmd" | "sh" | "appimage")
            )
        };
        if executable {
            choices.push(ExecutableChoice {
                display: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Executable")
                    .to_string(),
                path: path.clone(),
            });
        }
        if is_dir && extension.as_deref() != Some("app") {
            visit_executables(&path, choices)?;
        }
    }
    Ok(())
}

pub fn load_library(paths: &Paths, config: &Config) -> Vec<LibraryEntry> {
    let mut library = fs::read_dir(&paths.applications)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let executable = config.get(&name).and_then(|entry| entry.executable.clone());
            LibraryEntry { name, executable }
        })
        .collect::<Vec<_>>();
    library.sort_by_cached_key(|entry| entry.name.to_ascii_lowercase());
    library
}

pub fn launch_app(entry: &LibraryEntry) -> Result<()> {
    let executable = entry
        .executable
        .as_deref()
        .ok_or_else(|| anyhow!("no executable selected"))?;
    let path = Path::new(executable);
    if !path.exists() {
        return Err(anyhow!("selected executable does not exist"));
    }
    ensure_unix_executable(path);
    let cwd = path.parent().unwrap_or_else(|| Path::new("."));

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
                .current_dir(cwd)
                .spawn()?;
        } else {
            Command::new(path).current_dir(cwd).spawn()?;
        }
    }

    #[cfg(target_os = "macos")]
    {
        if path.extension().and_then(|ext| ext.to_str()) == Some("app") {
            Command::new("open").arg(path).spawn()?;
        } else {
            Command::new(path).current_dir(cwd).spawn()?;
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new(path).current_dir(cwd).spawn()?;
    }

    Ok(())
}

#[cfg(unix)]
fn ensure_unix_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    let mut permissions = metadata.permissions();
    let mode = permissions.mode();
    if mode & 0o111 == 0 {
        permissions.set_mode(mode | 0o755);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
fn ensure_unix_executable(_path: &Path) {}

pub fn delete_application(paths: &Paths, app_name: &str) -> Result<()> {
    let app_folder = paths.app_dir(app_name);
    if app_folder.exists() {
        ensure_inside(&app_folder, &paths.applications)?;
        fs::remove_dir_all(app_folder)?;
    }
    Ok(())
}

pub fn sync_cloud_save(paths: &Paths, app_name: &str, source: &Path) -> Result<()> {
    if !source.exists() {
        return Err(anyhow!("cloud save folder does not exist"));
    }
    let destination = paths.saves_dir(app_name);
    fs::create_dir_all(&destination)?;
    copy_dir_contents(source, &destination)
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

pub fn find_artwork_path(paths: &Paths, app_name: &str) -> Option<PathBuf> {
    IMAGE_EXTENSIONS
        .iter()
        .map(|extension| paths.artwork.join(format!("{app_name}.{extension}")))
        .find(|path| path.is_file())
}

pub fn remove_existing_artwork(paths: &Paths, app_name: &str) -> Result<()> {
    for extension in IMAGE_EXTENSIONS {
        let path = paths.artwork.join(format!("{app_name}.{extension}"));
        if path.is_file() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub fn load_slint_image(path: &Path) -> Option<Image> {
    Image::load_from_path(path).ok()
}

pub fn sanitize_name(name: &str) -> String {
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
        "Application".into()
    } else {
        sanitized
    }
}

pub fn unique_app_name(paths: &Paths, base_name: &str) -> String {
    let base_name = sanitize_name(base_name);
    if !paths.app_dir(&base_name).exists() {
        return base_name;
    }
    (2..)
        .map(|index| format!("{base_name} {index}"))
        .find(|candidate| !paths.app_dir(candidate).exists())
        .expect("unbounded unique app name search")
}

fn is_zip_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
}

fn is_html_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"))
}

fn format_rfc3339(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let rem = unix % 86_400;
    let hour = rem / 3_600;
    let min = (rem % 3_600) / 60;
    let sec = rem % 60;

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_illegal_path_chars() {
        assert_eq!(sanitize_name("Cool:Game?*"), "Cool_Game__");
        assert_eq!(sanitize_name("   ...  "), "Application");
    }

    #[test]
    fn rfc3339_unix_epoch() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(1_724_889_600), "2024-08-29T00:00:00Z");
    }
}
