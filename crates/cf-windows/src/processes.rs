//! Running processes: list them, ask one to close, end one (Windows only).

use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{
    CloseHandle, BOOL, FILETIME, HWND, INVALID_HANDLE_VALUE, LPARAM,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
    TerminateProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetLastActivePopup, GetWindow, GetWindowThreadProcessId,
    IsWindowVisible, PostMessageW, GW_OWNER, WM_CLOSE,
};

use crate::text::from_wide;
use crate::ProcessInfo;

/// One process as the system's process list shows it, without opening it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub pid: u32,
    pub parent: u32,
    /// Executable file name, e.g. `pwsh.exe`.
    pub name: String,
}

/// Every running process, from one system snapshot. Opens no process, so it
/// is cheap; [`list_processes`] adds what needs each one opened.
pub(crate) fn snapshot() -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    // SAFETY: a plain ToolHelp snapshot, closed before returning.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut entry) != 0;
        while ok {
            out.push(Entry {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
                name: from_wide(&entry.szExeFile),
            });
            ok = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
    }
    Ok(out)
}

struct Details {
    path: Option<String>,
    ram_bytes: u64,
    cpu_time_100ns: u64,
}

unsafe fn details(pid: u32) -> Details {
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if handle.is_null() {
        // Elevated or protected: nothing can be read, and nothing can be closed.
        return Details {
            path: None,
            ram_bytes: 0,
            cpu_time_100ns: 0,
        };
    }
    let mut buf = vec![0u16; 32_768];
    let mut size = buf.len() as u32;
    let path =
        (QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut size) != 0)
            .then(|| String::from_utf16_lossy(&buf[..size as usize]));

    let mut counters: PROCESS_MEMORY_COUNTERS = zeroed();
    counters.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let ram_bytes = if K32GetProcessMemoryInfo(handle, &mut counters, counters.cb) != 0 {
        counters.WorkingSetSize as u64
    } else {
        0
    };

    let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) =
        (zeroed(), zeroed(), zeroed(), zeroed());
    let cpu_time_100ns =
        if GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0 {
            let t = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
            t(kernel) + t(user)
        } else {
            0
        };
    CloseHandle(handle);
    Details {
        path,
        ram_bytes,
        cpu_time_100ns,
    }
}

pub fn list_processes() -> std::io::Result<Vec<ProcessInfo>> {
    Ok(snapshot()?
        .into_iter()
        .map(|e| {
            // SAFETY: reads a process's details; handles are closed inside.
            let d = unsafe { details(e.pid) };
            let mut session = 0u32;
            // SAFETY: writes only to `session`.
            let session_id =
                (unsafe { ProcessIdToSessionId(e.pid, &mut session) } != 0).then_some(session);
            ProcessInfo {
                pid: e.pid,
                parent_pid: e.parent,
                name: e.name,
                path: d.path,
                session_id,
                ram_bytes: d.ram_bytes,
                cpu_time_100ns: d.cpu_time_100ns,
            }
        })
        .collect())
}

/// The ids of every running process, and nothing else: one system snapshot,
/// without opening any process. Much cheaper than [`list_processes`], which
/// opens each one for its path, memory and CPU time.
pub fn running_pids() -> std::collections::HashSet<u32> {
    snapshot()
        .map(|list| list.into_iter().map(|e| e.pid).collect())
        .unwrap_or_default()
}

/// The session this program runs in; only processes in it are ever touched.
pub fn current_session() -> Option<u32> {
    let mut session = 0u32;
    unsafe { (ProcessIdToSessionId(GetCurrentProcessId(), &mut session) != 0).then_some(session) }
}

struct CloseRequest {
    pid: u32,
    sent: usize,
}

unsafe extern "system" fn post_close(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let req = &mut *(lparam as *mut CloseRequest);
    let mut owner_pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut owner_pid);
    // Visible, un-owned top-level windows: the ones a user would close by hand.
    if owner_pid == req.pid
        && IsWindowVisible(hwnd) != 0
        && GetWindow(hwnd, GW_OWNER).is_null()
        && PostMessageW(hwnd, WM_CLOSE, 0, 0) != 0
    {
        req.sent += 1;
    }
    1
}

/// Ask a process to close by sending `WM_CLOSE` to its main windows, exactly
/// like the window's close button. The app decides what to do (it may ask to
/// save, or just hide to the tray). Returns how many windows were asked.
pub fn request_close(pid: u32) -> usize {
    let mut req = CloseRequest { pid, sent: 0 };
    unsafe {
        EnumWindows(Some(post_close), &mut req as *mut CloseRequest as LPARAM);
    }
    req.sent
}

struct PromptSearch {
    pid: u32,
    found: bool,
}

/// The class every standard Windows dialog box has (message boxes, "Save
/// changes?" prompts, file pickers).
const DIALOG_CLASS: &str = "#32770";

unsafe extern "system" fn look_for_prompt(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let search = &mut *(lparam as *mut PromptSearch);
    let mut owner_pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut owner_pid);
    if owner_pid != search.pid || IsWindowVisible(hwnd) == 0 {
        return 1;
    }
    // A window with a visible popup of its own has a dialog open on it.
    let popup = GetLastActivePopup(hwnd);
    let mut class = [0u16; 64];
    let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
    if (popup != hwnd && IsWindowVisible(popup) != 0)
        || String::from_utf16_lossy(&class[..len]) == DIALOG_CLASS
    {
        search.found = true;
        return 0; // stop looking
    }
    1
}

/// Whether process `pid` is showing a dialog box (such as "Do you want to save
/// changes?"), which closing it would leave waiting for an answer.
pub fn has_open_prompt(pid: u32) -> bool {
    let mut search = PromptSearch { pid, found: false };
    // SAFETY: the callback only reads window properties; `search` outlives the call.
    unsafe {
        EnumWindows(
            Some(look_for_prompt),
            &mut search as *mut PromptSearch as LPARAM,
        );
    }
    search.found
}

/// End a process immediately. Unsaved work in it is lost.
pub fn terminate_process(pid: u32) -> std::io::Result<()> {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let ok = TerminateProcess(handle, 1);
        let err = std::io::Error::last_os_error();
        CloseHandle(handle);
        if ok == 0 {
            return Err(err);
        }
    }
    Ok(())
}
