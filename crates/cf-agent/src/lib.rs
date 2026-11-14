//! The ContextFlow agent: the request handler both the background agent and
//! the in-process fallback run, and the client helper that picks between them.
//!
//! See `docs/AGENT.md`.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;

use cf_app::session::{self, RestoreArgs, RestoreBusy, RestoreOutcome};
use cf_config::Config;
use cf_ipc::{
    CallError, ErrorCode, Problem, RequestBody, ResponseBody, RestoreRequest, RestoreSummary,
    MIN_SUPPORTED, PROTOCOL_VERSION,
};
use cf_storage::Database;

/// How long a client waits for a free pipe instance before working in-process.
pub const CONNECT_WAIT: Duration = Duration::from_secs(2);

/// Answers requests. The agent serves it over the pipe; a front end with no
/// agent calls [`Service::handle`] directly, so both paths behave the same.
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

    pub fn handle(&self, request: RequestBody) -> ResponseBody {
        match request {
            RequestBody::Hello { client } => {
                tracing::debug!("hello from {client}");
                ResponseBody::Hello {
                    agent_version: env!("CARGO_PKG_VERSION").into(),
                    protocol: PROTOCOL_VERSION,
                    min_protocol: MIN_SUPPORTED,
                }
            }
            RequestBody::Restore(req) => self.restore(&req),
            RequestBody::Shutdown => ResponseBody::ShuttingDown,
        }
    }

    fn restore(&self, req: &RestoreRequest) -> ResponseBody {
        // Read the config each time: the desktop app may have changed it.
        let cfg = match Config::load(&self.config_path) {
            Ok(cfg) => cfg,
            Err(e) => return ResponseBody::error(ErrorCode::Failed, format!("{e:#}")),
        };
        let mut args = restore_args(&cfg);
        args.version = req.version;
        // A poisoned lock only means an earlier restore panicked; the
        // connection itself is still usable.
        let mut db = self.db.lock().unwrap_or_else(|p| p.into_inner());
        match session::restore_workspace(&mut db, &req.workspace, &args, &mut |m| {
            tracing::info!("{m}")
        }) {
            Ok(outcome) => ResponseBody::Restored(summarize(&outcome)),
            Err(e) if e.downcast_ref::<RestoreBusy>().is_some() => ResponseBody::Busy,
            Err(e) => ResponseBody::error(ErrorCode::Failed, format!("{e:#}")),
        }
    }
}

impl cf_ipc::Handler for Service {
    fn handle(&self, request: RequestBody) -> ResponseBody {
        Service::handle(self, request)
    }
}

/// How a desktop icon (or the agent on its behalf) switches workspace.
pub fn restore_args(cfg: &Config) -> RestoreArgs {
    RestoreArgs {
        version: None,
        only: Vec::new(),
        dry_run: false,
        timeout: Duration::from_secs(30),
        max_tabs_per_window: cfg.browser.max_tabs_per_window,
        overall_timeout: cfg.timeouts.restore_overall(),
        cancel: None,
        skip_cleanup: false,
        always_on: cfg.capture.normalized_always_on(),
        leave_note: None,
        match_display: cfg.app.match_display_setup,
        // A switch saves where you are first, like every other way of switching.
        save_before_leaving: Some(cfg.clone()),
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
/// `local` instead. `pipe: None` skips the agent (e.g. the caller was given a
/// `--db` or `--config` the agent does not use).
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
