//! Settings, browser-extension setup and data maintenance: everything the CLI
//! does with `config`, `browser`, `export`, `import`, `backup`, `verify` and
//! `prune`, so the app needs no command line.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt as _;

use cf_app::maintenance;
use cf_config::Config;
use cf_storage::Database;

use crate::commands::blocking;
use crate::dto::*;
use crate::error::{CmdResult, CommandError, ErrorCode};
use crate::state::AppState;

fn conflict(msg: impl std::fmt::Display) -> CommandError {
    CommandError::new(ErrorCode::Conflict, msg.to_string())
}

fn open(path: &Path) -> CmdResult<Database> {
    Database::open(path).map_err(Into::into)
}

// ------------------------------------------------------------------ settings

fn settings_from(cfg: &Config, start_with_windows: bool) -> Settings {
    Settings {
        capture: CaptureSettings {
            include_windows: cfg.capture.include_windows,
            excluded_processes: cfg.capture.excluded_processes.clone(),
            excluded_title_patterns: cfg.capture.excluded_title_patterns.clone(),
            always_on: cfg.capture.always_on.clone(),
        },
        browser: BrowserSettings {
            capture_tabs: cfg.browser.capture_tabs,
            excluded_url_patterns: cfg.browser.excluded_url_patterns.clone(),
            max_tabs_per_window: cfg.browser.max_tabs_per_window as u32,
        },
        retention: RetentionSettings {
            keep_last: cfg.retention.keep_last,
            max_age_days: cfg.retention.max_age_days,
            auto_prune: cfg.retention.auto_prune,
        },
        timeouts: TimeoutSettings {
            git_ms: cfg.timeouts.git_ms,
            restore_overall_secs: cfg.timeouts.restore_overall_secs,
        },
        app: AppSettings {
            notifications: cfg.app.notifications,
            close_to_tray: cfg.app.close_to_tray,
            start_with_windows,
            capture_hotkey: cfg.app.capture_hotkey.clone(),
            switcher_hotkey: cfg.app.switcher_hotkey.clone(),
            auto_save_on_switch: cfg.app.auto_save_on_switch,
            offer_resume: cfg.app.offer_resume,
            checkpoint_minutes: cfg.app.checkpoint_minutes,
            match_display_setup: cfg.app.match_display_setup,
        },
        log_level: cfg.logging.level.clone(),
    }
}

/// `base` with the settings the screen edits replaced. Everything the screen
/// doesn't show (database location, log file options) is kept.
fn apply_settings(base: &Config, s: &Settings) -> Config {
    let clean = |v: &[String]| -> Vec<String> {
        v.iter()
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect()
    };
    let mut cfg = base.clone();
    cfg.capture.include_windows = s.capture.include_windows;
    cfg.capture.excluded_processes = clean(&s.capture.excluded_processes);
    cfg.capture.excluded_title_patterns = clean(&s.capture.excluded_title_patterns);
    cfg.capture.always_on = clean(&s.capture.always_on);
    cfg.browser.capture_tabs = s.browser.capture_tabs;
    cfg.browser.excluded_url_patterns = clean(&s.browser.excluded_url_patterns);
    cfg.browser.max_tabs_per_window = s.browser.max_tabs_per_window as usize;
    cfg.retention.keep_last = s.retention.keep_last;
    cfg.retention.max_age_days = s.retention.max_age_days;
    cfg.retention.auto_prune = s.retention.auto_prune;
    cfg.timeouts.git_ms = s.timeouts.git_ms;
    cfg.timeouts.restore_overall_secs = s.timeouts.restore_overall_secs;
    cfg.app.notifications = s.app.notifications;
    cfg.app.close_to_tray = s.app.close_to_tray;
    cfg.app.capture_hotkey = s.app.capture_hotkey.trim().to_string();
    cfg.app.switcher_hotkey = s.app.switcher_hotkey.trim().to_string();
    cfg.app.auto_save_on_switch = s.app.auto_save_on_switch;
    cfg.app.offer_resume = s.app.offer_resume;
    cfg.app.checkpoint_minutes = s.app.checkpoint_minutes;
    cfg.app.match_display_setup = s.app.match_display_setup;
    cfg.logging.level = s.log_level.clone();
    cfg
}

