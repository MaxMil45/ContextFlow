//! The folders a terminal window's shells are in (Windows only).
//!
//! A terminal window (Windows Terminal) is one process; every tab runs a shell
//! (PowerShell, cmd, ...) somewhere below it. The folder a shell is in is read
//! from that process's own bookkeeping, the same place Process Explorer shows it.
//!
//! This sees where a shell **was started**, and where `cmd` is now. PowerShell
//! does not change the folder of its process when you `cd`, so for it this is
//! the folder it started in, unless the user has installed the opt-in hook
//! (`contextflow terminal hook install`): then the shell writes the folder it
//! is in to `%LOCALAPPDATA%\ContextFlow\shells\<its process id>.txt`, and that
//! is used when it is fresh (see [`reported_folder`]).

use std::ffi::c_void;
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};

use crate::processes::{snapshot, Entry as Proc};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, IsWow64Process, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_VM_READ,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

/// Programs that are a tab's shell. `wsl` and `bash` are left out on purpose:
/// their process is in a Windows folder, which says nothing about where the
/// Linux shell inside is.
const SHELLS: [&str; 4] = ["pwsh.exe", "powershell.exe", "cmd.exe", "nu.exe"];

/// The shells below `root`, whatever depth: a terminal starts its console host
/// first and the shell below that. A shell started *from* a shell (a `cmd`
/// typed into PowerShell) is part of that tab, not another one.
pub(crate) fn shells_under(table: &[Proc], root: u32) -> Vec<u32> {
    let is_shell = |p: &Proc| SHELLS.iter().any(|s| p.name.eq_ignore_ascii_case(s));
    let mut out = Vec::new();
    // (process, whether a shell is already above it)
    let mut work: Vec<(u32, bool)> = vec![(root, false)];
    let mut seen = std::collections::HashSet::new();
    while let Some((pid, under_shell)) = work.pop() {
        if !seen.insert(pid) {
            continue; // process ids are reused: never loop
        }
        for child in table.iter().filter(|p| p.parent == pid && p.pid != pid) {
            let shell = is_shell(child);
            if shell && !under_shell {
                out.push(child.pid);
            }
            work.push((child.pid, under_shell || shell));
        }
    }
    out
}

#[repr(C)]
struct ProcessBasicInformation {
    exit_status: isize,
    peb: *mut c_void,
    affinity_mask: usize,
    base_priority: isize,
    pid: usize,
    parent_pid: usize,
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationProcess(
        process: HANDLE,
        class: u32,
        info: *mut c_void,
        len: u32,
        returned: *mut u32,
    ) -> i32;
}

/// Where in the process's memory the current folder is kept (64-bit layout):
/// the process block -> its parameters -> the current directory text.
const PEB_PARAMETERS: usize = 0x20;
const PARAMETERS_CURRENT_DIR: usize = 0x38;

unsafe fn read(process: HANDLE, from: usize, into: *mut u8, len: usize) -> bool {
    let mut got = 0usize;
    ReadProcessMemory(process, from as *const c_void, into.cast(), len, &mut got) != 0 && got == len
}

/// The current folder of process `pid`, or `None` when it can't be read
/// (elevated, protected, gone, or a 32-bit program).
pub(crate) fn process_cwd(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed on every path; every read is bounds-checked
    // by length and validated afterwards.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid);
        if process.is_null() {
            return None;
        }
        let result = (|| {
            let mut wow = 0;
            if IsWow64Process(process, &mut wow) == 0 || wow != 0 {
                return None; // a different layout: not read
            }
            let mut info: ProcessBasicInformation = zeroed();
            let mut returned = 0u32;
            let status = NtQueryInformationProcess(
                process,
                0, // ProcessBasicInformation
                (&mut info as *mut ProcessBasicInformation).cast(),
                size_of::<ProcessBasicInformation>() as u32,
                &mut returned,
            );
            if status < 0 || info.peb.is_null() {
                return None;
            }
            let mut params = 0usize;
            if !read(
                process,
                info.peb as usize + PEB_PARAMETERS,
                (&mut params as *mut usize).cast(),
                size_of::<usize>(),
            ) || params == 0
            {
                return None;
            }
            // UNICODE_STRING: length in bytes (u16), capacity (u16), padding, buffer.
            let mut text = [0u8; 16];
            if !read(
                process,
                params + PARAMETERS_CURRENT_DIR,
                text.as_mut_ptr(),
                16,
            ) {
                return None;
            }
            let length = u16::from_le_bytes([text[0], text[1]]) as usize;
            let buffer = usize::from_le_bytes(text[8..16].try_into().ok()?);
            if length == 0 || length > 32_768 || length % 2 != 0 || buffer == 0 {
                return None;
            }
            let mut wide = vec![0u16; length / 2];
            if !read(process, buffer, wide.as_mut_ptr().cast(), length) {
                return None;
            }
            let path = String::from_utf16(&wide).ok()?;
            // `C:\Users\me\` -> `C:\Users\me`, but never `C:\` -> `C:`.
            let trimmed = path.trim_end_matches('\\');
            Some(if trimmed.len() <= 2 {
                path
            } else {
                trimmed.to_string()
            })
        })();
        CloseHandle(process);
        result
    }
}

