#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use anyhow::{Context, Result};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;

mod net;
mod ops;
mod state;
mod theme;

use net::{
    fetch_releases, load_artwork_grids, prepare_artwork_page_previews, search_artwork_games,
    search_repositories,
};
use ops::{
    create_folder_structure, delete_application, download_file, find_artwork_path, launch_app,
    load_library, load_slint_image, log_error, read_json_or_default, sanitize_name,
    sync_cloud_save, unique_app_name, write_json_pretty,
};
use state::{
    ARTWORK_PAGE_SIZE, AppState, FileOption, Paths, Provider, SharedState, lock, steamgriddb_key,
};
use theme::{ThemeColors, load_themes};

slint::include_modules!();

#[derive(Clone)]
struct Ctx {
    ui: slint::Weak<AppWindow>,
    state: SharedState,
    paths: Paths,
}

impl Ctx {
    fn with_ui(&self, f: impl FnOnce(&AppWindow)) {
        if let Some(ui) = self.ui.upgrade() {
            f(&ui);
        }
    }

    fn spawn<T, W, D>(self, work: W, done: D)
    where
        T: Send + 'static,
        W: FnOnce() -> T + Send + 'static,
        D: FnOnce(&AppWindow, T) + Send + 'static,
    {
        let ui = self.ui;
        thread::spawn(move || {
            let result = work();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui.upgrade() {
                    done(&ui, result);
                }
            });
        });
    }
}

fn main() -> Result<()> {
    let paths = Paths::new().context("failed to read current directory")?;
    create_folder_structure(&paths)?;

    let state = Arc::new(std::sync::Mutex::new(AppState {
        settings: read_json_or_default(&paths.settings),
        config: read_json_or_default(&paths.config),
        ..AppState::default()
    }));

    let ui = AppWindow::new().context("failed to create UI")?;
    initialize_ui(&ui, &paths, &state);
    install_callbacks(&ui, paths, state);
    ui.run().context("UI exited with an error")
}

fn initialize_ui(ui: &AppWindow, paths: &Paths, state: &SharedState) {
    {
        let mut state = lock(state);
        state.themes = load_themes(paths);
        state.library = load_library(paths, &state.config);
        ui.set_provider_index(state.settings.provider_index());
        ui.set_fullscreen_enabled(state.settings.fullscreen_enabled());
        ui.window()
            .set_fullscreen(state.settings.fullscreen_enabled());
        ui.set_steamgriddb_api_key(state.settings.steamgriddb_api_key.as_str().into());
        refresh_themes(ui, &state);
        refresh_library(ui, paths, &state);
        apply_theme(ui, ThemeColors::named(paths, &state.settings.theme));
    }
    set_status(ui, "Ready");
}

