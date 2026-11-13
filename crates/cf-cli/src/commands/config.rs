//! `contextflow config ...` and the `windows` debug listing.

use std::path::Path;

use anyhow::{bail, Result};

use cf_cli::capture;
use cf_config::Config;

use crate::ConfigCmd;

pub(crate) fn config_cmd(cmd: &ConfigCmd, path: &Path, cfg: Option<&Config>) -> Result<()> {
    match cmd {
        ConfigCmd::Path => println!("{}", path.display()),
        ConfigCmd::Show => {
            let cfg = cfg.expect("config loaded for show");
            println!("# effective configuration ({})", path.display());
            print!("{}", cfg.to_toml());
        }
        ConfigCmd::Init { force } => {
            if path.exists() && !force {
                bail!(
                    "{} already exists; use --force to replace it with the template",
                    path.display()
                );
            }
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, cf_config::TEMPLATE)?;
            println!("Wrote {}", path.display());
        }
    }
    Ok(())
}

pub(crate) fn windows(json: bool) -> Result<()> {
    let windows = cf_windows::enumerate_windows()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&windows)?);
        return Ok(());
    }
    for w in windows {
        println!(
            "{:>6}  {:<10} {:>6},{:<6} {:>5}x{:<5} {:<24} {}",
            w.process_id,
            w.display_state.as_str(),
            w.rect.x,
            w.rect.y,
            w.rect.width,
            w.rect.height,
            w.executable_path
                .as_deref()
                .map(capture::application_id)
                .unwrap_or_else(|| "?".into()),
            w.title
        );
    }
    Ok(())
}
