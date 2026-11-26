//! Windows platform integration.
//!
//! Currently: read-only discovery of top-level windows and monitors.
//! Phase 2 adds launching, window identification and placement.
//!
//! Window handles (`hwnd`) are exposed for the current session only and must
//! never be persisted: Windows reuses them.

use cf_core::{DisplayState, MonitorInfo, Rect};
use serde::Serialize;
use thiserror::Error;

#[cfg(windows)]
mod ancestry;
#[cfg(windows)]
mod control;
#[cfg(windows)]
mod imp;
#[cfg(windows)]
mod lock;
#[cfg(windows)]
mod processes;
#[cfg(windows)]
mod shells;
#[cfg(windows)]
mod text;

#[cfg(windows)]
pub use lock::InstanceLock;

/// The program that owns window `hwnd`, and the folder each shell running in it
/// is in, first tab first (see the `shells` module for what that means for
/// PowerShell). `None` when the window is gone or this isn't Windows.
pub fn window_shell_folders(hwnd: isize) -> Option<(u32, Vec<String>)> {
    #[cfg(windows)]
    {
        shells::window_shell_folders(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        None
    }
}

/// Delete the folder reports of PowerShell shells that have ended (see the
/// `shells` module). Does nothing off Windows.
pub fn clean_shell_reports() {
    #[cfg(windows)]
    shells::clean_reports();
}

/// Placeholder so callers compile off Windows; it never excludes anything.
#[cfg(not(windows))]
pub struct InstanceLock;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("this operation is only supported on Windows")]
    Unsupported,
    #[error("operating system error: {0}")]
    Os(#[from] std::io::Error),
}

pub type Result<T, E = PlatformError> = std::result::Result<T, E>;

/// A visible, user-facing top-level window found on the desktop.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiscoveredWindow {
    /// Session-only handle. Do not store.
    pub hwnd: isize,
    pub process_id: u32,
    pub executable_path: Option<String>,
    pub title: String,
    pub class_name: String,
    pub rect: Rect,
    pub display_state: DisplayState,
    pub monitor_id: Option<String>,
}

/// Opt this process into per-monitor-v2 DPI awareness so that all coordinates
/// are physical pixels. Call once, before any window or monitor query.
pub fn init_dpi_awareness() {
    #[cfg(windows)]
    imp::init_dpi_awareness();
}

pub fn enumerate_windows() -> Result<Vec<DiscoveredWindow>> {
    #[cfg(windows)]
    {
        imp::enumerate_windows()
    }
    #[cfg(not(windows))]
    {
        Err(PlatformError::Unsupported)
    }
}

pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
    #[cfg(windows)]
    {
        imp::enumerate_monitors()
    }
    #[cfg(not(windows))]
    {
        Err(PlatformError::Unsupported)
    }
}

/// A running process, as listed by [`list_processes`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: u32,
    /// Executable file name, e.g. `Discord.exe`.
    pub name: String,
    /// Full path; `None` when the process can't be inspected (elevated, protected).
    pub path: Option<String>,
    pub session_id: Option<u32>,
    /// Working set (memory currently in RAM), in bytes.
    pub ram_bytes: u64,
    /// Total CPU time used so far (kernel + user), in 100 ns units. Two
    /// readings a little apart give the CPU use in between.
    pub cpu_time_100ns: u64,
}

/// All running processes.
pub fn list_processes() -> Result<Vec<ProcessInfo>> {
    #[cfg(windows)]
    {
        processes::list_processes().map_err(Into::into)
    }
    #[cfg(not(windows))]
    {
        Err(PlatformError::Unsupported)
    }
}

/// The ids of every running process: cheap, for "is it still running?".
/// Empty off Windows.
pub fn running_pids() -> std::collections::HashSet<u32> {
    #[cfg(windows)]
    {
        processes::running_pids()
    }
    #[cfg(not(windows))]
    {
        std::collections::HashSet::new()
    }
}

