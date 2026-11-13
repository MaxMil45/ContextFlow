//! Create a desktop shortcut that restores a workspace with a double-click.
//!
//! The shortcut starts `contextflow-launch.exe` (a windowless program) with the
//! workspace id, so no console window flashes and renaming the workspace later
//! does not break it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context as _, Result};

const LAUNCHER: &str = "contextflow-launch.exe";

/// Where the launcher is: next to the running program (`contextflow.exe` or
/// the desktop app), else on `PATH` (e.g. `~/.cargo/bin` after
/// `cargo install`).
pub fn launcher_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("could not locate the running program")?;
    let beside = exe.with_file_name(LAUNCHER);
    if beside.is_file() {
        return Ok(beside);
    }
    if let Some(found) = std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(LAUNCHER))
        .find(|p| p.is_file())
    {
        return Ok(found);
    }
    bail!(
        "{LAUNCHER} was not found next to {} or on PATH. Install it with: cargo install --path crates/cf-cli",
        exe.display()
    )
}

/// Returned by [`create`] when a shortcut with that name is already there and
/// `overwrite` is off.
#[derive(Debug)]
pub struct ShortcutExists(pub PathBuf);

impl std::fmt::Display for ShortcutExists {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} already exists; use --force to replace it",
            self.0.display()
        )
    }
}

impl std::error::Error for ShortcutExists {}

/// Make a workspace name safe to use as a Windows file name.
pub fn file_safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim().to_string();
    if cleaned.is_empty() {
        "ContextFlow".into()
    } else {
        cleaned
    }
}

/// Quote one argument for a Windows command line (rejects embedded quotes).
fn quote_arg(a: &str) -> Result<String> {
    if a.contains('"') || a.contains('\0') {
        bail!("'{a}' contains characters that cannot be used in a shortcut");
    }
    Ok(format!("\"{a}\""))
}

pub struct ShortcutSpec<'a> {
    pub workspace_id: &'a str,
    pub display_name: &'a str,
    pub folder: Option<&'a Path>,
    pub db: Option<&'a Path>,
    pub config: Option<&'a Path>,
    pub overwrite: bool,
    /// `.ico` to show (see [`crate::icons`]); `None` = the launcher's own icon.
    pub icon: Option<&'a Path>,
}

/// Create the shortcut and return its path.
pub fn create(spec: &ShortcutSpec<'_>) -> Result<PathBuf> {
    let launcher = launcher_path()?;
    let mut args = vec![quote_arg(spec.workspace_id)?];
    if let Some(db) = spec.db {
        args.push("--db".into());
        args.push(quote_arg(&db.display().to_string())?);
    }
    if let Some(cfg) = spec.config {
        args.push("--config".into());
        args.push(quote_arg(&cfg.display().to_string())?);
    }

    // Values go in through the environment, never spliced into the script text.
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$dir = if ($env:CF_FOLDER) { $env:CF_FOLDER } else { [Environment]::GetFolderPath('Desktop') }
if (-not (Test-Path -LiteralPath $dir)) { throw "folder not found: $dir" }
$path = Join-Path $dir ($env:CF_NAME + '.lnk')
if ((Test-Path -LiteralPath $path) -and ($env:CF_OVERWRITE -ne '1')) { [Console]::Out.Write("EXISTS:$path"); exit 3 }
$s = (New-Object -ComObject WScript.Shell).CreateShortcut($path)
$s.TargetPath = $env:CF_TARGET
$s.Arguments = $env:CF_ARGS
$s.WorkingDirectory = Split-Path -Parent $env:CF_TARGET
$s.IconLocation = $(if ($env:CF_ICON) { $env:CF_ICON } else { $env:CF_TARGET }) + ',0'
$s.Description = 'Restore ' + $env:CF_NAME + ' with ContextFlow'
$s.Save()
[Console]::Out.Write($path)
"#;
    let mut cmd = powershell(SCRIPT);
    cmd.env("CF_NAME", file_safe_name(spec.display_name))
        .env("CF_TARGET", &launcher)
        .env("CF_ARGS", args.join(" "))
        .env("CF_OVERWRITE", if spec.overwrite { "1" } else { "0" })
        .env(
            "CF_ICON",
            spec.icon.map(|p| p.as_os_str()).unwrap_or_default(),
        )
        .env(
            "CF_FOLDER",
            spec.folder.map(|f| f.as_os_str()).unwrap_or_default(),
        );
    let out = cmd.output().context("could not run PowerShell")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if let Some(existing) = stdout.trim().strip_prefix("EXISTS:") {
        return Err(ShortcutExists(PathBuf::from(existing)).into());
    }
    if !out.status.success() {
        bail!(
            "could not create the shortcut: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(PathBuf::from(stdout.trim()))
}

/// Change the icon of the shortcut [`create`] made for `display_name` in
/// `folder` (default: the Desktop), if it is there. `icon = None` goes back to
/// the launcher's own icon. Returns the shortcut's path, or `None` if there is
/// no such shortcut (nothing is created).
pub fn update_icon(
    display_name: &str,
    folder: Option<&Path>,
    icon: Option<&Path>,
) -> Result<Option<PathBuf>> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$dir = if ($env:CF_FOLDER) { $env:CF_FOLDER } else { [Environment]::GetFolderPath('Desktop') }
$path = Join-Path $dir ($env:CF_NAME + '.lnk')
if (-not (Test-Path -LiteralPath $path)) { [Console]::Out.Write('MISSING'); exit 0 }
$s = (New-Object -ComObject WScript.Shell).CreateShortcut($path)
$s.IconLocation = $(if ($env:CF_ICON) { $env:CF_ICON } else { $s.TargetPath }) + ',0'
$s.Save()
[Console]::Out.Write($path)
"#;
    let out = powershell(SCRIPT)
        .env("CF_NAME", file_safe_name(display_name))
        .env("CF_ICON", icon.map(|p| p.as_os_str()).unwrap_or_default())
        .env(
            "CF_FOLDER",
            folder.map(|f| f.as_os_str()).unwrap_or_default(),
        )
        .output()
        .context("could not run PowerShell")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        bail!(
            "could not change the shortcut's icon: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(match stdout.trim() {
        "MISSING" => None,
        p => Some(PathBuf::from(p)),
    })
}

fn powershell(script: &str) -> Command {
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(file_safe_name("Uni: Dev/Work?"), "Uni_ Dev_Work_");
        assert_eq!(file_safe_name("  ...  "), "ContextFlow");
        assert_eq!(file_safe_name("Personal Projects"), "Personal Projects");
    }

    #[test]
    fn quoting_rejects_quotes() {
        assert_eq!(quote_arg("abc def").unwrap(), "\"abc def\"");
        assert!(quote_arg("a\"b").is_err());
    }
}
