//! The opt-in PowerShell hook that lets ContextFlow see where each PowerShell
//! tab is.
//!
//! PowerShell doesn't tell Windows when you `cd`, so from outside a tab looks
//! as if it is still in the folder it started in. The hook is a small block in
//! the user's PowerShell profile that wraps the prompt: after the existing
//! prompt has run (so tools that read the result of the last command see it
//! untouched), it writes the current folder to
//! `%LOCALAPPDATA%\ContextFlow\shells\<PID>.txt` when the folder changed.
//! Capture reads that file (see `cf_windows`'s shells module).
//!
//! Everything here is text in, text out: nothing touches a profile unless
//! [`install_into`] and friends are told which file to use.

use std::path::{Path, PathBuf};

/// The first line of the block. Everything from here to [`END`] is ours.
pub const BEGIN: &str = "# >>> ContextFlow terminal hook >>>";
/// The last line of the block.
pub const END: &str = "# <<< ContextFlow terminal hook <<<";

/// The block added to a profile.
pub fn block() -> String {
    format!(
        r#"{BEGIN}
# Added by `contextflow terminal hook install`; remove with
# `contextflow terminal hook uninstall`. Tells ContextFlow which folder this
# tab is in, so a saved workspace reopens it there. It wraps your prompt and
# never changes what the prompt shows.
if (-not $global:__cf_prompt_wrapped) {{
  $global:__cf_prompt_wrapped = $true
  $global:__cf_old_prompt = $function:prompt
  $global:__cf_last_folder = $null
  function global:prompt {{
    $__cf_text = if ($global:__cf_old_prompt) {{ & $global:__cf_old_prompt }} else {{ "PS $($executionContext.SessionState.Path.CurrentLocation)> " }}
    try {{
      if ($executionContext.SessionState.Path.CurrentLocation.Provider.Name -eq 'FileSystem') {{
        $__cf_folder = $executionContext.SessionState.Path.CurrentLocation.ProviderPath
        if ($__cf_folder -ne $global:__cf_last_folder) {{
          $__cf_dir = Join-Path $env:LOCALAPPDATA 'ContextFlow\shells'
          [void][System.IO.Directory]::CreateDirectory($__cf_dir)
          [System.IO.File]::WriteAllText((Join-Path $__cf_dir "$PID.txt"), $__cf_folder)
          $global:__cf_last_folder = $__cf_folder
        }}
      }}
    }} catch {{ }}
    $__cf_text
  }}
}}
{END}
"#
    )
}

/// Whether `profile` already has the block.
pub fn has_block(profile: &str) -> bool {
    profile.lines().any(|l| l.trim() == BEGIN)
}

/// `profile` with the block added at the end (nothing changes if it is
/// already there). Returns the new text and whether it changed.
pub fn add_block(profile: &str) -> (String, bool) {
    if has_block(profile) {
        return (profile.to_string(), false);
    }
    let mut text = profile.to_string();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push_str("\r\n");
    }
    if !text.is_empty() {
        text.push_str("\r\n");
    }
    // Profiles on Windows use CRLF.
    text.push_str(&block().replace('\n', "\r\n"));
    (text, true)
}

/// `profile` without the block, and with the blank line `add_block` put in
/// front of it. Everything else is kept exactly. Returns the new text and
/// whether it changed. A block with no end line is left alone: it can't be
/// said where it stops.
pub fn remove_block(profile: &str) -> (String, bool) {
    let lines: Vec<&str> = profile.split_inclusive('\n').collect();
    let Some(start) = lines.iter().position(|l| l.trim() == BEGIN) else {
        return (profile.to_string(), false);
    };
    let Some(end) = lines[start..]
        .iter()
        .position(|l| l.trim() == END)
        .map(|i| start + i)
    else {
        return (profile.to_string(), false);
    };
    let mut keep: Vec<&str> = lines[..start].to_vec();
    // The blank line added in front of the block.
    if keep.last().is_some_and(|l| l.trim().is_empty()) {
        keep.pop();
    }
    keep.extend(&lines[end + 1..]);
    (keep.concat(), true)
}

/// What happened to one profile file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Added,
    AlreadyThere,
    Removed,
    NotThere,
}

fn read(path: &Path) -> std::io::Result<String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// Add the hook to the profile file at `path` (created if missing).
pub fn install_into(path: &Path) -> std::io::Result<Outcome> {
    let (text, changed) = add_block(&read(path)?);
    if !changed {
        return Ok(Outcome::AlreadyThere);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)?;
    Ok(Outcome::Added)
}

/// Take the hook out of the profile file at `path`. A profile that is empty
/// afterwards, because the hook was all it had, is deleted.
pub fn uninstall_from(path: &Path) -> std::io::Result<Outcome> {
    if !path.exists() {
        return Ok(Outcome::NotThere);
    }
    let (text, changed) = remove_block(&read(path)?);
    if !changed {
        return Ok(Outcome::NotThere);
    }
    if text.trim().is_empty() {
        std::fs::remove_file(path)?;
    } else {
        std::fs::write(path, text)?;
    }
    Ok(Outcome::Removed)
}

/// Whether the profile file at `path` has the hook.
pub fn is_installed(path: &Path) -> bool {
    read(path).is_ok_and(|t| has_block(&t))
}