/// The login session this program runs in.
pub fn current_session() -> Option<u32> {
    #[cfg(windows)]
    {
        processes::current_session()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Ask a process to close, like its window's close button. Returns how many
/// windows were asked (0: it has no window to ask).
pub fn request_close(pid: u32) -> usize {
    #[cfg(windows)]
    {
        processes::request_close(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        0
    }
}

/// Whether the process is showing a dialog box, such as "Save changes?".
/// Always `false` off Windows.
pub fn has_open_prompt(pid: u32) -> bool {
    #[cfg(windows)]
    {
        processes::has_open_prompt(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

/// End a process immediately; its unsaved work is lost.
pub fn terminate_process(pid: u32) -> Result<()> {
    #[cfg(windows)]
    {
        processes::terminate_process(pid).map_err(Into::into)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        Err(PlatformError::Unsupported)
    }
}

/// Executable paths of the programs that started process `pid`, parent
/// first. Empty when unknown (or not on Windows).
pub fn process_ancestors(pid: u32) -> Vec<String> {
    #[cfg(windows)]
    {
        ancestry::process_ancestors(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        Vec::new()
    }
}

/// Place a window (see `control::place_window`). Fails with an OS error 5
/// ("access denied") for windows of elevated processes.
pub fn place_window(hwnd: isize, rect: Rect, state: DisplayState) -> Result<()> {
    #[cfg(windows)]
    {
        control::place_window(hwnd, rect, state).map_err(Into::into)
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, rect, state);
        Err(PlatformError::Unsupported)
    }
}

/// Put a window above the others without activating it (see
/// `control::raise_window`).
pub fn raise_window(hwnd: isize) -> Result<()> {
    #[cfg(windows)]
    {
        control::raise_window(hwnd).map_err(Into::into)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        Err(PlatformError::Unsupported)
    }
}

/// Give a window the keyboard focus; `false` if Windows would not allow it
/// (see `control::focus_window`).
pub fn focus_window(hwnd: isize) -> bool {
    #[cfg(windows)]
    {
        control::focus_window(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        false
    }
}

/// Take a machine-wide (per login session) lock called `name`, e.g.
/// `Local\ContextFlow.Restore`. `Ok(None)` means another process holds it. The
/// lock is released when the returned guard is dropped or the process exits.
pub fn try_lock_instance(name: &str) -> Result<Option<InstanceLock>> {
    #[cfg(windows)]
    {
        lock::try_lock(name).map_err(Into::into)
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        Ok(Some(InstanceLock))
    }
}

/// For an executable inside a Microsoft Store package
/// (`...\WindowsApps\<Name>_<ver>_<arch>__<publisher>\...`), return its
/// Application User Model ID (`<Name>_<publisher>!<AppId>`), read from the
/// package's `AppxManifest.xml`. `None` for anything else.
pub fn packaged_app_id(exe_path: &str) -> Option<String> {
    let manifest_for =
        |root: &str| std::fs::read_to_string(format!("{root}\\AppxManifest.xml")).ok();
    packaged_app_id_with(exe_path, &manifest_for)
}

/// [`packaged_app_id`] with the manifest reader injected, for testing.
pub fn packaged_app_id_with(
    exe_path: &str,
    read_manifest: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let norm = exe_path.replace('/', "\\");
    let lower = norm.to_lowercase();
    let marker = "\\windowsapps\\";
    let start = lower.find(marker)? + marker.len();
    let rest = &norm[start..];
    let (folder, relative) = rest.split_once('\\')?;
    let root = &norm[..start + folder.len()];

    // Folder: Name_Version_Arch_ResourceId_PublisherId; family = Name_PublisherId.
    let parts: Vec<&str> = folder.split('_').collect();
    if parts.len() < 5 {
        return None;
    }
    let family = format!("{}_{}", parts[0], parts[parts.len() - 1]);

    let manifest = read_manifest(root)?;
    let app_id = pick_manifest_application(&manifest, relative)?;
    Some(format!("{family}!{app_id}"))
}

/// Choose the `<Application Id=...>` whose `Executable` is `relative`. Some
/// packages list several entries for one executable (Teams has a "remote
/// module container" next to the real app), so prefer full-trust entries.
fn pick_manifest_application(manifest: &str, relative: &str) -> Option<String> {
    fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
        let key = format!("{name}=\"");
        let i = tag.find(&key)? + key.len();
        Some(&tag[i..i + tag[i..].find('"')?])
    }
    let wanted = relative.replace('/', "\\").to_lowercase();
    let mut candidates: Vec<(bool, String)> = Vec::new();
    let mut from = 0;
    while let Some(i) = manifest[from..].find("<Application ") {
        let begin = from + i;
        let end = begin + manifest[begin..].find('>')?;
        let tag = &manifest[begin..end];
        from = end;
        let (Some(id), Some(exe)) = (attr(tag, "Id"), attr(tag, "Executable")) else {
            continue;
        };
        if exe.replace('/', "\\").to_lowercase() == wanted {
            let full_trust = attr(tag, "EntryPoint") == Some("Windows.FullTrustApplication");
            candidates.push((full_trust, id.to_string()));
        }
    }
    candidates
        .iter()
        .find(|(full, _)| *full)
        .or_else(|| candidates.first())
        .map(|(_, id)| id.clone())
}

/// Start a Microsoft Store app by its Application User Model ID. If it is
/// already running, Windows brings its window forward.
pub fn launch_packaged(app_user_model_id: &str) -> std::io::Result<()> {
    let valid = !app_user_model_id.is_empty()
        && app_user_model_id.contains('!')
        && app_user_model_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-!".contains(c));
    if !valid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a valid app package id",
        ));
    }
    use std::process::{Command, Stdio};
    // explorer.exe resolves the id and activates the app, then exits.
    Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{app_user_model_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Show a modal message box. No-op off Windows.
pub fn message_box(title: &str, text: &str, warning: bool) {
    #[cfg(windows)]
    control::message_box(title, text, warning);
    #[cfg(not(windows))]
    let _ = (title, text, warning);
}

/// Start `exe` as an independent process (no shell, no inherited console) and
/// return its process id. Console programs get their own window.
pub fn launch_process(
    exe: &str,
    args: &[String],
    working_dir: Option<&str>,
) -> std::io::Result<u32> {
    use std::process::{Command, Stdio};
    if exe.contains('\0') || args.iter().any(|a| a.contains('\0')) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "launch parameters must not contain NUL characters",
        ));
    }
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = working_dir.filter(|d| std::path::Path::new(d).is_dir()) {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        cmd.creation_flags(CREATE_NEW_CONSOLE);
    }
    // Dropping the Child handle detaches it; the app keeps running after we exit.
    Ok(cmd.spawn()?.id())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Only stale handles are used, so no real window is touched.
    #[test]
    fn raising_and_focusing_a_window_that_does_not_exist_fails_cleanly() {
        assert!(raise_window(0).is_err());
        assert!(raise_window(0x7FFF_FFF0).is_err());
        assert!(!focus_window(0));
        assert!(!focus_window(0x7FFF_FFF0));
    }

    #[test]
    fn monitors_are_discoverable() {
        init_dpi_awareness();
        let monitors = enumerate_monitors().unwrap();
        // CI runners and interactive desktops both expose at least one display.
        assert!(!monitors.is_empty());
        assert!(monitors.iter().filter(|m| m.primary).count() <= 1);
        for m in &monitors {
            assert!(m.rect.width > 0 && m.rect.height > 0);
        }
    }

    #[test]
    fn instance_lock_is_exclusive_until_dropped() {
        let name = format!(r"Local\ContextFlow.Test.{}", std::process::id());
        let first = try_lock_instance(&name).unwrap();
        assert!(first.is_some(), "first caller gets the lock");
        assert!(
            try_lock_instance(&name).unwrap().is_none(),
            "second caller is refused while it is held"
        );
        drop(first);
        assert!(
            try_lock_instance(&name).unwrap().is_some(),
            "free again after the holder drops it"
        );
    }

    /// What listing processes costs here. Run with
    /// `cargo test -p cf-windows --release process_listing_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn process_listing_cost() {
        let time = |f: &dyn Fn() -> usize| {
            let start = std::time::Instant::now();
            let n = (0..5).map(|_| f()).last().unwrap_or(0);
            (n, start.elapsed() / 5)
        };
        let (n, full) = time(&|| list_processes().unwrap().len());
        let (m, ids) = time(&|| running_pids().len());
        println!("{n} processes: full listing {full:?}, ids only {ids:?} ({m})");
    }

    /// What scanning the windows costs here. Run with
    /// `cargo test -p cf-windows --release window_scan_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn window_scan_cost() {
        init_dpi_awareness();
        let start = std::time::Instant::now();
        let mut windows = Vec::new();
        for _ in 0..10 {
            windows = enumerate_windows().unwrap();
        }
        let each = start.elapsed() / 10;
        let pids: std::collections::HashSet<u32> = windows.iter().map(|w| w.process_id).collect();
        println!(
            "{} windows from {} programs: {each:?} per scan",
            windows.len(),
            pids.len()
        );
    }

    #[test]
    fn window_enumeration_does_not_fail() {
        init_dpi_awareness();
        // May legitimately be empty on a headless runner.
        let windows = enumerate_windows().unwrap();
        for w in &windows {
            assert!(!w.title.is_empty());
        }
    }
}

