//! Window control: placing windows and showing message boxes (Windows only).

use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    MonitorFromRect, MonitorFromWindow, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsIconic, IsWindow, IsZoomed, MessageBoxW, SetWindowPlacement, MB_ICONINFORMATION,
    MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, SW_SHOWMAXIMIZED, SW_SHOWMINNOACTIVE,
    SW_SHOWNOACTIVATE, SW_SHOWNORMAL, WINDOWPLACEMENT,
};

use cf_core::{DisplayState, Rect};

/// Move/resize `hwnd` and apply its show state. `rect` is the *restored*
/// rectangle (the one the window returns to when un-maximized).
///
/// A window that is already maximized or minimized can't be moved to another
/// monitor in one `SetWindowPlacement` call: Windows only updates the position
/// it will return to and leaves it maximized where it is. Browsers open new
/// windows maximized on the monitor last used, so that would put every window
/// on one screen. Such a window is first restored onto the target monitor,
/// placed again (its size may have been rescaled if that monitor has a
/// different DPI), and only then maximized or minimized.
pub fn place_window(hwnd: isize, rect: Rect, state: DisplayState) -> std::io::Result<()> {
    unsafe {
        let hwnd = hwnd as HWND;
        if IsWindow(hwnd) == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the window no longer exists",
            ));
        }
        let target = RECT {
            left: rect.x,
            top: rect.y,
            right: rect.x + rect.width,
            bottom: rect.y + rect.height,
        };
        let final_cmd = match state {
            DisplayState::Normal => SW_SHOWNORMAL,
            DisplayState::Maximized => SW_SHOWMAXIMIZED,
            DisplayState::Minimized => SW_SHOWMINNOACTIVE,
        } as u32;

        let not_normal = IsZoomed(hwnd) != 0 || IsIconic(hwnd) != 0;
        let other_monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)
            != MonitorFromRect(&target, MONITOR_DEFAULTTONEAREST);
        if not_normal && (other_monitor || state == DisplayState::Normal) {
            // Restore it onto the target monitor first; twice, because crossing
            // to a monitor with another DPI makes the window rescale itself.
            for _ in 0..2 {
                set_placement(hwnd, &target, SW_SHOWNOACTIVATE as u32)?;
            }
        }
        set_placement(hwnd, &target, final_cmd)?;
        // A normal window that just crossed to a monitor with another DPI may
        // have resized itself; put it back to the saved size.
        if state == DisplayState::Normal && other_monitor {
            set_placement(hwnd, &target, final_cmd)?;
        }
    }
    Ok(())
}

unsafe fn set_placement(hwnd: HWND, rect: &RECT, show_cmd: u32) -> std::io::Result<()> {
    let mut wp: WINDOWPLACEMENT = zeroed();
    wp.length = size_of::<WINDOWPLACEMENT>() as u32;
    wp.showCmd = show_cmd;
    wp.rcNormalPosition = *rect;
    if SetWindowPlacement(hwnd, &wp) == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Show a modal message box (used by the windowless launcher to report problems).
pub fn message_box(title: &str, text: &str, warning: bool) {
    let (t, x) = (wide(title), wide(text));
    let icon = if warning {
        MB_ICONWARNING
    } else {
        MB_ICONINFORMATION
    };
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            x.as_ptr(),
            t.as_ptr(),
            MB_OK | icon | MB_SETFOREGROUND,
        );
    }
}