/// The all-hosts profile files of the PowerShells installed here (Windows
/// PowerShell and PowerShell 7), found by asking each one, and only those that
/// answer. Starts each for a moment.
pub fn profile_paths() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for shell in ["powershell.exe", "pwsh.exe"] {
        let mut cmd = std::process::Command::new(shell);
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$PROFILE.CurrentUserAllHosts",
        ])
        .stdin(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let Ok(out) = cmd.output() else { continue };
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && Path::new(&path).is_absolute() {
            let path = PathBuf::from(path);
            if !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = "Import-Module posh-git\r\nSet-Alias ll Get-ChildItem\r\n\r\nfunction prompt { \"my> \" }\r\n";

    #[test]
    fn the_block_is_added_once_and_the_rest_of_the_profile_is_kept() {
        let (once, changed) = add_block(PROFILE);
        assert!(changed && once.starts_with(PROFILE.trim_end_matches("\r\n")));
        assert!(has_block(&once));
        let (twice, changed_again) = add_block(&once);
        assert!(!changed_again);
        assert_eq!(twice, once);
        assert_eq!(once.matches(BEGIN).count(), 1);
    }

    #[test]
    fn removing_gives_back_exactly_what_was_there() {
        for original in [PROFILE, "", "no newline at end", "one\r\n"] {
            let (added, _) = add_block(original);
            let (back, changed) = remove_block(&added);
            assert!(changed, "{original:?}");
            assert_eq!(back.trim_end(), original.trim_end(), "{original:?}");
            assert!(!has_block(&back));
        }
        // Text after the block is kept too.
        let (added, _) = add_block(PROFILE);
        let with_tail = format!("{added}Set-Alias gs git-status\r\n");
        let (back, _) = remove_block(&with_tail);
        assert!(back.ends_with("Set-Alias gs git-status\r\n"));
        assert!(!back.contains("ContextFlow"));
    }

    #[test]
    fn removing_what_is_not_there_or_is_unfinished_changes_nothing() {
        assert_eq!(remove_block(PROFILE), (PROFILE.to_string(), false));
        let broken = format!("{PROFILE}{BEGIN}\r\nsomething\r\n");
        assert_eq!(remove_block(&broken), (broken.clone(), false));
    }

    #[test]
    fn the_hook_runs_the_existing_prompt_first_and_returns_its_text() {
        let b = block();
        let old = b.find("& $global:__cf_old_prompt").unwrap();
        let write = b.find("WriteAllText").unwrap();
        assert!(old < write, "the old prompt runs before anything of ours");
        // The prompt's own text is what is returned, last.
        assert!(b.contains("\n    $__cf_text\n  }"));
        assert!(b.contains("-not $global:__cf_prompt_wrapped"));
    }

    #[test]
    fn install_and_uninstall_work_on_a_file_and_delete_a_profile_that_was_only_ours() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("sub").join("profile.ps1");
        // A missing file (and folder) is created.
        assert_eq!(install_into(&existing).unwrap(), Outcome::Added);
        assert!(is_installed(&existing));
        assert_eq!(install_into(&existing).unwrap(), Outcome::AlreadyThere);
        assert_eq!(uninstall_from(&existing).unwrap(), Outcome::Removed);
        assert!(
            !existing.exists(),
            "a profile that was only the hook is removed"
        );
        assert_eq!(uninstall_from(&existing).unwrap(), Outcome::NotThere);

        // A profile with other content keeps it.
        let mine = dir.path().join("mine.ps1");
        std::fs::write(&mine, PROFILE).unwrap();
        assert_eq!(install_into(&mine).unwrap(), Outcome::Added);
        assert_eq!(uninstall_from(&mine).unwrap(), Outcome::Removed);
        assert_eq!(
            std::fs::read_to_string(&mine).unwrap().trim_end(),
            PROFILE.trim_end()
        );
    }

    /// Runs the block in a real Windows PowerShell, with a custom prompt in
    /// place, and checks that the prompt still shows what it showed and that
    /// the folder is written for this shell's id. Nothing is installed: the
    /// block is dot-sourced from a temporary file, and the one file it writes
    /// (named by that shell's id) is removed afterwards.
    #[test]
    fn the_block_works_in_a_real_powershell_and_keeps_a_custom_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile.ps1");
        let target = dir.path().join("target folder");
        std::fs::create_dir_all(&target).unwrap();
        let (text, _) = add_block("function prompt { 'custom> ' }\r\n");
        std::fs::write(&profile, text).unwrap();
        let script = [
            format!(". '{}'", profile.display()),
            "$first = prompt".to_string(),
            format!("Set-Location -LiteralPath '{}'", target.display()),
            "$second = prompt".to_string(),
            r#"$file = Join-Path $env:LOCALAPPDATA "ContextFlow\shells\$PID.txt""#.to_string(),
            "$written = Get-Content -Raw -LiteralPath $file".to_string(),
            "Remove-Item -LiteralPath $file".to_string(),
            r#""$first|$second|$written""#.to_string(),
        ]
        .join("; ");
        let Ok(out) = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
        else {
            return; // no Windows PowerShell here
        };
        let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert!(
            out.status.success(),
            "{said} {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let parts: Vec<&str> = said.split('|').collect();
        assert_eq!(&parts[..2], ["custom> ", "custom> "], "{said}");
        // The folder is written as the shell reports it; compare the real folders.
        let real = |p: &str| std::fs::canonicalize(p.trim()).unwrap();
        assert_eq!(real(parts[2]), real(&target.to_string_lossy()), "{said}");
    }
}
