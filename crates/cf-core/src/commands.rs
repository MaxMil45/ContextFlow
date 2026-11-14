//! A workspace's start and stop commands: programs run when you enter it
//! (start a dev server) or leave it (stop containers). Pure data and
//! validation; running them is `cf-app::commands`.
//!
//! These are code execution by design, so they are deliberately narrow: a
//! program and its arguments, never a shell line; they live only in this
//! database (never exported or imported); and they never run for a workspace
//! that hasn't been approved.

use serde::{Deserialize, Serialize};

use crate::{CoreError, Result};

/// Most commands one side may hold.
pub const MAX_COMMANDS: usize = 10;
const MAX_ARGS: usize = 32;
const MAX_TEXT: usize = 1024;

/// One program to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    /// The program: a full path, or a name found on PATH (`npm`, `docker`).
    pub program: String,
    /// Arguments, one per item: never parsed by a shell.
    #[serde(default)]
    pub args: Vec<String>,
    /// The folder it runs in.
    #[serde(default)]
    pub folder: Option<String>,
    /// Wait for it to finish (hidden), up to a minute, instead of opening it in
    /// its own console window and carrying on. Stop commands usually wait.
    #[serde(default)]
    pub wait: bool,
}

/// A workspace's start and stop commands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCommands {
    #[serde(default)]
    pub on_enter: Vec<CommandSpec>,
    #[serde(default)]
    pub on_leave: Vec<CommandSpec>,
}

fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::InvalidCleanup(msg.into())
}

fn clean_text(s: &str, what: &str) -> Result<String> {
    let t = s.trim();
    if t.chars().count() > MAX_TEXT || t.chars().any(char::is_control) {
        return Err(bad(format!(
            "{what} is too long or contains control characters"
        )));
    }
    Ok(t.to_string())
}

impl CommandSpec {
    fn normalized(self) -> Result<Self> {
        let program = clean_text(&self.program, "a command's program")?;
        if program.is_empty() {
            return Err(bad("a command needs a program"));
        }
        if program.starts_with('-') {
            return Err(bad("a program can't start with '-'"));
        }
        if self.args.len() > MAX_ARGS {
            return Err(bad(format!(
                "a command can have at most {MAX_ARGS} arguments"
            )));
        }
        let args = self
            .args
            .iter()
            .map(|a| clean_text(a, "an argument"))
            .collect::<Result<Vec<_>>>()?;
        let folder = match self
            .folder
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
        {
            Some(f) => Some(clean_text(f, "a command's folder")?),
            None => None,
        };
        Ok(Self {
            program,
            args,
            folder,
            wait: self.wait,
        })
    }
}

impl WorkspaceCommands {
    pub fn is_empty(&self) -> bool {
        self.on_enter.is_empty() && self.on_leave.is_empty()
    }

    /// Checked and tidied, ready to store.
    pub fn normalized(self) -> Result<Self> {
        for (side, list) in [("start", &self.on_enter), ("stop", &self.on_leave)] {
            if list.len() > MAX_COMMANDS {
                return Err(bad(format!("at most {MAX_COMMANDS} {side} commands")));
            }
        }
        Ok(Self {
            on_enter: self
                .on_enter
                .into_iter()
                .map(CommandSpec::normalized)
                .collect::<Result<_>>()?,
            on_leave: self
                .on_leave
                .into_iter()
                .map(CommandSpec::normalized)
                .collect::<Result<_>>()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(program: &str, args: &[&str]) -> CommandSpec {
        CommandSpec {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            folder: Some("  C:\\work\\app ".into()),
            wait: false,
        }
    }

    #[test]
    fn commands_are_tidied_and_checked() {
        let ok = WorkspaceCommands {
            on_enter: vec![spec(" npm ", &["run", "dev"])],
            on_leave: vec![],
        }
        .normalized()
        .unwrap();
        assert_eq!(ok.on_enter[0].program, "npm");
        assert_eq!(ok.on_enter[0].folder.as_deref(), Some("C:\\work\\app"));

        for bad in [spec("", &[]), spec("--x", &[]), spec("npm", &["a\u{0}b"])] {
            let c = WorkspaceCommands {
                on_enter: vec![bad],
                on_leave: vec![],
            };
            assert!(c.normalized().is_err());
        }
        let many = WorkspaceCommands {
            on_enter: vec![spec("npm", &[]); MAX_COMMANDS + 1],
            on_leave: vec![],
        };
        assert!(many.normalized().is_err());
    }
}