#[tauri::command]
pub async fn get_settings(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Settings> {
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    Ok(settings_from(&state.config(), autostart))
}

/// Which global shortcuts are working right now, with a message for each that isn't.
#[tauri::command]
pub async fn get_hotkey_status(state: State<'_, AppState>) -> CmdResult<HotkeyStatusDto> {
    let s = state.hotkeys.lock().map(|s| s.clone()).unwrap_or_default();
    Ok(HotkeyStatusDto {
        capture: s.capture,
        switcher: s.switcher,
    })
}

/// Validate and save the settings to `config.toml`. Nothing is changed if the
/// values are not valid.
#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> CmdResult<Settings> {
    let next = apply_settings(&state.config(), &settings);
    next.validate().map_err(conflict)?;

    // Take the new shortcut first: if another program owns it, nothing is saved.
    let old = state.config().app;
    let hotkey_changed = next.app.capture_hotkey != old.capture_hotkey
        || next.app.switcher_hotkey != old.switcher_hotkey;
    if hotkey_changed {
        let status =
            crate::hotkey::register(&app, &next.app.capture_hotkey, &next.app.switcher_hotkey);
        // Only a shortcut you just changed can stop the save; one that was already
        // unavailable stays reported in Settings.
        let changed_problem = [
            (
                next.app.capture_hotkey != old.capture_hotkey,
                status.capture,
            ),
            (
                next.app.switcher_hotkey != old.switcher_hotkey,
                status.switcher,
            ),
        ]
        .into_iter()
        .find_map(|(changed, problem)| problem.filter(|_| changed));
        if let Some(problem) = changed_problem {
            // Put the old shortcuts back, so a failed change doesn't leave none.
            let _ = crate::hotkey::register(&app, &old.capture_hotkey, &old.switcher_hotkey);
            return Err(conflict(problem));
        }
    }
    state.save_config(next.clone()).map_err(|e| {
        if hotkey_changed {
            let _ = crate::hotkey::register(&app, &old.capture_hotkey, &old.switcher_hotkey);
        }
        CommandError::from(e)
    })?;

    let manager = app.autolaunch();
    let enabled = manager.is_enabled().unwrap_or(false);
    if settings.app.start_with_windows != enabled {
        let res = if settings.app.start_with_windows {
            manager.enable()
        } else {
            manager.disable()
        };
        res.map_err(|e| conflict(format!("could not change start with Windows: {e}")))?;
    }
    tracing::info!("settings saved");
    Ok(settings_from(
        &next,
        manager
            .is_enabled()
            .unwrap_or(settings.app.start_with_windows),
    ))
}

// ------------------------------------------------------------ browser setup

fn browser_status() -> BrowserStatus {
    use cf_browser::install;
    let s = install::status();
    let dir = cf_browser::default_state_dir();
    let browsers = cf_browser::BROWSERS
        .iter()
        .map(|(id, name)| {
            let states = dir
                .as_deref()
                .map(|d| cf_browser::read_live_states(d, id))
                .unwrap_or_default();
            let windows: usize = states.iter().map(|s| s.windows.len()).sum();
            let tabs: usize = states
                .iter()
                .flat_map(|s| &s.windows)
                .map(|w| w.tabs.len())
                .sum();
            BrowserLink {
                name: (*name).to_string(),
                registered: s
                    .registered
                    .iter()
                    .any(|(n, ok)| *ok && n.eq_ignore_ascii_case(name)),
                connected: !states.is_empty(),
                windows: windows as u32,
                tabs: tabs as u32,
            }
        })
        .collect();
    BrowserStatus {
        browsers,
        manifest_present: s.manifest_present,
        extension_unpacked: s.extension_unpacked,
        extension_dir: install::extension_dir()
            .ok()
            .map(|p| p.display().to_string()),
    }
}

#[tauri::command]
pub async fn get_browser_status() -> CmdResult<BrowserStatus> {
    blocking(|| Ok(browser_status())).await
}

