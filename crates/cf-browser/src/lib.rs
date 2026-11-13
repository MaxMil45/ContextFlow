//! Browser integration.
//!
//! ```text
//!   Chrome / Edge ──(extension)──▶ native host ──▶ live state files ──▶ capture
//!   restore ──▶ browser.exe --new-window <saved urls>
//! ```
//!
//! * The extension reports open windows and tabs to a small local program (the
//!   *native messaging host*). It never sends cookies, history, page content or
//!   private windows.
//! * The host validates everything it receives and keeps the latest state in a
//!   file per browser profile, only while the browser is running.
//! * Capture reads those files and attaches the tabs to the matching windows.
//! * Restore needs no extension: it just starts the browser with the saved
//!   addresses.

pub mod host;
pub mod install;
pub mod protocol;
pub mod saved;
pub mod state;

pub use protocol::{is_restorable_url, LiveState, LiveTab, LiveWindow};
pub use saved::{normalize_url, BrowserAppState, SavedTab, SavedWindow, ADAPTER_ID};
pub use state::{default_state_dir, live_urls, read_live_states};

/// Browsers ContextFlow understands: the application id (the executable name
/// without `.exe`) and the name shown to the user.
pub const BROWSERS: &[(&str, &str)] = &[("chrome", "Google Chrome"), ("msedge", "Microsoft Edge")];

pub fn is_browser(application_id: &str) -> bool {
    BROWSERS.iter().any(|(id, _)| *id == application_id)
}
