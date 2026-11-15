//! The folders a terminal window's shells are in (Windows only).
//!
//! A terminal window (Windows Terminal) is one process; every tab runs a shell
//! (PowerShell, cmd, ...) somewhere below it. The folder a shell is in is read
//! from that process's own bookkeeping, the same place Process Explorer shows it.
//!
//! This sees where a shell **was started**, and where `cmd` is now. PowerShell
//! does not change the folder of its process when you `cd`, so for it this is
//! the folder it started in.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, IsWow64Process, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_VM_READ,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

/// Programs that are a tab's shell. `wsl` and `bash` are left out on purpose:
/// their process is in a Windows folder, which says nothing about where the
/// Linux shell inside is.
const SHELLS: [&str; 4] = ["pwsh.exe", "powershell.exe", "cmd.exe", "nu.exe"];

/// One process, as the snapshot lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Proc {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
}

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

fn snapshot() -> Vec<Proc> {
    let mut out = Vec::new();
    // SAFETY: a plain ToolHelp snapshot, closed before returning.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut entry) != 0;
        while ok {
            let n = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            out.push(Proc {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
                name: String::from_utf16_lossy(&entry.szExeFile[..n]),
            });
            ok = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
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
    let mut shells = shells_under(&snapshot(), pid);
    shells.sort_by_key(|&p| started_at(p));
    let folders = shells.into_iter().filter_map(process_cwd).collect();
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
        let want = std::fs::canonicalize(&dir).unwrap();
        let want = want
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string();
        assert!(got.eq_ignore_ascii_case(&want), "{got} vs {want}");
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
        let before = terminals(&snapshot());
        crate::launch_process(alias.to_str().unwrap(), &args, None).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(6));
        let table = snapshot();
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
    fn a_stale_window_has_no_shells() {
        assert!(window_shell_folders(0x10).is_none());
    }
}
