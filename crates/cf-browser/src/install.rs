//! Installing the browser integration for the current user (no administrator
//! rights): the extension files, the host manifest, and the registry entries
//! that let Chrome/Edge find the host.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::json;
use thiserror::Error;

pub const HOST_NAME: &str = "dev.contextflow.host";

/// Fixed by the `key` in the extension manifest, so the host manifest can
/// restrict itself to exactly this extension.
pub const EXTENSION_ID: &str = "dklnklfompfninkhpanjobnohejplfnj";

/// The extension, embedded so `contextflow browser install` is self-contained.
pub const EXTENSION_FILES: &[(&str, &str)] = &[
    (
        "manifest.json",
        include_str!("../../../extensions/browser/manifest.json"),
    ),
    (
        "background.js",
        include_str!("../../../extensions/browser/background.js"),
    ),
];

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("could not determine the per-user data directory")]
    NoDataDir,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("could not update the registry ({0})")]
    Registry(String),
}

/// Registry locations Chrome and Edge search for native hosts.
const REGISTRY_KEYS: &[(&str, &str)] = &[
    (
        "Google Chrome",
        r"HKCU\Software\Google\Chrome\NativeMessagingHosts",
    ),
    (
        "Microsoft Edge",
        r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts",
    ),
];

fn browser_dir() -> Result<PathBuf, InstallError> {
    cf_config::project_dirs()
        .map(|d| d.data_dir().join("browser"))
        .ok_or(InstallError::NoDataDir)
}

/// Where the extension is unpacked for "Load unpacked".
pub fn extension_dir() -> Result<PathBuf, InstallError> {
    Ok(browser_dir()?.join("extension"))
}

fn manifest_path() -> Result<PathBuf, InstallError> {
    Ok(browser_dir()?
        .join("native-host")
        .join(format!("{HOST_NAME}.json")))
}

/// Unpack the embedded extension and return its folder.
pub fn write_extension() -> Result<PathBuf, InstallError> {
    let dir = extension_dir()?;
    std::fs::create_dir_all(&dir)?;
    for (name, content) in EXTENSION_FILES {
        std::fs::write(dir.join(name), content)?;
    }
    Ok(dir)
}

/// Contents of the native messaging host manifest.
pub fn host_manifest_json(host_exe: &Path) -> String {
    serde_json::to_string_pretty(&json!({
        "name": HOST_NAME,
        "description": "ContextFlow: lets the ContextFlow browser extension report open tabs to this computer",
        "path": host_exe.display().to_string(),
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")],
    }))
    .expect("manifest serialises")
}

fn reg(args: &[&str]) -> Result<std::process::Output, InstallError> {
    let mut cmd = Command::new("reg");
    cmd.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    Ok(cmd.output()?)
}

#[derive(Debug)]
pub struct InstallReport {
    pub extension_dir: PathBuf,
    pub manifest: PathBuf,
    pub registered: Vec<&'static str>,
}

/// Install everything except loading the extension into the browser, which
/// only the user can do.
pub fn install(host_exe: &Path) -> Result<InstallReport, InstallError> {
    if !host_exe.is_file() {
        return Err(InstallError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} was not found", host_exe.display()),
        )));
    }
    let extension_dir = write_extension()?;
    let manifest = manifest_path()?;
    std::fs::create_dir_all(manifest.parent().expect("has parent"))?;
    std::fs::write(&manifest, host_manifest_json(host_exe))?;

    let mut registered = Vec::new();
    for (name, base) in REGISTRY_KEYS {
        let key = format!(r"{base}\{HOST_NAME}");
        let out = reg(&[
            "add",
            &key,
            "/ve",
            "/t",
            "REG_SZ",
            "/d",
            &manifest.display().to_string(),
            "/f",
        ])?;
        if !out.status.success() {
            return Err(InstallError::Registry(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        registered.push(*name);
    }
    Ok(InstallReport {
        extension_dir,
        manifest,
        registered,
    })
}

/// Remove the registry entries and the host manifest. The extension itself
/// must be removed in the browser's extensions page.
pub fn uninstall() -> Result<(), InstallError> {
    for (_, base) in REGISTRY_KEYS {
        let key = format!(r"{base}\{HOST_NAME}");
        // Not present is fine.
        let _ = reg(&["delete", &key, "/f"])?;
    }
    if let Ok(m) = manifest_path() {
        let _ = std::fs::remove_file(m);
    }
    Ok(())
}

#[derive(Debug)]
pub struct Status {
    /// (browser name, registered)
    pub registered: Vec<(&'static str, bool)>,
    pub manifest_present: bool,
    pub extension_unpacked: bool,
}

pub fn status() -> Status {
    let registered = REGISTRY_KEYS
        .iter()
        .map(|(name, base)| {
            let key = format!(r"{base}\{HOST_NAME}");
            let ok = reg(&["query", &key, "/ve"])
                .map(|o| o.status.success())
                .unwrap_or(false);
            (*name, ok)
        })
        .collect();
    Status {
        registered,
        manifest_present: manifest_path().map(|p| p.is_file()).unwrap_or(false),
        extension_unpacked: extension_dir()
            .map(|d| d.join("manifest.json").is_file())
            .unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use sha2::{Digest, Sha256};

    /// The ID hard-coded here must be the ID Chrome derives from the public
    /// key in the extension manifest, or the host would reject the extension.
    #[test]
    fn extension_id_matches_manifest_key() {
        let manifest: serde_json::Value =
            serde_json::from_str(EXTENSION_FILES[0].1).expect("manifest is JSON");
        let key = manifest["key"].as_str().expect("manifest has a key");
        let der = base64::engine::general_purpose::STANDARD
            .decode(key)
            .unwrap();
        let digest = Sha256::digest(&der);
        let id: String = digest[..16]
            .iter()
            .flat_map(|b| [b >> 4, b & 0x0f])
            .map(|n| (b'a' + n) as char)
            .collect();
        assert_eq!(id, EXTENSION_ID);
    }

    #[test]
    fn host_manifest_is_restricted_to_our_extension() {
        let m: serde_json::Value =
            serde_json::from_str(&host_manifest_json(Path::new(r"C:\x y\host.exe"))).unwrap();
        assert_eq!(m["name"], HOST_NAME);
        assert_eq!(m["type"], "stdio");
        assert_eq!(m["path"], r"C:\x y\host.exe");
        assert_eq!(
            m["allowed_origins"],
            json!([format!("chrome-extension://{EXTENSION_ID}/")])
        );
    }

    #[test]
    fn extension_requests_only_the_permissions_it_needs() {
        let manifest: serde_json::Value = serde_json::from_str(EXTENSION_FILES[0].1).unwrap();
        let mut perms: Vec<String> = manifest["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        perms.sort();
        assert_eq!(perms, ["alarms", "nativeMessaging", "storage", "tabs"]);
        assert!(manifest.get("host_permissions").is_none());
        assert!(manifest.get("content_scripts").is_none());
        // Never anything that could read cookies, history or page content.
        for forbidden in [
            "cookies",
            "history",
            "webRequest",
            "scripting",
            "<all_urls>",
        ] {
            assert!(!EXTENSION_FILES[0].1.contains(forbidden), "{forbidden}");
        }
    }
}
