use std::ffi::c_void;
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    MDT_EFFECTIVE_DPI,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindow, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed, GW_OWNER,
};

use cf_core::{DisplayState, MonitorInfo, Rect};

use crate::{DiscoveredWindow, Result};

const MONITORINFOF_PRIMARY: u32 = 1;

pub fn init_dpi_awareness() {
    // Fails harmlessly if awareness was already set (e.g. by the app manifest).
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

fn rect_from(r: &RECT) -> Rect {
    Rect {
        x: r.left,
        y: r.top,
        width: r.right - r.left,
        height: r.bottom - r.top,
    }
}

fn utf16_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

unsafe extern "system" fn collect_hwnd(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let handles = &mut *(lparam as *mut Vec<HWND>);
    handles.push(hwnd);
    1
}

unsafe fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    let hr = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED as _,
        &mut cloaked as *mut u32 as *mut c_void,
        size_of::<u32>() as u32,
    );
    hr >= 0 && cloaked != 0
}

unsafe fn window_text(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    utf16_to_string(&buf[..copied.max(0) as usize])
}

unsafe fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    utf16_to_string(&buf[..n.max(0) as usize])
}

unsafe fn process_image_path(pid: u32) -> Option<String> {
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if handle.is_null() {
        return None; // e.g. elevated or protected process
    }
    let mut buf = vec![0u16; 32_768];
    let mut size = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut size);
    CloseHandle(handle);
    (ok != 0).then(|| utf16_to_string(&buf[..size as usize]))
}

unsafe fn monitor_device_name(hmon: HMONITOR) -> Option<(String, MONITORINFO)> {
    let mut info: MONITORINFOEXW = zeroed();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hmon, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) == 0 {
        return None;
    }
    Some((utf16_to_string(&info.szDevice), info.monitorInfo))
}

pub fn enumerate_windows() -> Result<Vec<DiscoveredWindow>> {
    let mut handles: Vec<HWND> = Vec::new();
    unsafe {
        if EnumWindows(Some(collect_hwnd), &mut handles as *mut Vec<HWND> as LPARAM) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }

    let mut out = Vec::new();
    for hwnd in handles {
        unsafe {
            // User-facing top-level windows only: visible, unowned, not cloaked
            // (hidden UWP frames / other virtual desktops), with a title.
            if IsWindowVisible(hwnd) == 0
                || !GetWindow(hwnd, GW_OWNER).is_null()
                || is_cloaked(hwnd)
            {
                continue;
            }
            let title = window_text(hwnd);
            if title.trim().is_empty() {
                continue;
            }
            let mut r: RECT = zeroed();
            if GetWindowRect(hwnd, &mut r) == 0 {
                continue;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);

            let display_state = if IsIconic(hwnd) != 0 {
                DisplayState::Minimized
            } else if IsZoomed(hwnd) != 0 {
                DisplayState::Maximized
            } else {
                DisplayState::Normal
            };
            let monitor_id = monitor_device_name(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST))
                .map(|(name, _)| name);

            out.push(DiscoveredWindow {
                hwnd: hwnd as isize,
                process_id: pid,
                executable_path: process_image_path(pid),
                title,
                class_name: class_name(hwnd),
                rect: rect_from(&r),
                display_state,
                monitor_id,
            });
        }
    }
    Ok(out)
}

unsafe extern "system" fn collect_monitor(
    hmon: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let monitors = &mut *(lparam as *mut Vec<HMONITOR>);
    monitors.push(hmon);
    1
}

pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        if EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_monitor),
            &mut handles as *mut Vec<HMONITOR> as LPARAM,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }

    let mut out = Vec::new();
    for hmon in handles {
        unsafe {
            let Some((id, info)) = monitor_device_name(hmon) else {
                continue;
            };
            let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
            let dpi = (GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) >= 0)
                .then_some(dpi_x);
            out.push(MonitorInfo {
                id,
                primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
                rect: rect_from(&info.rcMonitor),
                work_area: rect_from(&info.rcWork),
                dpi,
            });
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}
