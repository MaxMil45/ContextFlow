//! The ContextFlow agent: the request handler both the background agent and
//! the in-process fallback run, and the client helper that picks between them.
//!
//! See `docs/AGENT.md`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;

use cf_app::session::{
    self, CleanupReport, LeaveOutcome, RestoreArgs, RestoreBusy, RestoreOutcome,
};
use cf_config::Config;
use cf_ipc::{
    CallError, Cleanup, CleanupOutcome, CleanupPhase, CleanupResult, ErrorCode, LeaveRequest,
    LeaveSummary, Problem, RequestBody, ResponseBody, RestoreRequest, RestoreSummary, Saved, Step,
    MIN_SUPPORTED, PROTOCOL_VERSION,
};
use cf_storage::Database;

pub mod lifecycle;

/// How long a client waits for a free pipe instance before working in-process.
pub const CONNECT_WAIT: Duration = Duration::from_secs(2);

/// Seconds to wait for each application's window unless the request says.
const DEFAULT_APP_TIMEOUT: u64 = 30;

/// Answers requests over the pipe, with the agent's own database connection.
pub struct Service {
    config_path: PathBuf,
    db: Mutex<Database>,
}

impl Service {
    pub fn new(config_path: PathBuf, db: Database) -> Self {
        Self {
            config_path,
            db: Mutex::new(db),
        }
    }

    /// Answer one request. Progress lines go to the log file and come back in
    /// the summary, since the client cannot watch them live yet.
    pub fn handle(&self, request: RequestBody) -> ResponseBody {
        // A poisoned lock only means an earlier request panicked; the
        // connection itself is still usable.
        let mut db = self.db.lock().unwrap_or_else(|p| p.into_inner());
        let mut lines = Vec::new();
        let mut reply = handle(&mut db, &self.config_path, request, &mut |m| {
            tracing::info!("{m}");
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

/// Answer one request against `db`. The agent and a front end working
/// in-process both end up here, so the two paths behave the same. `log`
/// receives progress lines as they happen.
pub fn handle(
    db: &mut Database,
    config_path: &Path,
    request: RequestBody,
    log: &mut dyn FnMut(&str),
) -> ResponseBody {
    match request {
        RequestBody::Hello { client } => {
            tracing::debug!("hello from {client}");
            ResponseBody::Hello {
                agent_version: env!("CARGO_PKG_VERSION").into(),
                protocol: PROTOCOL_VERSION,
                min_protocol: MIN_SUPPORTED,
            }
        }
        RequestBody::Restore(req) => {
            answer(restore(db, config_path, &req, log).map(ResponseBody::Restored))
        }
        RequestBody::Leave(req) => {
            answer(leave(db, config_path, &req, log).map(ResponseBody::Left))
        }
        RequestBody::Shutdown => ResponseBody::ShuttingDown,
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
    RestoreArgs {
        version: req.version,
        only: req.only.clone(),
        dry_run: false,
        timeout: Duration::from_secs(
            req.timeout_secs
                .unwrap_or(DEFAULT_APP_TIMEOUT)
                .clamp(1, 600),
        ),
        max_tabs_per_window: cfg.browser.max_tabs_per_window,
        overall_timeout: cfg.timeouts.restore_overall(),
        cancel: None,
        skip_cleanup: req.no_cleanup,
        always_on: cfg.capture.normalized_always_on(),
        leave_note: None,
        match_display: cfg.app.match_display_setup,
        // A switch saves where you are first, like every other way of switching.
        save_before_leaving: (!req.no_save).then(|| cfg.clone()),
    }
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

/// The agent's pipe for a front end, or `None` when the front end was given
/// its own database or config: the agent only knows the per-user ones, so
/// that work stays in-process.
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
    Agent,
    /// In this process, and why the agent was not used.
    InProcess(String),
}

/// Send `request` to the agent on `pipe`; if no agent can take it, run
/// `local` instead. `pipe: None` skips the agent (see [`pipe_for`]).
///
/// Once a side-effecting request may have reached the agent, a lost reply is
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
    tracing::debug!("agent not used ({reason}); working in-process");
    Ok((local(request)?, Via::InProcess(reason)))
}