#[cfg(test)]
mod packaged_tests {
    use super::*;

    const TEAMS: &str = r#"<Package><Applications>
      <Application Id="MSTeamsRemoteModuleContainer" Executable="ms-teams.exe" EntryPoint="Windows.PartialTrustApplication" uap10:RuntimeBehavior="packagedClassicApp">
      <Application Id="MSTeams" Executable="ms-teams.exe" EntryPoint="Windows.FullTrustApplication">
      <Application Id="MSTeams.Update" Executable="ms-teamsupdate.exe" EntryPoint="Windows.FullTrustApplication">
    </Applications></Package>"#;

    #[test]
    fn picks_the_full_trust_entry_for_a_shared_executable() {
        let id = packaged_app_id_with(
            r"C:\Program Files\WindowsApps\MSTeams_26304.1201.5310.7985_x64__8wekyb3d8bbwe\ms-teams.exe",
            &|root| {
                assert!(root.ends_with("8wekyb3d8bbwe"));
                Some(TEAMS.into())
            },
        );
        assert_eq!(id.as_deref(), Some("MSTeams_8wekyb3d8bbwe!MSTeams"));
    }

    #[test]
    fn handles_subfolder_executables_and_other_paths() {
        let manifest = r#"<Application Id="Claude" Executable="app\Claude.exe" EntryPoint="Windows.FullTrustApplication">
                          <Application Id="SshProxy" Executable="app\resources\claude-ssh-proxy.exe">"#;
        let id = packaged_app_id_with(
            r"C:\Program Files\WindowsApps\Claude_5.11492.0.0_x64__pzs8sxrjxfjjc\app\claude.exe",
            &|_| Some(manifest.into()),
        );
        assert_eq!(id.as_deref(), Some("Claude_pzs8sxrjxfjjc!Claude"));

        let never = |_: &str| -> Option<String> { panic!("must not read") };
        assert_eq!(
            packaged_app_id_with(r"C:\Windows\notepad.exe", &never),
            None
        );
        // Unreadable manifest or unknown executable -> None, not a guess.
        assert_eq!(
            packaged_app_id_with(r"C:\Program Files\WindowsApps\A_1_x64__pub\x.exe", &|_| {
                None
            }),
            None
        );
        assert_eq!(
            packaged_app_id_with(r"C:\Program Files\WindowsApps\A_1_x64__pub\x.exe", &|_| {
                Some(manifest.into())
            }),
            None
        );
    }

    #[test]
    fn rejects_unsafe_ids() {
        for bad in ["", "NoBang", "A!B C", "A!B&calc", "A!B\"", r"..\x!y"] {
            assert!(launch_packaged(bad).is_err(), "{bad}");
        }
    }
}
