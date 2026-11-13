//! Create a desktop shortcut that restores a workspace with a double-click.
//!
//! The shortcut starts `contextflow-launch.exe` (a windowless program) with the
//! workspace id, so no console window flashes and renaming the workspace later
//! does not break it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context as _, Result};

/// Where the launcher is expected: next to the running `contextflow.exe`.
pub fn launcher_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("could not locate contextflow.exe")?;
    let launcher = exe.with_file_name("contextflow-launch.exe");
    if !launcher.is_file() {
        bail!(
            "{} was not found. Build or install both programs: cargo install --path crates/cf-cli",
            launcher.display()
        );
    }
    Ok(launcher)
}

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
$s.IconLocation = $env:CF_TARGET + ',0'
$s.Description = 'Restore ' + $env:CF_NAME + ' with ContextFlow'
$s.Save()
[Console]::Out.Write($path)
"#;
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("CF_NAME", file_safe_name(spec.display_name))
        .env("CF_TARGET", &launcher)
        .env("CF_ARGS", args.join(" "))
        .env("CF_OVERWRITE", if spec.overwrite { "1" } else { "0" })
        .env(
            "CF_FOLDER",
            spec.folder.map(|f| f.as_os_str()).unwrap_or_default(),
        )
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().context("could not run PowerShell")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if let Some(existing) = stdout.trim().strip_prefix("EXISTS:") {
        bail!("{existing} already exists; use --force to replace it");
    }
    if !out.status.success() {
        bail!(
            "could not create the shortcut: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(PathBuf::from(stdout.trim()))
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
