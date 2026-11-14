//! Programs cleanup refuses to close.
//!
//! Closing most of these would log you out, blank the screen or switch off
//! protection, and none of them is a distraction a workspace should hide.

/// Executable names (without `.exe`, lower case) that are never closed.
const PROTECTED: &[&str] = &[
    // Windows core
    "system",
    "idle",
    "registry",
    "memcompression",
    "smss",
    "csrss",
    "wininit",
    "winlogon",
    "services",
    "lsass",
    "lsaiso",
    "svchost",
    "fontdrvhost",
    "dwm",
    "logonui",
    "userinit",
    "audiodg",
    "spoolsv",
    "wudfhost",
    "wmiprvse",
    "dllhost",
    "conhost",
    // The shell and its parts
    "explorer",
    "sihost",
    "taskhostw",
    "ctfmon",
    "runtimebroker",
    "searchhost",
    "searchindexer",
    "startmenuexperiencehost",
    "shellexperiencehost",
    "textinputhost",
    "applicationframehost",
    "systemsettings",
    "lockapp",
    // Security
    "msmpeng",
    "nissrv",
    "securityhealthservice",
    "securityhealthsystray",
    "smartscreen",
];

/// `Some(reason)` when a program must never be closed. `path` is optional: the
/// name alone is enough to refuse, so a protected program is refused even when
/// it can't be inspected.
pub fn protected_reason(name: &str, path: Option<&str>) -> Option<&'static str> {
    let lower = name.to_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    if stem.starts_with("contextflow") {
        return Some("this is ContextFlow itself");
    }
    if PROTECTED.contains(&stem) {
        return Some("a Windows system program");
    }
    // Anything that lives in the Windows folder (C:\Windows\...).
    if let Some(path) = path {
        let lower_path = path.to_lowercase();
        let mut parts = lower_path.split('\\').filter(|p| !p.is_empty());
        let _drive = parts.next();
        if parts.next() == Some("windows") {
            return Some("a Windows system program");
        }
    }
    None
}
