//! Health check: the things that commonly stop ContextFlow from working, each
//! with a plain answer and what to do about it. Shared by the desktop app's
//! Settings and `contextflow doctor`.

use std::path::Path;

use cf_config::Config;
use cf_storage::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// Works, but something is missing or worth a look.
    Warn,
    /// Something is broken.
    Bad,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Bad => "bad",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub message: String,
}

fn check(name: &str, status: Status, message: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        status,
        message: message.into(),
    }
}

/// Run every check. Never fails: a check that can't run reports why.
pub fn run(db_path: &Path, backups: &Path, _cfg: &Config) -> Vec<Check> {
    let mut out = Vec::new();

    // Database: opens, passes SQLite's own check, every snapshot re-hashes.
    match Database::open(db_path) {
        Err(e) => out.push(check(
            "Database",
            Status::Bad,
            format!("can't be opened: {e}"),
        )),
        Ok(db) => {
            match db.integrity_check() {
                Ok(problems) if problems.is_empty() => {
                    let count = db.list_contexts(true).map(|c| c.len()).unwrap_or(0);
                    out.push(check(
                        "Database",
                        Status::Ok,
                        format!(
                            "healthy · {count} workspace(s) · schema v{}",
                            Database::latest_schema_version()
                        ),
                    ));
                }
                Ok(problems) => out.push(check(
                    "Database",
                    Status::Bad,
                    format!(
                        "SQLite reports damage ({}). Restore a backup from Settings → Your data.",
                        problems.join("; ")
                    ),
                )),
                Err(e) => out.push(check(
                    "Database",
                    Status::Bad,
                    format!("couldn't be checked: {e}"),
                )),
            }
            out.push(recent_restores(&db));
            out.push(untrusted(&db));
        }
    }

    out.push(backups_check(backups));
    out.push(browser_check());

    let vscode = cf_vscode::default_storage_json().is_some_and(|p| p.is_file());
    out.push(check(
        "VS Code folders",
        Status::Ok,
        if vscode {
            "VS Code's list of folders was found, so its folders are saved."
        } else {
            "VS Code isn't set up on this computer (nothing to save)."
        },
    ));
    let studio = !cf_vstudio::default_settings_files().is_empty();
    out.push(check(
        "Visual Studio solutions",
        Status::Ok,
        if studio {
            "Visual Studio's list of recent solutions was found, so its solutions are saved."
        } else {
            "Visual Studio isn't set up on this computer (nothing to save)."
        },
    ));
    out
}

fn recent_restores(db: &Database) -> Check {
    let mut failing: Vec<String> = Vec::new();
    for ctx in db.list_contexts(false).unwrap_or_default() {
        if let Some(last) = db
            .recent_restore_runs(ctx.id, 1)
            .ok()
            .and_then(|r| r.into_iter().next())
        {
            if !last.problems.is_empty() {
                let apps: Vec<&str> = last.problems.iter().map(|(a, ..)| a.as_str()).collect();
                failing.push(format!("{} ({})", ctx.name, apps.join(", ")));
            }
        }
    }
    if failing.is_empty() {
        check(
            "Last restores",
            Status::Ok,
            "every workspace's last restore brought everything back",
        )
    } else {
        check(
            "Last restores",
            Status::Warn,
            format!(
                "some programs didn't come back last time: {}. Open the workspace's History tab for details.",
                failing.join("; ")
            ),
        )
    }
}

fn untrusted(db: &Database) -> Check {
    let names: Vec<String> = db
        .list_contexts(false)
        .unwrap_or_default()
        .into_iter()
        .filter(|c| !db.is_trusted(c.id).unwrap_or(true))
        .map(|c| c.name)
        .collect();
    if names.is_empty() {
        check(
            "Imported workspaces",
            Status::Ok,
            "none waiting for approval",
        )
    } else {
        check(
            "Imported workspaces",
            Status::Warn,
            format!(
                "waiting for your approval before they can restore: {}",
                names.join(", ")
            ),
        )
    }
}

fn backups_check(dir: &Path) -> Check {
    let newest = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "db"))
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .max();
    match newest {
        None => check(
            "Backups",
            Status::Warn,
            "no backup yet. Make one in Settings → Your data (or `contextflow backup`).",
        ),
        Some(t) => {
            let days = t.elapsed().map(|d| d.as_secs() / 86_400).unwrap_or(0);
            if days > 30 {
                check(
                    "Backups",
                    Status::Warn,
                    format!("the newest backup is {days} days old"),
                )
            } else {
                check(
                    "Backups",
                    Status::Ok,
                    format!("newest backup is {days} day(s) old"),
                )
            }
        }
    }
}

fn browser_check() -> Check {
    let s = cf_browser::install::status();
    let registered: Vec<&str> = s
        .registered
        .iter()
        .filter(|(_, ok)| *ok)
        .map(|(n, _)| *n)
        .collect();
    if registered.is_empty() || !s.extension_unpacked {
        return check(
            "Browser tabs",
            Status::Warn,
            "the browser helper isn't installed, so tabs aren't saved. Set it up in Settings → Capture → Browser tabs.",
        );
    }
    let dir = cf_browser::default_state_dir();
    let connected = cf_browser::BROWSERS.iter().any(|(id, _)| {
        dir.as_deref()
            .is_some_and(|d| !cf_browser::read_live_states(d, id).is_empty())
    });
    if connected {
        check(
            "Browser tabs",
            Status::Ok,
            format!("connected ({})", registered.join(", ")),
        )
    } else {
        check(
            "Browser tabs",
            Status::Warn,
            "the helper is installed but the extension isn't connected. Open Chrome or Edge and check the ContextFlow extension is loaded and turned on.",
        )
    }
}
