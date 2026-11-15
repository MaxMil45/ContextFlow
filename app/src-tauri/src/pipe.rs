//! The tray app serves this user's pipe, so the desktop icon and `contextflow`
//! hand their restores to it instead of each starting from scratch. With the
//! app closed they do the work themselves, exactly the same way.
//!
//! A switch handed over this way shows the same on-screen card as one started
//! from the switcher: what is coming back as it happens, then the result.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cf_app::session::Paths;
use cf_ipc::{RequestBody, ResponseBody, RestoreSummary};
use tauri::AppHandle;

/// Start serving in the background. Not when the app runs on a custom
/// database or config (`CONTEXTFLOW_DB` / `CONTEXTFLOW_CONFIG`): clients only
/// ask the pipe when they use the per-user ones, and must get those.
pub fn serve(app: &AppHandle, paths: &Paths) {
    if std::env::var_os("CONTEXTFLOW_DB").is_some()
        || std::env::var_os("CONTEXTFLOW_CONFIG").is_some()
    {
        tracing::info!("pipe not served: this app uses a custom database or config");
        return;
    }
    let (config, db_path, app) = (paths.config.clone(), paths.db.clone(), app.clone());
    std::thread::spawn(move || {
        let served = (|| -> anyhow::Result<()> {
            let name = cf_ipc::default_pipe_name()?;
            // Fails if the pipe already exists: an old `contextflow-agent` is
            // still running. The icon and CLI then keep using that one.
            let server = cf_ipc::Server::bind(&name)?;
            let db = cf_storage::Database::open(&db_path)?;
            tracing::info!("serving the desktop icon and the CLI over the pipe");
            // Only a switch shows the card; a `leave` from the CLI doesn't.
            let switching = Arc::new(AtomicBool::new(false));
            let (card_app, card_on) = (app.clone(), switching.clone());
            let service = cf_app::service::Service::new(config, db).with_progress(move |m| {
                if card_on.load(Ordering::SeqCst) {
                    crate::overlay::show(&card_app, "saving", "Switching…", m);
                }
            });
            server.run(move |request: RequestBody| {
                let restore = matches!(request, RequestBody::Restore(_));
                switching.store(restore, Ordering::SeqCst);
                let began = Instant::now();
                let reply = service.handle(request);
                switching.store(false, Ordering::SeqCst);
                if restore {
                    show_result(&app, &reply, began.elapsed());
                }
                reply
            })?;
            Ok(())
        })();
        match served {
            Ok(()) => tracing::info!("stopped serving the pipe"),
            Err(e) => tracing::warn!("not serving the pipe: {e:#}"),
        }
    });
}

/// The card a finished switch leaves on screen. `busy` shows nothing: the
/// switch already running shows its own.
fn show_result(app: &AppHandle, reply: &ResponseBody, took: Duration) {
    match reply {
        ResponseBody::Restored(s) => {
            let (kind, title, body) = result_card(s, took);
            crate::overlay::show(app, kind, &title, &body);
        }
        ResponseBody::Error(e) => {
            crate::overlay::show(app, "error", "The switch didn't happen", &e.message);
        }
        _ => {}
    }
}

/// "“Focus” is ready", "7 apps · 2.1 s · saved “Mail” v12".
fn result_card(s: &RestoreSummary, took: Duration) -> (&'static str, String, String) {
    let mut body = format!("{} apps · {:.1} s", s.steps.len(), took.as_secs_f64());
    if !s.problems.is_empty() {
        body.push_str(&format!(" · {} need attention", s.problems.len()));
    }
    if let Some(left) = &s.left {
        if let Some(v) = left.version {
            body.push_str(&format!(" · saved “{}” v{v}", left.workspace));
        }
    }
    let kind = if s.problems.is_empty() {
        "saved"
    } else {
        "error"
    };
    (kind, format!("“{}” is ready", s.workspace), body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cf_ipc::{Problem, Saved, Step};

    #[test]
    fn the_card_says_what_came_back() {
        let step = |a: &str| Step {
            application: a.into(),
            status: "success".into(),
            message: String::new(),
        };
        let mut s = RestoreSummary {
            workspace: "Focus".into(),
            steps: vec![step("code"), step("chrome")],
            left: Some(Saved {
                workspace: "Mail".into(),
                version: Some(12),
                not_saved: None,
            }),
            ..Default::default()
        };
        let (kind, title, body) = result_card(&s, Duration::from_millis(2100));
        assert_eq!(kind, "saved");
        assert_eq!(title, "“Focus” is ready");
        assert_eq!(body, "2 apps · 2.1 s · saved “Mail” v12");

        s.problems.push(Problem {
            subject: "code".into(),
            message: "not found".into(),
        });
        s.left = None;
        let (kind, _, body) = result_card(&s, Duration::from_millis(900));
        assert_eq!(
            (kind, body.as_str()),
            ("error", "2 apps · 0.9 s · 1 need attention")
        );
    }
}
