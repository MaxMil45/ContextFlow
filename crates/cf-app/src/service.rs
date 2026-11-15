//! Requests over this user's pipe: the handler the desktop app serves them
//! with, and the client helper the desktop icon and `contextflow` use to hand
//! their work to the running app, or do it themselves when it isn't running.
//! Both paths end in [`handle`], so they behave the same.
//!
//! See `docs/AGENT.md` for the protocol and its security.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;

use crate::session::{self, CleanupReport, LeaveOutcome, RestoreArgs, RestoreBusy, RestoreOutcome};
use cf_config::Config;
use cf_ipc::{
    CallError, Cleanup, CleanupOutcome, CleanupPhase, CleanupResult, ErrorCode, LeaveRequest,
    LeaveSummary, Problem, RequestBody, ResponseBody, RestoreRequest, RestoreSummary, Saved, Step,
    MIN_SUPPORTED, PROTOCOL_VERSION,
};
use cf_storage::Database;

/// How long a client waits for a free pipe instance before working in-process.
pub const CONNECT_WAIT: Duration = Duration::from_secs(2);

/// Answers requests over the pipe, with its own database connection.
pub struct Service {
    config_path: PathBuf,
    db: Mutex<Database>,
    /// Also told each progress line as it happens (the app shows them on screen).
    on_progress: Option<Box<Progress>>,
}

/// Something told each progress line of a request as it happens.
type Progress = dyn Fn(&str) + Send + Sync;

impl Service {
    pub fn new(config_path: PathBuf, db: Database) -> Self {
        Self {
            config_path,
            db: Mutex::new(db),
            on_progress: None,
        }
    }

    /// Call `f` with each progress line as it happens.
    pub fn with_progress(mut self, f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.on_progress = Some(Box::new(f));
        self
    }

    /// Answer one request. Progress lines go to the log file, to the progress
    /// callback, and back in the summary (the client cannot watch them live).
    pub fn handle(&self, request: RequestBody) -> ResponseBody {
        // These must answer at once even while a restore holds the database.
        if matches!(request, RequestBody::Hello { .. } | RequestBody::Shutdown) {
            return answer_without_db(request);
        }
        // A poisoned lock only means an earlier request panicked; the
        // connection itself is still usable.
        let mut db = self.db.lock().unwrap_or_else(|p| p.into_inner());
        let mut lines = Vec::new();
        let mut reply = handle(&mut db, &self.config_path, request, &mut |m| {
            tracing::info!("{m}");
            if let Some(f) = &self.on_progress {
                f(m);
            }
            lines.push(m.to_owned());
        });
        match &mut reply {
            ResponseBody::Restored(s) => s.log = lines,
            ResponseBody::Left(s) => s.log = lines,
            _ => {}
        }
        reply
    }
}

impl cf_ipc::Handler for Service {
    fn handle(&self, request: RequestBody) -> ResponseBody {
        Service::handle(self, request)
    }
}

/// Answer one request against `db`. The app serving the pipe and a front end
/// working in-process both end up here, so the two paths behave the same.
/// `log` receives progress lines as they happen.
pub fn handle(
    db: &mut Database,
    config_path: &Path,
    request: RequestBody,
    log: &mut dyn FnMut(&str),
) -> ResponseBody {
    match request {
        RequestBody::Hello { .. } | RequestBody::Shutdown => answer_without_db(request),
        RequestBody::Restore(req) => {
            answer(restore(db, config_path, &req, log).map(ResponseBody::Restored))
        }
        RequestBody::Leave(req) => {
            answer(leave(db, config_path, &req, log).map(ResponseBody::Left))
        }
    }
}

fn answer_without_db(request: RequestBody) -> ResponseBody {
    match request {
        RequestBody::Hello { client } => {
            tracing::debug!("hello from {client}");
            ResponseBody::Hello {
                agent_version: env!("CARGO_PKG_VERSION").into(),
                protocol: PROTOCOL_VERSION,
                min_protocol: MIN_SUPPORTED,
            }
        }
        RequestBody::Shutdown => ResponseBody::ShuttingDown,
        _ => ResponseBody::error(ErrorCode::BadRequest, "this request needs the database"),
    }
}

fn answer(result: Result<ResponseBody>) -> ResponseBody {
    match result {
        Ok(reply) => reply,
        Err(e) if e.downcast_ref::<RestoreBusy>().is_some() => ResponseBody::Busy,
        Err(e) => ResponseBody::error(ErrorCode::Failed, format!("{e:#}")),
    }
}

fn restore(
    db: &mut Database,
    config_path: &Path,
    req: &RestoreRequest,
    log: &mut dyn FnMut(&str),
) -> Result<RestoreSummary> {
    // Read the config each time: the desktop app may have changed it.
    let cfg = Config::load(config_path)?;
    let outcome = session::restore_workspace(db, &req.workspace, &restore_args(&cfg, req), log)?;
    Ok(summarize(&outcome))
}

