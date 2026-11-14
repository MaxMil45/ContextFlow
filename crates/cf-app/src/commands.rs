//! Running a workspace's start and stop commands.
//!
//! A command is a program plus arguments, started directly (never through a
//! shell, so nothing in an argument is ever interpreted). The program is a full
//! path or a name found on PATH, including `.cmd`/`.bat` tools like `npm`.
//!
//! * Commands that don't wait open in their own console window, so a dev
//!   server can be seen and stopped like any other window.
//! * Commands that wait run hidden, for up to a minute, and report how they
//!   ended. One still running after that is left running and reported.
//!
//! Nothing is ever elevated: a program that needs administrator rights fails
//! to start and says so.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cf_core::CommandSpec;

/// Longest a waiting command is waited for.
const WAIT_LIMIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    /// `start` (entering) or `stop` (leaving).
    pub phase: &'static str,
    pub workspace: String,
    /// The command line as the user wrote it, for the report.
    pub command: String,
    pub ok: bool,
    pub message: String,
}

/// The command line as shown to the user.
pub fn display(spec: &CommandSpec) -> String {
    std::iter::once(spec.program.as_str())
        .chain(spec.args.iter().map(String::as_str))
        .map(|a| {
            if a.contains(' ') {
                format!("\"{a}\"")
            } else {
                a.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A program name or path, resolved the way a command prompt would (PATH and
/// PATHEXT), but without running a shell.
pub fn resolve_program(program: &str) -> Option<PathBuf> {
    let p = Path::new(program);
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|e| !e.is_empty())
        .map(|e| e.to_lowercase())
        .collect();
    let with_exts = |base: &Path| -> Option<PathBuf> {
        if base.extension().is_some() && base.is_file() {
            return Some(base.to_path_buf());
        }
        exts.iter()
            .map(|e| PathBuf::from(format!("{}{e}", base.display())))
            .find(|c| c.is_file())
    };
    if p.is_absolute() || program.contains('\\') || program.contains('/') {
        return with_exts(p);
    }
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .find_map(|dir| with_exts(&dir.join(program)))
}

/// Run one command.
pub fn run(spec: &CommandSpec, workspace: &str, phase: &'static str) -> CommandResult {
    let result = |ok: bool, message: String| CommandResult {
        phase,
        workspace: workspace.to_string(),
        command: display(spec),
        ok,
        message,
    };
    let Some(program) = resolve_program(&spec.program) else {
        return result(
            false,
            format!("couldn't find the program “{}”", spec.program),
        );
    };
    let mut cmd = Command::new(&program);
    cmd.args(&spec.args).stdin(Stdio::null());
    if let Some(folder) = &spec.folder {
        if !Path::new(folder).is_dir() {
            return result(false, format!("the folder {folder} doesn't exist"));
        }
        cmd.current_dir(folder);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        cmd.creation_flags(if spec.wait {
            CREATE_NO_WINDOW
        } else {
            CREATE_NEW_CONSOLE
        });
    }
    if spec.wait {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.raw_os_error() == Some(740) => {
            return result(
                false,
                "it needs administrator rights, and ContextFlow never elevates".into(),
            )
        }
        Err(e) => return result(false, format!("couldn't start it: {e}")),
    };
    if !spec.wait {
        tracing::info!(workspace, phase, "command started");
        return result(true, "started in its own window".into());
    }
    let began = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                return result(
                    true,
                    format!("finished in {:.1} s", began.elapsed().as_secs_f32()),
                )
            }
            Ok(Some(status)) => {
                return result(
                    false,
                    format!(
                        "ended with exit code {}",
                        status.code().map_or("?".into(), |c| c.to_string())
                    ),
                )
            }
            Ok(None) if began.elapsed() >= WAIT_LIMIT => {
                return result(false, "still running after a minute; left running".into())
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return result(false, format!("couldn't wait for it: {e}")),
        }
    }
}

/// Run a list of commands in order, logging each.
pub fn run_all(
    specs: &[CommandSpec],
    workspace: &str,
    phase: &'static str,
    log: &mut dyn FnMut(&str),
) -> Vec<CommandResult> {
    specs
        .iter()
        .map(|s| {
            log(&format!(
                "running {phase} command for “{workspace}”: {}",
                display(s)
            ));
            run(s, workspace, phase)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(program: &str, args: &[&str], wait: bool) -> CommandSpec {
        CommandSpec {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            folder: None,
            wait,
        }
    }

    #[test]
    fn programs_are_found_on_path_with_their_extension() {
        let cmd = resolve_program("cmd").expect("cmd.exe is on PATH");
        assert!(cmd.to_string_lossy().to_lowercase().ends_with("cmd.exe"));
        assert!(resolve_program("surely-not-a-real-program-xyz").is_none());
    }

    #[test]
    fn a_waiting_command_reports_how_it_ended() {
        let ok = run(&spec("cmd", &["/d", "/c", "exit", "0"], true), "W", "stop");
        assert!(ok.ok, "{ok:?}");
        let fail = run(&spec("cmd", &["/d", "/c", "exit", "3"], true), "W", "stop");
        assert!(!fail.ok);
        assert!(fail.message.contains('3'), "{}", fail.message);
        let missing = run(
            &spec("surely-not-a-real-program-xyz", &[], true),
            "W",
            "stop",
        );
        assert!(!missing.ok && missing.message.contains("couldn't find"));
    }

    #[test]
    fn the_command_line_is_shown_as_written() {
        assert_eq!(
            display(&spec("npm", &["run", "dev server"], false)),
            "npm run \"dev server\""
        );
    }
}
