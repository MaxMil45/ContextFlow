//! Window control: placing windows and showing message boxes (Windows only).

use crate::text::wide;
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    MonitorFromRect, MonitorFromWindow, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, IsHungAppWindow, IsIconic, IsWindow, IsZoomed,
    MessageBoxW, SetForegroundWindow, SetWindowPlacement, SetWindowPos, HWND_TOP,
    MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SW_SHOWMAXIMIZED, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, SW_SHOWNORMAL,
    WINDOWPLACEMENT,
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

/// Put `hwnd` above the other windows without activating it. Raising a series
/// of windows from the bottom of a saved stack to its top rebuilds that stack.
pub fn raise_window(hwnd: isize) -> std::io::Result<()> {
    // SAFETY: plain Win32 call; a stale handle just fails.
    let done = unsafe {
        SetWindowPos(
            hwnd as HWND,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
    if done == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Give `hwnd` the keyboard focus. Windows only lets the program the user is
/// working in take the focus, and a switch can start from a program that is
/// not in front (the tray app after its picker closed, say). When the plain
/// call is refused, the calling thread joins the input queue of the window in
/// front for the moment of the call, which Windows treats as the same user
/// action, unless that window is not responding (then nothing is joined).
/// Returns whether the window ended up in front.
pub fn focus_window(hwnd: isize) -> bool {
    let target = hwnd as HWND;
    // SAFETY: plain Win32 calls; stale handles just fail.
    unsafe {
        if IsWindow(target) == 0 || IsIconic(target) != 0 {
            return false;
        }
        if SetForegroundWindow(target) != 0 {
            return true;
        }
        let front = GetForegroundWindow();
        let front_thread = if front.is_null() {
            0
        } else {
            GetWindowThreadProcessId(front, std::ptr::null_mut())
        };
        let ours = GetCurrentThreadId();
        // Joining the input queue of a program that is not answering can hang
        // this thread too, so a hung window in front is left alone.
        if front_thread == 0 || front_thread == ours || IsHungAppWindow(front) != 0 {
            return false;
        }
        AttachThreadInput(ours, front_thread, 1);
        let done = SetForegroundWindow(target) != 0;
        AttachThreadInput(ours, front_thread, 0);
        done
    }
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
