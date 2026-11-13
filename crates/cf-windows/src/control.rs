//! Window control: placing windows and showing message boxes (Windows only).

use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsWindow, MessageBoxW, SetWindowPlacement, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK,
    MB_SETFOREGROUND, SW_SHOWMAXIMIZED, SW_SHOWMINNOACTIVE, SW_SHOWNORMAL, WINDOWPLACEMENT,
};

use cf_core::{DisplayState, Rect};

/// Move/resize `hwnd` and apply its show state in one call. `rect` is the
/// *restored* rectangle (the one the window returns to when un-maximized).
pub fn place_window(hwnd: isize, rect: Rect, state: DisplayState) -> std::io::Result<()> {
    unsafe {
        let hwnd = hwnd as HWND;
        if IsWindow(hwnd) == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the window no longer exists",
            ));
        }
        let mut wp: WINDOWPLACEMENT = zeroed();
        wp.length = size_of::<WINDOWPLACEMENT>() as u32;
        wp.showCmd = match state {
            DisplayState::Normal => SW_SHOWNORMAL,
            DisplayState::Maximized => SW_SHOWMAXIMIZED,
            DisplayState::Minimized => SW_SHOWMINNOACTIVE,
        } as u32;
        wp.rcNormalPosition = RECT {
            left: rect.x,
            top: rect.y,
            right: rect.x + rect.width,
            bottom: rect.y + rect.height,
        };
        if SetWindowPlacement(hwnd, &wp) == 0 {
            return Err(std::io::Error::last_os_error());
        }
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
