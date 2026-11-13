//! Cross-process single-instance lock built on a named Windows mutex
//! (Windows only).
//!
//! The mutex is owned by the kernel, so it is released automatically when the
//! process exits or crashes; a stale lock file can never block the user.

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::Threading::CreateMutexW;

/// Holds the named mutex until dropped.
pub struct InstanceLock(isize);

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateMutexW and is closed exactly once.
        unsafe {
            CloseHandle(self.0 as _);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Try to take the lock called `name` (use a `Local\` prefix so it is
/// per-session). Returns `Ok(None)` if another process already holds it.
pub fn try_lock(name: &str) -> std::io::Result<Option<InstanceLock>> {
    let name = wide(name);
    // SAFETY: `name` is NUL-terminated and outlives the call; a null security
    // descriptor means the default one.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: reads the thread's last error set by CreateMutexW just above.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // Someone else created it first. Drop our extra handle.
        unsafe {
            CloseHandle(handle);
        }
        return Ok(None);
    }
    Ok(Some(InstanceLock(handle as isize)))
}