/// Register the browser host for Chrome and Edge and unpack the extension.
/// Loading the extension in the browser is the one step only the user can do.
#[tauri::command]
pub async fn install_browser_integration() -> CmdResult<BrowserStatus> {
    blocking(|| {
        let host = std::env::current_exe()
            .map_err(conflict)?
            .with_file_name("contextflow-browser-host.exe");
        cf_browser::install::install(&host).map_err(|e| {
            conflict(format!(
                "{e}. The browser host is installed with ContextFlow; if you run the app from a build folder, build it first."
            ))
        })?;
        Ok(browser_status())
    })
    .await
}

#[tauri::command]
pub async fn remove_browser_integration() -> CmdResult<BrowserStatus> {
    blocking(|| {
        cf_browser::install::uninstall().map_err(conflict)?;
        Ok(browser_status())
    })
    .await
}

/// Open one of ContextFlow's folders in Explorer.
#[tauri::command]
pub async fn open_folder(state: State<'_, AppState>, which: String) -> CmdResult<()> {
    let dir: PathBuf = match which.as_str() {
        "extension" => cf_browser::install::extension_dir().map_err(conflict)?,
        "data" => state
            .paths
            .db
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
        "logs" => state.paths.logs.clone(),
        "backups" => state.paths.backups.clone(),
        other => return Err(conflict(format!("unknown folder '{other}'"))),
    };
    std::fs::create_dir_all(&dir).map_err(conflict)?;
    let mut cmd = std::process::Command::new("explorer.exe");
    cmd.arg(&dir);
    cmd.spawn().map_err(conflict)?;
    Ok(())
}

// ------------------------------------------------------------ data actions

fn picked(path: tauri_plugin_dialog::FilePath) -> CmdResult<PathBuf> {
    path.into_path().map_err(conflict)
}

