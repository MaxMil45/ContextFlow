//! Which programs started a process (Windows only).

use std::collections::HashMap;
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Longest chain followed; real launch chains are two or three deep.
const MAX_DEPTH: usize = 8;

struct ProcessInfo {
    path: Option<String>,
    started: Option<u64>,
}

unsafe fn process_info(pid: u32) -> ProcessInfo {
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if handle.is_null() {
        return ProcessInfo {
            path: None,
            started: None,
        };
    }
    let mut buf = vec![0u16; 32_768];
    let mut size = buf.len() as u32;
    let path =
        (QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut size) != 0)
            .then(|| String::from_utf16_lossy(&buf[..size as usize]));
    let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) =
        (zeroed(), zeroed(), zeroed(), zeroed());
    let started = (GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0)
        .then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime));
    CloseHandle(handle);
    ProcessInfo { path, started }
}

/// pid -> parent pid, for every process running now.
unsafe fn parent_map() -> HashMap<u32, u32> {
    let mut map = HashMap::new();
    let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if snap == INVALID_HANDLE_VALUE {
        return map;
    }
    let mut entry: PROCESSENTRY32W = zeroed();
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    if Process32FirstW(snap, &mut entry) != 0 {
        loop {
            map.insert(entry.th32ProcessID, entry.th32ParentProcessID);
            if Process32NextW(snap, &mut entry) == 0 {
                break;
            }
        }
    }
    CloseHandle(snap);
    map
}

/// Executable paths of the processes that started `pid`: its parent first,
/// then the parent's parent, and so on. The chain stops at a process that has
/// exited, can't be inspected, or started *after* its supposed child (its
/// process id was reused by an unrelated program).
pub fn process_ancestors(pid: u32) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let parents = parent_map();
        let mut child = process_info(pid);
        let mut current = pid;
        for _ in 0..MAX_DEPTH {
            let Some(&parent) = parents.get(&current) else {
                break;
            };
            if parent == 0 || parent == current {
                break;
            }
            let info = process_info(parent);
            let (Some(path), Some(started)) = (info.path.clone(), info.started) else {
                break;
            };
            if child.started.is_some_and(|c| started > c) {
                break;
            }
            out.push(path);
            current = parent;
            child = info;
        }
    }
    out
}
