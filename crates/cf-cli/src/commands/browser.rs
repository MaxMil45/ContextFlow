//! `contextflow browser ...`

use anyhow::{Context as _, Result};

use crate::BrowserCmd;

pub(crate) fn browser_cmd(cmd: &BrowserCmd) -> Result<()> {
    use cf_browser::install;
    match cmd {
        BrowserCmd::Install => {
            let host = std::env::current_exe()?.with_file_name("contextflow-browser-host.exe");
            let report = install::install(&host).with_context(|| {
                "install failed (build/install both programs: cargo install --path crates/cf-cli)"
            })?;
            println!(
                "Registered ContextFlow with: {}",
                report.registered.join(", ")
            );
            println!("Extension files: {}", report.extension_dir.display());
            println!("\nOne manual step (only you can do it, in your browser):");
            println!("  1. Open chrome://extensions  (or edge://extensions)");
            println!("  2. Turn on \"Developer mode\"");
            println!("  3. Click \"Load unpacked\" and choose the folder above");
            println!("Then run `contextflow browser status` to check it is connected.");
            println!(
                "\nThe extension can see each tab's address and title only. It cannot read cookies, history, passwords or page content, and it ignores private windows."
            );
        }
        BrowserCmd::Uninstall => {
            install::uninstall()?;
            println!("Removed the registration. Remove the \"ContextFlow\" extension on your browser's extensions page to finish.");
        }
        BrowserCmd::Status => {
            let s = install::status();
            for (name, ok) in &s.registered {
                println!(
                    "{:<16} registered: {}",
                    name,
                    if *ok { "yes" } else { "no" }
                );
            }
            println!(
                "host manifest    {}",
                if s.manifest_present {
                    "present"
                } else {
                    "missing"
                }
            );
            println!(
                "extension files  {}",
                if s.extension_unpacked {
                    "unpacked"
                } else {
                    "not unpacked"
                }
            );
            let dir = cf_browser::default_state_dir();
            for (id, name) in cf_browser::BROWSERS {
                let states = dir
                    .as_deref()
                    .map(|d| cf_browser::read_live_states(d, id))
                    .unwrap_or_default();
                if states.is_empty() {
                    println!("{name:<16} extension connected: no (browser closed, or extension not loaded)");
                } else {
                    let windows: usize = states.iter().map(|s| s.windows.len()).sum();
                    let tabs: usize = states
                        .iter()
                        .flat_map(|s| &s.windows)
                        .map(|w| w.tabs.len())
                        .sum();
                    println!(
                        "{name:<16} extension connected: yes, {windows} window(s), {tabs} web tab(s) visible"
                    );
                }
            }
            if s.registered.iter().all(|(_, ok)| !ok) {
                println!("\nNot set up yet: run `contextflow browser install`.");
            }
        }
    }
    Ok(())
}