fn started_at(pid: u32) -> u64 {
    // SAFETY: handle closed before returning.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return u64::MAX;
        }
        let (mut created, mut a, mut b, mut c): (FILETIME, FILETIME, FILETIME, FILETIME) =
            (zeroed(), zeroed(), zeroed(), zeroed());
        let ok = GetProcessTimes(process, &mut created, &mut a, &mut b, &mut c) != 0;
        CloseHandle(process);
        if ok {
            (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime)
        } else {
            u64::MAX
        }
    }
}

/// Where PowerShell's hook writes the folder each shell is in.
pub(crate) fn reports_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| {
        std::path::PathBuf::from(d)
            .join("ContextFlow")
            .join("shells")
    })
}

/// A time as the 100 ns ticks since 1601 that Windows uses for files and
/// processes, so the two can be compared.
fn filetime(t: std::time::SystemTime) -> u64 {
    const UNIX_TO_1601_SECS: u64 = 11_644_473_600;
    let d = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() + UNIX_TO_1601_SECS) * 10_000_000 + u64::from(d.subsec_nanos()) / 100
}

/// The folder a shell reported in its file, if the report is believable: text
/// in the file (first line, at most 1,024 characters), written after the
/// process started. A file from an older process that had the same id (ids are
/// reused) was written before this one started, so it is refused.
pub(crate) fn reported_folder(text: &str, written: u64, process_started: u64) -> Option<String> {
    if written < process_started {
        return None;
    }
    let line = text.lines().next()?.trim();
    (!line.is_empty() && line.len() <= 1024).then(|| line.to_string())
}

/// What shell `pid` reported, from `dir`, if it reported anything believable.
fn reported_by(dir: &std::path::Path, pid: u32, process_started: u64) -> Option<String> {
    let path = dir.join(format!("{pid}.txt"));
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() > 4096 {
        return None;
    }
    let text = String::from_utf8_lossy(&std::fs::read(&path).ok()?).into_owned();
    reported_folder(&text, filetime(meta.modified().ok()?), process_started)
}

/// Delete reports of shells that are gone. Cheap; capture does it first.
pub(crate) fn clean_reports() {
    let (Some(dir), Ok(running)) = (reports_dir(), snapshot()) else {
        return;
    };
    let running: std::collections::HashSet<u32> = running.into_iter().map(|e| e.pid).collect();
    let Ok(files) = std::fs::read_dir(dir) else {
        return;
    };
    for f in files.flatten() {
        let name = f.file_name().to_string_lossy().into_owned();
        let pid = name
            .strip_suffix(".txt")
            .and_then(|n| n.parse::<u32>().ok());
        if pid.is_some_and(|p| !running.contains(&p)) {
            let _ = std::fs::remove_file(f.path());
        }
    }
}