fn install_callbacks(ui: &AppWindow, paths: Paths, state: SharedState) {
    let ctx = Ctx {
        ui: ui.as_weak(),
        state,
        paths,
    };

    {
        let ctx = ctx.clone();
        ui.on_request_search(move |query, provider_index| {
            let query = query.to_string();
            if query.trim().is_empty() {
                ctx.with_ui(|ui| set_status(ui, "Type a search query first."));
                return;
            }
            let provider = Provider::from_index(provider_index);
            ctx.with_ui(|ui| {
                ui.set_busy(true);
                ui.set_repo_results(empty_model());
                ui.set_status_text(format!("Searching {}...", provider.as_str()).into());
            });
            let ctx = ctx.clone();
            ctx.clone().spawn(
                move || search_repositories(provider, &query),
                move |ui, result| {
                    ui.set_busy(false);
                    match result {
                        Ok(repos) => {
                            let count = repos.len();
                            let mut state = lock(&ctx.state);
                            if state.settings.repo_source != provider.as_str() {
                                state.settings.repo_source = provider.as_str().into();
                                let _ = write_json_pretty(&ctx.paths.settings, &state.settings);
                            }
                            state.repos = repos;
                            refresh_repo_results(ui, &state);
                            set_status(ui, format!("Found {count} result(s)."));
                        }
                        Err(err) => {
                            log_error(&ctx.paths, &format!("Search failed: {err:?}"));
                            set_status(ui, format!("Search failed: {err}"));
                        }
                    }
                },
            );
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_repo_details(move |index| {
            let repo = lock(&ctx.state).repos.get(index as usize).cloned();
            let Some(repo) = repo else {
                return;
            };
            ctx.with_ui(|ui| {
                ui.set_busy(true);
                ui.set_selected_release(-1);
                ui.set_selected_file(-1);
                ui.set_release_options(empty_model());
                ui.set_file_options(empty_model());
                ui.set_repo_title(repo.name.as_str().into());
                ui.set_repo_description(repo.description.as_str().into());
                ui.set_page(1);
                set_status(ui, "Fetching releases...");
            });
            lock(&ctx.state).current_repo = Some(repo.clone());
            let ctx = ctx.clone();
            ctx.clone().spawn(
                move || fetch_releases(&repo),
                move |ui, result| {
                    ui.set_busy(false);
                    match result {
                        Ok(releases) => {
                            let count = releases.len();
                            let mut state = lock(&ctx.state);
                            state.releases = releases;
                            state.files.clear();
                            refresh_releases(ui, &state);
                            set_status(ui, format!("Loaded {count} release option(s)."));
                        }
                        Err(err) => {
                            log_error(&ctx.paths, &format!("Release fetch failed: {err:?}"));
                            set_status(ui, format!("Could not load releases: {err}"));
                        }
                    }
                },
            );
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_release_selected(move |index| {
            let mut state = lock(&ctx.state);
            state.files = state
                .releases
                .get(index as usize)
                .map(|release| release.files.clone())
                .unwrap_or_default();
            ctx.with_ui(|ui| refresh_files(ui, &state));
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_download(move |release_index, file_index| {
            let (repo_name, file) = selected_download(&ctx.state, release_index, file_index);
            let Some(file) = file else {
                ctx.with_ui(|ui| set_status(ui, "Choose a downloadable file first."));
                return;
            };
            let app_name = sanitize_name(&repo_name);
            if ctx.paths.app_dir(&app_name).exists() {
                ctx.with_ui(|ui| {
                    ui.set_pending_download_release(release_index);
                    ui.set_pending_download_file(file_index);
                    ui.set_pending_download_original(app_name.as_str().into());
                    ui.set_pending_download_name(unique_app_name(&ctx.paths, &app_name).into());
                    ui.set_pending_download_visible(true);
                    set_status(
                        ui,
                        format!("{app_name} already exists. Choose a new folder name."),
                    );
                });
                return;
            }
            begin_download(&ctx, file, app_name);
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_download_named(move |release_index, file_index, folder_name| {
            let (_, file) = selected_download(&ctx.state, release_index, file_index);
            let Some(file) = file else {
                ctx.with_ui(|ui| set_status(ui, "Choose a downloadable file first."));
                return;
            };
            let app_name = sanitize_name(folder_name.as_str());
            if ctx.paths.app_dir(&app_name).exists() {
                ctx.with_ui(|ui| {
                    ui.set_pending_download_name(unique_app_name(&ctx.paths, &app_name).into());
                    set_status(
                        ui,
                        format!("{app_name} already exists. Pick a different folder name."),
                    );
                });
                return;
            }
            begin_download(&ctx, file, app_name);
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_library_refresh(move || {
            ctx.with_ui(|ui| {
                let mut state = lock(&ctx.state);
                state.library = load_library(&ctx.paths, &state.config);
                refresh_library(ui, &ctx.paths, &state);
                set_status(ui, "Library refreshed.");
            });
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_launch_app(move |index| {
            let entry = lock(&ctx.state).library.get(index as usize).cloned();
            let Some(entry) = entry else {
                return;
            };
            match launch_app(&entry) {
                Ok(()) => ctx.with_ui(|ui| set_status(ui, format!("Launched {}.", entry.name))),
                Err(err) => {
                    log_error(&ctx.paths, &format!("Launch failed: {err:?}"));
                    ctx.with_ui(|ui| {
                        set_status(ui, format!("Could not launch {}: {err}", entry.name))
                    });
                }
            }
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_delete_app(move |index| {
            let app_name = lock(&ctx.state)
                .library
                .get(index as usize)
                .map(|entry| entry.name.clone());
            let Some(app_name) = app_name else {
                return;
            };
            match delete_application(&ctx.paths, &app_name) {
                Ok(()) => {
                    let mut state = lock(&ctx.state);
                    state.config.remove(&app_name);
                    let _ = write_json_pretty(&ctx.paths.config, &state.config);
                    state.library = load_library(&ctx.paths, &state.config);
                    ctx.with_ui(|ui| {
                        refresh_library(ui, &ctx.paths, &state);
                        set_status(ui, format!("Deleted {app_name}."));
                    });
                }
                Err(err) => {
                    log_error(&ctx.paths, &format!("Delete failed: {err:?}"));
                    ctx.with_ui(|ui| set_status(ui, format!("Could not delete {app_name}: {err}")));
                }
            }
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_change_artwork(move |index| {
            let app_name = {
                let mut state = lock(&ctx.state);
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
            if let Some(app_name) = app_name {
                ctx.with_ui(|ui| {
                    ui.set_current_app_name(app_name.as_str().into());
                    ui.set_artwork_query(app_name.as_str().into());
                    ui.set_artwork_games(empty_model());
                    ui.set_artwork_images(empty_model());
                    ui.set_artwork_page(0);
                    ui.set_artwork_page_count(1);
                    ui.set_page(5);
                    set_status(ui, "Search SteamGridDB for matching artwork.");
                });
            }
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_choose_cloud_save(move |index| {
            let app_name = lock(&ctx.state)
                .library
                .get(index as usize)
                .map(|entry| entry.name.clone());
            let Some(app_name) = app_name else {
                return;
            };
            let Some(folder) = rfd::FileDialog::new()
                .set_directory(&ctx.paths.root)
                .pick_folder()
            else {
                return;
            };
            {
                let mut state = lock(&ctx.state);
                state
                    .config
                    .entry(app_name.clone())
                    .or_default()
                    .cloud_save_location = Some(folder.to_string_lossy().into_owned());
                if let Err(err) = write_json_pretty(&ctx.paths.config, &state.config) {
                    log_error(&ctx.paths, &format!("Cloud save setup failed: {err:?}"));
                    ctx.with_ui(|ui| set_status(ui, format!("Cloud save failed: {err}")));
                    return;
                }
            }
            let paths = ctx.paths.clone();
            let done = ctx.clone();
            ctx.clone().spawn(
                move || sync_cloud_save(&paths, &app_name, &folder).map(|()| app_name),
                move |ui, result| match result {
                    Ok(app_name) => set_status(ui, format!("Cloud save synced for {app_name}.")),
                    Err(err) => {
                        log_error(&done.paths, &format!("Cloud save setup failed: {err:?}"));
                        set_status(ui, format!("Cloud save failed: {err}"));
                    }
                },
            );
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_sync_cloud_save(move |index| {
            let (app_name, location) = {
                let state = lock(&ctx.state);
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
                    let paths = ctx.paths.clone();
                    let done = ctx.clone();
                    ctx.clone().spawn(
                        move || {
                            sync_cloud_save(&paths, &app_name, Path::new(&location))
                                .map(|()| app_name)
                        },
                        move |ui, result| match result {
                            Ok(app_name) => {
                                set_status(ui, format!("Cloud save synced for {app_name}."))
                            }
                            Err(err) => {
                                log_error(&done.paths, &format!("Cloud save sync failed: {err:?}"));
                                set_status(ui, format!("Cloud save sync failed: {err}"));
                            }
                        },
                    );
                }
                (Some(app_name), None) => ctx.with_ui(|ui| {
                    set_status(
                        ui,
                        format!("No cloud save folder is configured for {app_name}."),
                    );
                }),
                _ => {}
            }
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_artwork_search(move |query| {
            let query = query.to_string();
            let api_key = steamgriddb_key(&lock(&ctx.state).settings);
            let Some(api_key) = api_key else {
                ctx.with_ui(|ui| set_status(ui, "Add a SteamGridDB API key in Settings first."));
                return;
            };
            ctx.with_ui(|ui| {
                ui.set_busy(true);
                ui.set_artwork_games(empty_model());
                set_status(ui, "Searching SteamGridDB...");
            });
            let ctx = ctx.clone();
            ctx.clone().spawn(
                move || search_artwork_games(&api_key, &query),
                move |ui, result| {
                    ui.set_busy(false);
                    match result {
                        Ok(games) => {
                            let count = games.len();
                            let mut state = lock(&ctx.state);
                            state.artwork_games = games;
                            state.artwork_grids.clear();
                            state.artwork_page = 0;
                            refresh_artwork_games(ui, &state);
                            set_status(ui, format!("Found {count} SteamGridDB game(s)."));
                        }
                        Err(err) => {
                            log_error(&ctx.paths, &format!("SteamGridDB search failed: {err:?}"));
                            set_status(ui, format!("SteamGridDB search failed: {err}"));
                        }
                    }
                },
            );
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_artwork_game(move |index| {
            let game = lock(&ctx.state).artwork_games.get(index as usize).cloned();
            let Some(game) = game else {
                return;
            };
            let api_key = steamgriddb_key(&lock(&ctx.state).settings);
            let Some(api_key) = api_key else {
                ctx.with_ui(|ui| set_status(ui, "Add a SteamGridDB API key in Settings first."));
                return;
            };
            ctx.with_ui(|ui| {
                ui.set_busy(true);
                ui.set_artwork_images(empty_model());
                ui.set_artwork_page(0);
                ui.set_artwork_page_count(1);
                ui.set_page(6);
                set_status(ui, format!("Loading artwork for {}...", game.name));
            });
            let ctx = ctx.clone();
            let paths = ctx.paths.clone();
            ctx.clone().spawn(
                move || load_artwork_grids(&paths, &api_key, game.id, 0),
                move |ui, result| {
                    ui.set_busy(false);
                    match result {
                        Ok(grids) => {
                            let count = grids.len();
                            let mut state = lock(&ctx.state);
                            state.artwork_grids = grids;
                            state.artwork_page = 0;
                            ui.set_artwork_page(0);
                            set_artwork_page_count(ui, count);
                            refresh_artwork_images(ui, &state);
                            set_status(ui, format!("Loaded {count} landscape artwork option(s)."));
                        }
                        Err(err) => {
                            log_error(&ctx.paths, &format!("Artwork load failed: {err:?}"));
                            set_status(ui, format!("Could not load artwork: {err}"));
                        }
                    }
                },
            );
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_artwork_delta(move |delta| {
            let (page, grids) = {
                let mut state = lock(&ctx.state);
                let max_page = state.artwork_grids.len().saturating_sub(1) / ARTWORK_PAGE_SIZE;
                let next = if delta < 0 {
                    state.artwork_page.saturating_sub(1)
                } else {
                    (state.artwork_page + 1).min(max_page)
                };
                state.artwork_page = next;
                (next, state.artwork_grids.clone())
            };
            ctx.with_ui(|ui| {
                ui.set_busy(true);
                ui.set_artwork_page(page as i32);
                set_status(ui, "Preparing artwork previews...");
            });
            let ctx = ctx.clone();
            let paths = ctx.paths.clone();
            ctx.clone().spawn(
                move || prepare_artwork_page_previews(&paths, grids, page),
                move |ui, updated| {
                    let mut state = lock(&ctx.state);
                    state.artwork_grids = updated;
                    ui.set_busy(false);
                    refresh_artwork_images(ui, &state);
                    set_status(ui, format!("Artwork page {}.", page + 1));
                },
            );
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_apply_artwork(move |index| {
            let (app_name, url) = {
                let state = lock(&ctx.state);
                let grid = state
                    .artwork_grids
                    .get(state.artwork_page * ARTWORK_PAGE_SIZE + index as usize);
                (
                    state.current_app_name.clone(),
                    grid.map(|grid| grid.url.clone()),
                )
            };
            let (Some(app_name), Some(url)) = (app_name, url) else {
                return;
            };
            ctx.with_ui(|ui| {
                ui.set_busy(true);
                set_status(ui, "Applying artwork...");
            });
            let ctx = ctx.clone();
            let paths = ctx.paths.clone();
            ctx.clone().spawn(
                move || crate::net::download_artwork(&paths, &app_name, &url).map(|()| app_name),
                move |ui, result| {
                    ui.set_busy(false);
                    match result {
                        Ok(app_name) => {
                            let mut state = lock(&ctx.state);
                            state.library = load_library(&ctx.paths, &state.config);
                            refresh_library(ui, &ctx.paths, &state);
                            ui.set_page(2);
                            set_status(ui, format!("Artwork applied to {app_name}."));
                        }
                        Err(err) => {
                            log_error(&ctx.paths, &format!("Artwork apply failed: {err:?}"));
                            set_status(ui, format!("Could not apply artwork: {err}"));
                        }
                    }
                },
            );
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_select_executable(move |index| {
            let (app_name, choice) = {
                let state = lock(&ctx.state);
                (
                    state.current_app_name.clone(),
                    state.executable_choices.get(index as usize).cloned(),
                )
            };
            let (Some(app_name), Some(choice)) = (app_name, choice) else {
                return;
            };
            let mut state = lock(&ctx.state);
            state.config.entry(app_name.clone()).or_default().executable =
                Some(choice.path.to_string_lossy().into_owned());
            let _ = write_json_pretty(&ctx.paths.config, &state.config);
            state.library = load_library(&ctx.paths, &state.config);
            ctx.with_ui(|ui| {
                refresh_library(ui, &ctx.paths, &state);
                ui.set_page(2);
                set_status(ui, format!("Set executable for {app_name}."));
            });
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_page(move |page| {
            ctx.with_ui(|ui| {
                let mut state = lock(&ctx.state);
                if page == 2 {
                    state.library = load_library(&ctx.paths, &state.config);
                    refresh_library(ui, &ctx.paths, &state);
                } else if page == 3 {
                    state.themes = load_themes(&ctx.paths);
                    refresh_themes(ui, &state);
                }
            });
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_save_settings(move |api_key, fullscreen| {
            {
                let mut state = lock(&ctx.state);
                state.settings.steamgriddb_api_key = api_key.to_string();
                state.settings.set_fullscreen_enabled(fullscreen);
                if let Some(ui) = ctx.ui.upgrade() {
                    state.settings.repo_source = Provider::from_index(ui.get_provider_index())
                        .as_str()
                        .into();
                }
                if let Err(err) = write_json_pretty(&ctx.paths.settings, &state.settings) {
                    log_error(&ctx.paths, &format!("Settings save failed: {err:?}"));
                }
            }
            ctx.with_ui(|ui| {
                ui.window().set_fullscreen(fullscreen);
                set_status(ui, "Settings saved.");
            });
        });
    }

    {
        let ctx = ctx.clone();
        ui.on_request_theme(move |index| {
            let theme = lock(&ctx.state).themes.get(index as usize).cloned();
            if let Some(theme) = theme {
                let mut state = lock(&ctx.state);
                state.settings.theme = theme;
                let _ = write_json_pretty(&ctx.paths.settings, &state.settings);
                ctx.with_ui(|ui| {
                    apply_theme(ui, ThemeColors::named(&ctx.paths, &state.settings.theme));
                    set_status(ui, "Theme applied.");
                });
            }
        });
    }
}

fn selected_download(
    state: &SharedState,
    release_index: i32,
    file_index: i32,
) -> (String, Option<FileOption>) {
    let state = lock(state);
    let repo_name = state
        .current_repo
        .as_ref()
        .map(|repo| repo.name.clone())
        .unwrap_or_else(|| "Application".into());
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

fn begin_download(ctx: &Ctx, file: FileOption, app_name: String) {
    ctx.with_ui(|ui| {
        ui.set_pending_download_visible(false);
        ui.set_pending_download_release(-1);
        ui.set_pending_download_file(-1);
        ui.set_pending_download_name("".into());
        ui.set_pending_download_original("".into());
        ui.set_busy(true);
        ui.set_progress(0.0);
        ui.set_progress_visible(true);
        set_status(ui, format!("Downloading {}...", file.name));
    });

    let ui_progress = ctx.ui.clone();
    let paths = ctx.paths.clone();
    let done = ctx.clone();
    ctx.clone().spawn(
        move || {
            download_file(&paths, &file, &app_name, move |downloaded, total| {
                let Some(total) = total.filter(|total| *total > 0) else {
                    return;
                };
                let progress = (downloaded as f32 / total as f32).clamp(0.0, 1.0);
                let ui_progress = ui_progress.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_progress.upgrade() {
                        ui.set_progress(progress);
                    }
                });
            })
        },
        move |ui, result| {
            ui.set_busy(false);
            ui.set_progress_visible(false);
            ui.set_progress(0.0);
            match result {
                Ok(outcome) => {
                    if outcome.opened_html {
                        set_status(ui, format!("Downloaded and opened {}.", outcome.file_name));
                    } else {
                        let app_name = outcome.app_name.clone();
                        let mut state = lock(&done.state);
                        state.current_app_name = Some(app_name.clone());
                        state.executable_choices = outcome.executables;
                        state.library = load_library(&done.paths, &state.config);
                        refresh_executables(ui, &state);
                        ui.set_current_app_name(app_name.into());
                        ui.set_page(4);
                        set_status(ui, "Download complete. Choose the executable to launch.");
                    }
                }
                Err(err) => {
                    log_error(&done.paths, &format!("Download failed: {err:?}"));
                    set_status(ui, format!("Download failed: {err}"));
                }
            }
        },
    );
}

fn apply_theme(ui: &AppWindow, colors: ThemeColors) {
    let palette = ui.global::<Palette>();
    palette.set_background(colors.background);
    palette.set_panel(colors.panel);
    palette.set_surface(colors.surface);
    palette.set_text(colors.text);
    palette.set_muted(colors.muted);
    palette.set_accent(colors.accent);
    palette.set_border(colors.border);
    palette.set_hover(colors.hover_surface);
    palette.set_selected(colors.selected_surface);
    palette.set_placeholder(colors.placeholder);
    palette.set_progress_track(colors.progress_track);
    palette.set_overlay(colors.overlay);
    palette.set_modal(colors.modal);
    palette.set_danger(colors.danger);
    palette.set_danger_text(colors.danger_text);
}

fn refresh_repo_results(ui: &AppWindow, state: &AppState) {
    ui.set_repo_results(map_model(&state.repos, |repo| RepoRow {
        title: repo.name.as_str().into(),
        subtitle: format!("{} by {}", repo.provider.as_str(), repo.owner).into(),
        description: repo.description.as_str().into(),
    }));
}

fn refresh_releases(ui: &AppWindow, state: &AppState) {
    ui.set_release_options(map_model(&state.releases, |release| OptionRow {
        title: release.title.as_str().into(),
        subtitle: release.subtitle.as_str().into(),
    }));
}

fn refresh_files(ui: &AppWindow, state: &AppState) {
    ui.set_file_options(map_model(&state.files, |file| OptionRow {
        title: file.name.as_str().into(),
        subtitle: file.url.as_str().into(),
    }));
}

fn refresh_library(ui: &AppWindow, paths: &Paths, state: &AppState) {
    ui.set_library_apps(map_model(&state.library, |entry| {
        let artwork =
            find_artwork_path(paths, &entry.name).and_then(|path| load_slint_image(&path));
        LibraryRow {
            name: entry.name.as_str().into(),
            executable: entry
                .executable
                .as_deref()
                .unwrap_or("No executable selected")
                .into(),
            has_artwork: artwork.is_some(),
            artwork: artwork.unwrap_or_default(),
        }
    }));
}

fn refresh_executables(ui: &AppWindow, state: &AppState) {
    ui.set_executable_options(map_model(&state.executable_choices, |choice| OptionRow {
        title: choice.display.as_str().into(),
        subtitle: choice.path.to_string_lossy().as_ref().into(),
    }));
}

fn refresh_artwork_games(ui: &AppWindow, state: &AppState) {
    ui.set_artwork_games(map_model(&state.artwork_games, |game| ArtworkGameRow {
        name: game.name.as_str().into(),
        subtitle: game.subtitle.as_str().into(),
    }));
}

fn refresh_artwork_images(ui: &AppWindow, state: &AppState) {
    let start = state.artwork_page * ARTWORK_PAGE_SIZE;
    let end = (start + ARTWORK_PAGE_SIZE).min(state.artwork_grids.len());
    ui.set_artwork_images(map_model(&state.artwork_grids[start..end], |grid| {
        let preview = grid.preview_path.as_deref().and_then(load_slint_image);
        ArtworkImageRow {
            url: grid.url.as_str().into(),
            dimensions: format!("{} x {}", grid.width, grid.height).into(),
            has_preview: preview.is_some(),
            preview: preview.unwrap_or_default(),
        }
    }));
}

fn refresh_themes(ui: &AppWindow, state: &AppState) {
    ui.set_themes(map_model(&state.themes, |theme| ThemeRow {
        name: theme.as_str().into(),
    }));
}

fn set_artwork_page_count(ui: &AppWindow, count: usize) {
    let pages = count.div_ceil(ARTWORK_PAGE_SIZE).max(1);
    ui.set_artwork_page_count(pages as i32);
}

fn map_model<T, R: Clone + 'static>(items: &[T], map: impl FnMut(&T) -> R) -> ModelRc<R> {
    model_from_vec(items.iter().map(map).collect())
}

fn model_from_vec<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(Rc::new(VecModel::from(rows)))
}

fn empty_model<T: Clone + 'static>() -> ModelRc<T> {
    model_from_vec(Vec::new())
}

fn set_status(ui: &AppWindow, status: impl AsRef<str>) {
    ui.set_status_text(SharedString::from(status.as_ref()));
}
