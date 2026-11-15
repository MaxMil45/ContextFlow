//! `contextflow cleanup ...` and `contextflow leave`

use anyhow::{bail, Result};

use cf_cleanup::{PlannedAction, WindowsProcesses};
use cf_cli::session::{self, CleanupPhase};
use cf_core::{cleanup::normalize_process_name, CleanupTarget, CloseMode, Context};
use cf_storage::Database;

use crate::CleanupCmd;

fn mb(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn mode_word(mode: CloseMode) -> &'static str {
    match mode {
        CloseMode::Graceful => "close",
        CloseMode::Force => "force close",
    }
}

pub(crate) fn print_plan(heading: &str, plan: &cf_cleanup::CleanupPlan) {
    println!("{heading}");
    if plan.items.is_empty() {
        println!("  (nothing listed)");
        return;
    }
    for item in &plan.items {
        let what = match &item.action {
            PlannedAction::Close {
                process_count,
                ram_bytes,
                cpu_percent,
            } => format!(
                "would {}: {process_count} process(es), {}, {cpu_percent:.1}% CPU",
                mode_word(item.mode),
                mb(*ram_bytes)
            ),
            PlannedAction::NotRunning => "not running".into(),
            PlannedAction::Protected { reason } => format!("never closed ({reason})"),
            PlannedAction::KeptByWorkspace => "kept: the workspace uses it".into(),
        };
        println!("  {:<24} {what}", item.process);
    }
    println!(
        "  Would give back about {} and {:.1}% CPU.",
        mb(plan.ram_bytes),
        plan.cpu_percent
    );
}

/// For `restore --dry-run`: what the cleanups of this restore would do.
pub(crate) fn print_restore_previews(db: &Database, ctx: &Context, always_on: &[String]) {
    if let Ok(Some(previous)) = db.active_context().map(|a| a.filter(|p| *p != ctx.id)) {
        if let Ok(prev) = db.find_context(&previous.to_string()) {
            if let Ok(plan) =
                session::preview_cleanup(db, &prev.name, CleanupPhase::Leaving, always_on)
            {
                if !plan.items.is_empty() {
                    print_plan(&format!("Cleanup on leaving '{}':", prev.name), &plan);
                }
            }
        }
    }
    if let Ok(plan) = session::preview_cleanup(db, &ctx.name, CleanupPhase::Entering, always_on) {
        if !plan.items.is_empty() {
            print_plan(&format!("Cleanup on entering '{}':", ctx.name), &plan);
        }
    }
}

pub(crate) fn print_cleanup(report: &cf_ipc::Cleanup) {
    use cf_ipc::{CleanupOutcome as O, CleanupPhase as P};
    let when = match report.phase {
        P::Leaving => "leaving",
        P::Entering => "entering",
    };
    println!("Cleanup on {when} '{}':", report.workspace);
    for r in &report.results {
        let mark = match r.outcome {
            O::Closed => "closed ",
            O::Skipped => "skipped",
            O::Refused => "refused",
            O::NeedsAttention | O::Other => "ACTION ",
        };
        println!("  {mark} {:<22} {}", r.process, r.message);
    }
}

/// Leave the active workspace: through the running ContextFlow app when there is one
/// (`pipe`), otherwise here.
pub(crate) fn leave(
    db: &mut Database,
    paths: &session::Paths,
    pipe: Option<&str>,
    note: Option<&str>,
) -> Result<()> {
    use cf_ipc::{LeaveRequest, RequestBody, ResponseBody};
    let request = RequestBody::Leave(LeaveRequest {
        note: note.map(str::to_owned),
    });
    let (reply, _via) = cf_app::service::call_or_fallback(pipe, request, |request| {
        Ok(cf_app::service::handle(
            db,
            &paths.config,
            request,
            &mut |m| println!("  {m}..."),
        ))
    })?;
    let out = match reply {
        ResponseBody::Left(out) => out,
        ResponseBody::Error(e) => anyhow::bail!("{}", e.message),
        other => anyhow::bail!("unexpected reply from the ContextFlow app: {other:?}"),
    };
    super::util::print_log(&out.log);
    let Some(name) = out.workspace else {
        println!("No workspace is active.");
        return Ok(());
    };
    match (out.saved.version, &out.saved.not_saved) {
        (Some(v), _) => println!("Saved '{name}' as v{v}."),
        (None, Some(why)) => println!("'{name}' was not saved: {why}"),
        (None, None) => {}
    }
    match &out.cleanup {
        Some(report) => print_cleanup(report),
        None => println!("Left '{name}'. It has no 'when leaving' rules."),
    }
    Ok(())
}

pub(crate) fn cleanup_cmd(db: &Database, cmd: CleanupCmd, always_on: &[String]) -> Result<()> {
    match cmd {
        CleanupCmd::Show { workspace } => {
            let ctx = db.find_context(&workspace)?;
            let config = db.cleanup_config(ctx.id)?;
            if config.is_empty() {
                println!("'{}' has no cleanup rules.", ctx.name);
                println!(
                    "Add one with: contextflow cleanup add \"{}\" <program>",
                    ctx.name
                );
                return Ok(());
            }
            for (phase, heading) in [
                (CleanupPhase::Entering, "When entering"),
                (CleanupPhase::Leaving, "When leaving"),
            ] {
                let plan = session::preview_cleanup(db, &ctx.name, phase, always_on)?;
                if !plan.items.is_empty() {
                    print_plan(&format!("{heading} '{}':", ctx.name), &plan);
                }
            }
        }
        CleanupCmd::Add {
            workspace,
            process,
            on_exit,
            force,
        } => {
            let ctx = db.find_context(&workspace)?;
            let process = normalize_process_name(&process)?;
            if let Some(reason) = cf_cleanup::protected_reason(&format!("{process}.exe"), None) {
                bail!("'{process}' can't be closed by a workspace: {reason}");
            }
            let mut config = db.cleanup_config(ctx.id)?;
            let list = if on_exit {
                &mut config.on_exit
            } else {
                &mut config.on_enter
            };
            list.retain(|t| t.process != process);
            list.push(CleanupTarget {
                process: process.clone(),
                mode: if force {
                    CloseMode::Force
                } else {
                    CloseMode::Graceful
                },
            });
            db.set_cleanup_config(ctx.id, config)?;
            println!(
                "'{}' will {} {process} when {}.",
                ctx.name,
                if force { "force close" } else { "close" },
                if on_exit { "left" } else { "entered" }
            );
        }
        CleanupCmd::Remove {
            workspace,
            process,
            on_exit,
        } => {
            let ctx = db.find_context(&workspace)?;
            let process = normalize_process_name(&process)?;
            let mut config = db.cleanup_config(ctx.id)?;
            let list = if on_exit {
                &mut config.on_exit
            } else {
                &mut config.on_enter
            };
            let before = list.len();
            list.retain(|t| t.process != process);
            if list.len() == before {
                bail!("{process} isn't in that list");
            }
            db.set_cleanup_config(ctx.id, config)?;
            println!("Removed {process}.");
        }
        CleanupCmd::Apps => {
            let apps =
                cf_cleanup::running_apps(&WindowsProcesses, std::time::Duration::from_millis(600));
            println!(
                "{:<28} {:>5} {:>10} {:>6}",
                "PROGRAM", "COUNT", "MEMORY", "CPU %"
            );
            for a in apps {
                println!(
                    "{:<28} {:>5} {:>10} {:>6.1}{}",
                    a.process,
                    a.process_count,
                    mb(a.ram_bytes),
                    a.cpu_percent,
                    a.protected
                        .map(|r| format!("  (never closed: {r})"))
                        .unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}