/// Copy the database somewhere you choose and prove the copy is usable.
#[tauri::command]
pub async fn backup_now(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<FileResult>> {
    let db_path = state.paths.db.clone();
    let dir = state.paths.backups.clone();
    blocking(move || {
        std::fs::create_dir_all(&dir).map_err(conflict)?;
        let name = format!(
            "contextflow-{}.db",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let Some(dest) = app
            .dialog()
            .file()
            .set_title("Save a backup of your ContextFlow data")
            .set_directory(&dir)
            .set_file_name(name)
            .add_filter("ContextFlow database", &["db"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let dest = picked(dest)?;
        maintenance::backup_verified(&open(&db_path)?, &dest)?;
        Ok(Some(FileResult {
            message: "Backup written and checked.".into(),
            path: dest.display().to_string(),
        }))
    })
    .await
}

/// Write workspaces (all, or the named ones) to a file you choose.
#[tauri::command]
pub async fn export_workspaces(
    app: AppHandle,
    state: State<'_, AppState>,
    workspaces: Option<Vec<String>>,
) -> CmdResult<Option<FileResult>> {
    let db_path = state.paths.db.clone();
    blocking(move || {
        let db = open(&db_path)?;
        let contexts = match &workspaces {
            Some(names) => names
                .iter()
                .map(|n| db.find_context(n))
                .collect::<Result<Vec<_>, _>>()?,
            None => db.list_contexts(true)?,
        };
        if contexts.is_empty() {
            return Err(conflict("there are no workspaces to export"));
        }
        let suggested = match contexts.as_slice() {
            [one] => format!("{}.contextflow.json", cf_app::shortcut::file_safe_name(&one.name)),
            _ => "contextflow-workspaces.json".to_string(),
        };
        let Some(dest) = app
            .dialog()
            .file()
            .set_title("Export workspaces")
            .set_file_name(suggested)
            .add_filter("ContextFlow export", &["json"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let dest = picked(dest)?;
        // The dialog already asked about replacing an existing file.
        let summary = maintenance::export_workspaces(&db, &contexts, &dest, true)?;
        Ok(Some(FileResult {
            message: format!(
                "Exported {} workspace(s) and {} snapshot(s). The file contains window titles and repository paths, so store it with care.",
                summary.workspaces, summary.snapshots
            ),
            path: dest.display().to_string(),
        }))
    })
    .await
}

/// Import workspaces from an export file you choose. They arrive unapproved.
#[tauri::command]
pub async fn import_workspaces(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<Vec<ImportedWorkspace>>> {
    let db_path = state.paths.db.clone();
    blocking(move || {
        let Some(file) = app
            .dialog()
            .file()
            .set_title("Import workspaces")
            .add_filter("ContextFlow export", &["json"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let file = picked(file)?;
        let mut db = open(&db_path)?;
        let imported = maintenance::import_file(&mut db, &file, None)?;
        Ok(Some(
            imported
                .into_iter()
                .map(|w| ImportedWorkspace {
                    name: w.name,
                    snapshots: w.snapshots as u32,
                })
                .collect(),
        ))
    })
    .await
}

/// Check the database and re-hash every snapshot. Problems are reported, never
/// repaired.
#[tauri::command]
pub async fn verify_data(state: State<'_, AppState>) -> CmdResult<VerifyResult> {
    let path = state.paths.db.clone();
    blocking(move || {
        let r = maintenance::verify_all(&open(&path)?)?;
        Ok(VerifyResult {
            ok: r.ok(),
            database_problems: r.database_problems.clone(),
            snapshots_checked: r.snapshots_checked as u32,
            failures: r
                .failures
                .into_iter()
                .map(|f| VerifyFailure {
                    workspace: f.workspace,
                    version: f.version,
                })
                .collect(),
        })
    })
    .await
}

/// Which snapshots the retention rules would delete now. Deletes nothing.
#[tauri::command]
pub async fn preview_retention(state: State<'_, AppState>) -> CmdResult<RetentionResult> {
    let paths_state = state.inner();
    let (db, cfg) = (paths_state.paths.db.clone(), paths_state.config());
    blocking(move || prune_with(&db, &cfg, false)).await
}

/// Delete the snapshots the retention rules select. The UI shows the preview first.
#[tauri::command]
pub async fn apply_retention(state: State<'_, AppState>) -> CmdResult<RetentionResult> {
    let paths_state = state.inner();
    let (db, cfg) = (paths_state.paths.db.clone(), paths_state.config());
    blocking(move || prune_with(&db, &cfg, true)).await
}

fn prune_with(db_path: &Path, cfg: &Config, apply: bool) -> CmdResult<RetentionResult> {
    let policy = maintenance::retention_policy(cfg).ok_or_else(|| {
        conflict("set how many snapshots to keep, or how many days, in Settings first")
    })?;
    let mut db = open(db_path)?;
    let items = maintenance::prune_all(&mut db, &policy, apply)?;
    Ok(RetentionResult {
        items: items
            .into_iter()
            .map(|i| PruneItem {
                workspace: i.workspace,
                versions: i.versions,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_into_config_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut s = settings_from(&Config::default(), false);
        s.capture.excluded_processes = vec!["  discord ".into(), "".into(), "steam".into()];
        s.browser.excluded_url_patterns = vec!["mybank.com".into()];
        s.retention.keep_last = Some(20);
        s.retention.auto_prune = true;
        s.timeouts.restore_overall_secs = 300;
        s.app.close_to_tray = true;
        s.log_level = "debug".into();

        // A value the screen doesn't edit must survive.
        let mut base = Config::default();
        base.storage.database_path = Some("D:\\data\\cf.db".into());
        let cfg = apply_settings(&base, &s);
        cfg.validate().unwrap();
        std::fs::write(&path, cfg.to_toml()).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded, cfg);
        assert_eq!(loaded.capture.excluded_processes, vec!["discord", "steam"]);
        assert_eq!(loaded.storage.database_path, base.storage.database_path);
        // And the screen shows back what was saved.
        let mut expected = s.clone();
        expected.capture.excluded_processes = vec!["discord".into(), "steam".into()];
        assert_eq!(settings_from(&loaded, false), expected);
    }

    #[test]
    fn invalid_settings_are_refused() {
        let mut s = settings_from(&Config::default(), false);
        s.retention.keep_last = Some(0);
        assert!(apply_settings(&Config::default(), &s).validate().is_err());

        let mut s = settings_from(&Config::default(), false);
        s.retention.auto_prune = true; // needs a keep_last or max_age_days
        assert!(apply_settings(&Config::default(), &s).validate().is_err());

        let mut s = settings_from(&Config::default(), false);
        s.log_level = "loud".into();
        assert!(apply_settings(&Config::default(), &s).validate().is_err());
    }
}