fn leave(
    db: &mut Database,
    config_path: &Path,
    req: &LeaveRequest,
    log: &mut dyn FnMut(&str),
) -> Result<LeaveSummary> {
    let cfg = Config::load(config_path)?;
    let outcome = session::leave_active_workspace(db, Some(&cfg), req.note.as_deref(), log)?;
    Ok(summarize_leave(outcome))
}

/// The restore a request asks for, with everything else from the config.
pub fn restore_args(cfg: &Config, req: &RestoreRequest) -> RestoreArgs {
    let mut args = RestoreArgs::new(cfg, req.timeout_secs);
    args.version = req.version;
    args.only = req.only.clone();
    args.skip_cleanup = req.no_cleanup;
    if req.no_save {
        args.save_before_leaving = None;
    }
    args
}

fn cleanup_summary(report: &CleanupReport) -> Cleanup {
    use cf_cleanup::Outcome as O;
    Cleanup {
        phase: match report.phase {
            session::CleanupPhase::Leaving => CleanupPhase::Leaving,
            session::CleanupPhase::Entering => CleanupPhase::Entering,
        },
        workspace: report.workspace.clone(),
        results: report
            .results
            .iter()
            .map(|r| CleanupResult {
                process: r.process.clone(),
                outcome: match r.outcome {
                    O::Closed | O::Terminated => CleanupOutcome::Closed,
                    O::NotRunning | O::KeptByWorkspace => CleanupOutcome::Skipped,
                    O::Protected => CleanupOutcome::Refused,
                    O::StillRunning | O::Failed => CleanupOutcome::NeedsAttention,
                },
                message: r.message.clone(),
            })
            .collect(),
    }
}

/// What a front end needs to tell the user about a restore.
pub fn summarize(outcome: &RestoreOutcome) -> RestoreSummary {
    let mut problems: Vec<Problem> = outcome
        .run
        .problems()
        .map(|s| Problem {
            subject: s.application_id.clone(),
            message: s.message.clone(),
        })
        .collect();
    // Programs the workspace asked to close but that stayed open or couldn't be ended.
    for cleanup in &outcome.cleanups {
        problems.extend(cleanup.problems().map(|r| Problem {
            subject: format!("{} (cleanup)", r.process),
            message: r.message.clone(),
        }));
    }
    RestoreSummary {
        workspace: outcome.context.name.clone(),
        problems,
        layout_note: outcome.layout_note.clone(),
        version: outcome.snapshot.version,
        overall: outcome.run.overall.as_str().to_owned(),
        steps: outcome
            .run
            .steps
            .iter()
            .map(|s| Step {
                application: s.application_id.clone(),
                status: s.status.as_str().to_owned(),
                message: s.message.clone(),
            })
            .collect(),
        left: outcome.left.from.as_ref().map(|from| Saved {
            workspace: from.clone(),
            version: outcome.left.saved_version,
            not_saved: outcome.left.note.clone(),
        }),
        cleanups: outcome.cleanups.iter().map(cleanup_summary).collect(),
        note: outcome.snapshot.payload.notes.last().cloned(),
        log: Vec::new(),
    }
}

fn summarize_leave(outcome: LeaveOutcome) -> LeaveSummary {
    LeaveSummary {
        saved: Saved {
            workspace: outcome.workspace.clone().unwrap_or_default(),
            version: outcome.saved.saved_version,
            not_saved: outcome.saved.note,
        },
        workspace: outcome.workspace,
        cleanup: outcome.cleanup.as_ref().map(cleanup_summary),
        log: Vec::new(),
    }
}

/// The pipe a front end asks, or `None` when it was given its own database
/// or config: the app serves only the per-user ones, so that work stays
/// in-process.
pub fn pipe_for(custom_paths: bool) -> Option<String> {
    if custom_paths {
        None
    } else {
        cf_ipc::default_pipe_name().ok()
    }
}

/// Where a request was answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Via {
    /// By the running desktop app.
    Agent,
    /// In this process, and why the app was not used.
    InProcess(String),
}

/// Send `request` to the app on `pipe`; if it can't take it (not running,
/// or an older build), run `local` instead. `pipe: None` skips the app (see
/// [`pipe_for`]).
///
/// Once a side-effecting request may have reached the app, a lost reply is
/// an error, never a reason to run it again here.
pub fn call_or_fallback(
    pipe: Option<&str>,
    request: RequestBody,
    local: impl FnOnce(RequestBody) -> Result<ResponseBody>,
) -> Result<(ResponseBody, Via)> {
    let reason = match pipe {
        None => "skipped (custom database or config)".to_string(),
        Some(name) => match cf_ipc::request(name, request.clone(), CONNECT_WAIT) {
            Ok(reply) => return Ok((reply, Via::Agent)),
            Err(CallError::Unavailable(why)) => why,
            Err(e @ CallError::Failed(_)) => return Err(e.into()),
        },
    };
    tracing::debug!("the app did not take it ({reason}); working in-process");
    Ok((local(request)?, Via::InProcess(reason)))
}