/// The program that owns window `hwnd` and the folder of each shell running in
/// it, oldest shell (first tab) first. Shells whose folder can't be read are
/// left out.
pub(crate) fn window_shell_folders(hwnd: isize) -> Option<(u32, Vec<String>)> {
    let mut pid = 0u32;
    // SAFETY: reads the owner of a window handle; a stale handle gives 0.
    unsafe { GetWindowThreadProcessId(hwnd as _, &mut pid) };
    if pid == 0 {
        return None;
    }
    let mut shells = shells_under(&snapshot().unwrap_or_default(), pid);
    shells.sort_by_key(|&p| started_at(p));
    let reports = reports_dir();
    let folders = shells
        .into_iter()
        .filter_map(|p| {
            // What the shell says about itself, when it does; otherwise where
            // its process is.
            reports
                .as_deref()
                .and_then(|dir| reported_by(dir, p, started_at(p)))
                .or_else(|| process_cwd(p))
        })
        .collect();
    Some((pid, folders))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, name: &str) -> Proc {
        Proc {
            pid,
            parent,
            name: name.into(),
        }
    }

    #[test]
    fn shells_are_found_below_the_console_host_and_nested_ones_are_not_extra_tabs() {
        let table = [
            p(1, 0, "WindowsTerminal.exe"),
            p(2, 1, "OpenConsole.exe"),
            p(3, 2, "pwsh.exe"),
            p(4, 3, "cmd.exe"), // typed into pwsh: same tab
            p(5, 4, "node.exe"),
            p(6, 1, "OpenConsole.exe"),
            p(7, 6, "CMD.EXE"),
            p(8, 6, "wsl.exe"),  // not a shell we can read
            p(9, 0, "pwsh.exe"), // another terminal's
        ];
        let mut got = shells_under(&table, 1);
        got.sort();
        assert_eq!(got, [3, 7]);
        assert!(shells_under(&table, 99).is_empty());
    }

    #[test]
    fn a_process_table_that_loops_does_not_hang() {
        let table = [p(1, 2, "a.exe"), p(2, 1, "pwsh.exe")];
        assert_eq!(shells_under(&table, 1), [2]);
    }

    #[test]
    fn the_folder_a_process_runs_in_is_read() {
        let dir = std::env::temp_dir().join("cf-shells-test");
        std::fs::create_dir_all(&dir).unwrap();
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/c", "ping -n 6 127.0.0.1 >nul"])
            .current_dir(&dir)
            .spawn()
            .unwrap();
        let mut got = None;
        for _ in 0..20 {
            got = process_cwd(child.id());
            if got.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        let got = got.expect("the folder of a child of ours can be read");
        // The temp folder may be spelled in its short form (`RUNNER~1`) on one
        // side and its long form on the other: compare the real folders.
        let real = |p: &std::path::Path| std::fs::canonicalize(p).unwrap();
        assert_eq!(real(std::path::Path::new(&got)), real(&dir), "{got}");
        let _ = std::fs::remove_dir(&dir);
    }

    /// What a shell's process says after the user has changed folder. Run with
    /// `cargo test -p cf-windows real_shells -- --ignored --nocapture`.
    #[test]
    #[ignore = "starts real shells for a few seconds"]
    fn real_shells_after_a_cd() {
        let start = std::env::temp_dir();
        for (exe, args) in [
            (
                "powershell.exe",
                vec![
                    "-NoProfile",
                    "-Command",
                    "Set-Location $env:windir; Start-Sleep 4",
                ],
            ),
            (
                "cmd.exe",
                vec!["/c", "cd /d %windir% && ping -n 5 127.0.0.1 >nul"],
            ),
        ] {
            let mut child = std::process::Command::new(exe)
                .args(&args)
                .current_dir(&start)
                .spawn()
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1500));
            println!(
                "{exe}: started in {start:?}, now says {:?}",
                process_cwd(child.id())
            );
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Opens (and closes again) a real Windows Terminal window through its
    /// alias, the way a restore does. Run with
    /// `cargo test -p cf-windows real_terminal -- --ignored --nocapture`.
    #[test]
    #[ignore = "opens a real Windows Terminal window for a few seconds"]
    fn real_terminal_is_started_by_its_alias_with_a_folder_per_tab() {
        let alias = std::path::Path::new(&std::env::var_os("LOCALAPPDATA").unwrap())
            .join(r"Microsoft\WindowsApps\wt.exe");
        let args: Vec<String> = [
            "-w",
            "new",
            "-d",
            r"C:\Windows",
            ";",
            "new-tab",
            "-d",
            r"C:\Users",
        ]
        .map(String::from)
        .into();
        let terminals = |table: &[Proc]| -> Vec<u32> {
            table
                .iter()
                .filter(|p| p.name.eq_ignore_ascii_case("WindowsTerminal.exe"))
                .map(|p| p.pid)
                .collect()
        };
        let before = terminals(&snapshot().unwrap_or_default());
        crate::launch_process(alias.to_str().unwrap(), &args, None).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(6));
        let table = snapshot().unwrap_or_default();
        let new: Vec<u32> = terminals(&table)
            .into_iter()
            .filter(|p| !before.contains(p))
            .collect();
        for &pid in &new {
            let folders: Vec<_> = shells_under(&table, pid)
                .into_iter()
                .filter_map(process_cwd)
                .collect();
            println!("terminal {pid}: {folders:?}");
        }
        for pid in new {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .output();
        }
    }

    #[test]
    fn a_report_is_believed_only_when_written_after_the_shell_started() {
        let started = 1_000;
        assert_eq!(
            reported_folder("C:\\work\\api\r\nignored", 2_000, started).as_deref(),
            Some("C:\\work\\api")
        );
        // Written before this process existed: from an older shell with the
        // same id.
        assert_eq!(reported_folder("C:\\old", 999, started), None);
        // A shell whose start time couldn't be read is never believed.
        assert_eq!(reported_folder("C:\\x", 2_000, u64::MAX), None);
        assert_eq!(reported_folder("", 2_000, started), None);
        assert_eq!(reported_folder("\r\n", 2_000, started), None);
        assert_eq!(reported_folder(&"x".repeat(2_000), 2_000, started), None);
    }

    #[test]
    fn reports_are_read_from_files_named_by_process_id_and_old_ones_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("42.txt"), "C:\\work\\api").unwrap();
        std::fs::write(dir.path().join("43.txt"), vec![b'x'; 5_000]).unwrap();
        assert_eq!(
            reported_by(dir.path(), 42, 0).as_deref(),
            Some("C:\\work\\api")
        );
        // The file was written just now: a process "started" in the far future is newer.
        assert_eq!(
            reported_by(
                dir.path(),
                42,
                filetime(std::time::SystemTime::now()) + 10_000_000_000
            ),
            None
        );
        assert_eq!(reported_by(dir.path(), 43, 0), None, "too large");
        assert_eq!(reported_by(dir.path(), 44, 0), None, "no file");
    }

    #[test]
    fn a_stale_window_has_no_shells() {
        assert!(window_shell_folders(0x10).is_none());
    }
}
