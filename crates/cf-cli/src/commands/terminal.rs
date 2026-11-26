//! `contextflow terminal hook ...`: the opt-in PowerShell hook that lets
//! ContextFlow see which folder each PowerShell tab is in.

use std::path::PathBuf;

use anyhow::{bail, Result};
use cf_app::terminal_hook::{self, Outcome};

use crate::{HookCmd, TerminalCmd};

/// The profile files to work on: the one asked for with `--profile`, or the
/// all-hosts profile of each PowerShell installed here.
fn targets(profile: &Option<PathBuf>) -> Result<Vec<PathBuf>> {
    if let Some(p) = profile {
        return Ok(vec![p.clone()]);
    }
    let found = terminal_hook::profile_paths();
    if found.is_empty() {
        bail!("couldn't find a PowerShell on this computer; name a profile file with --profile");
    }
    Ok(found)
}

pub(crate) fn terminal_cmd(cmd: &TerminalCmd) -> Result<()> {
    let TerminalCmd::Hook(hook) = cmd;
    match hook {
        HookCmd::Install { profile } => {
            let files = targets(profile)?;
            println!(
                "This adds a short, marked block to your PowerShell profile so ContextFlow can\n\
                 see which folder each PowerShell tab is in. It wraps your prompt and doesn't\n\
                 change what it shows. Remove it any time with `contextflow terminal hook uninstall`.\n"
            );
            for file in files {
                match terminal_hook::install_into(&file)? {
                    Outcome::Added => println!("added to      {}", file.display()),
                    _ => println!("already there {}", file.display()),
                }
            }
            println!("\nOpen a new PowerShell tab for it to take effect; tabs already open aren't changed.");
        }
        HookCmd::Uninstall { profile } => {
            for file in targets(profile)? {
                match terminal_hook::uninstall_from(&file)? {
                    Outcome::Removed => println!("removed from  {}", file.display()),
                    _ => println!("not there     {}", file.display()),
                }
            }
        }
        HookCmd::Status { profile } => {
            for file in targets(profile)? {
                let state = if terminal_hook::is_installed(&file) {
                    "installed"
                } else {
                    "not installed"
                };
                println!("{state:<14}{}", file.display());
            }
        }
    }
    Ok(())
}
