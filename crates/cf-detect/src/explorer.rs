//! File Explorer: Windows' own Shell lists every open Explorer window (and
//! every tab of one) with the folder it shows **and the window it is in**. So
//! unlike the editors, Explorer windows are matched by window handle, not by
//! title: it works in every language and tells apart two windows that show the
//! same name. A window with several tabs lists one entry per tab, all with the
//! window's handle; the window's title (`Downloads and 2 more tabs - File
//! Explorer`) names the tab that is in front.
//!
//! The Shell is asked directly through COM (`IShellWindows`), which takes a
//! few milliseconds. An earlier version started PowerShell for the same
//! question, which cost about 350 ms on every save.
//!
//! Only real folders on disk are saved: special places (This PC, Recycle Bin,
//! search results) have no path and are skipped, never guessed.

use crate::{is_dir, is_safe_path, OpenWindow, Sources, State, Window, STATE_SCHEMA};

const TITLE_SUFFIX: &str = " - File Explorer";

/// The folder name from a window title (`Downloads - File Explorer` ->
/// `Downloads`). A window with several tabs names the one in front and counts
/// the others: `Downloads and 2 more tabs - File Explorer` -> `Downloads`.
pub fn name_from_title(title: &str) -> Option<String> {
    let t = title.trim();
    let name = t.strip_suffix(TITLE_SUFFIX).unwrap_or(t).trim();
    let name = match name.rsplit_once(" and ") {
        Some((front, more)) if is_tab_count(more) => front.trim(),
        _ => name,
    };
    (!name.is_empty() && name != "File Explorer").then(|| name.to_string())
}

/// `2 more tabs` or `1 more tab`.
fn is_tab_count(text: &str) -> bool {
    let Some(rest) = text
        .strip_suffix(" more tabs")
        .or_else(|| text.strip_suffix(" more tab"))
    else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

/// One folder Explorer has open (a window, or a tab of one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFolder {
    /// What the Shell calls it: the name in the window title.
    pub name: String,
    pub path: String,
    /// The Explorer window it is in. Every tab of a window has the same one.
    pub handle: isize,
}

/// The folders open in Explorer right now, from Windows' Shell. Empty when the
/// Shell can't be asked (it is never an error to have none).
pub fn open_folders() -> Vec<OpenFolder> {
    // COM state belongs to a thread. A short-lived thread of our own means the
    // caller's thread (the app's UI thread, say) is never touched.
    #[cfg(windows)]
    {
        std::thread::Builder::new()
            .name("explorer-folders".into())
            .spawn(shell::open_folders)
            .ok()
            .and_then(|t| t.join().ok())
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
mod shell {
    use windows::core::{Interface, PCWSTR, PWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Shell::{
        IShellWindows, IWebBrowser2, PathCreateFromUrlW, ShellWindows,
    };

    use super::OpenFolder;

    /// Releases COM on this thread when dropped, if this thread started it.
    struct Com(bool);

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: paired with the successful CoInitializeEx below.
                unsafe { CoUninitialize() };
            }
        }
    }

    pub(super) fn open_folders() -> Vec<OpenFolder> {
        // SAFETY: plain COM calls on a thread that owns its COM state.
        let _com = Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok());
        query().unwrap_or_default()
    }

    fn query() -> windows::core::Result<Vec<OpenFolder>> {
        let mut out = Vec::new();
        // SAFETY: COM calls on this thread; every interface is released on drop.
        unsafe {
            let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
            for i in 0..windows.Count()? {
                let Ok(item) = windows.Item(&VARIANT::from(i)) else {
                    continue; // closed while we were looking
                };
                let Ok(browser) = item.cast::<IWebBrowser2>() else {
                    continue;
                };
                // The list also holds Internet Explorer windows; only Explorer's.
                let is_explorer = browser.FullName().is_ok_and(|p| {
                    let p = p.to_string().to_ascii_lowercase();
                    p.rsplit(['\\', '/']).next() == Some("explorer.exe")
                });
                if !is_explorer {
                    continue;
                }
                let (Ok(name), Ok(url), Ok(handle)) = (
                    browser.LocationName(),
                    browser.LocationURL(),
                    browser.HWND(),
                ) else {
                    continue;
                };
                // Special places (This PC, search results) are not `file:` addresses.
                if let Some(path) = path_from_url(&url.to_string()) {
                    out.push(OpenFolder {
                        name: name.to_string().trim().to_string(),
                        path,
                        handle: handle.0,
                    });
                }
            }
        }
        Ok(out)
    }

    /// `file:///C:/My%20Docs` -> `C:\My Docs`; `None` for anything that isn't a
    /// file or folder address.
    pub(super) fn path_from_url(url: &str) -> Option<String> {
        let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let mut buf = vec![0u16; 32_768];
        let mut len = buf.len() as u32;
        // SAFETY: `wide` is NUL-terminated; `buf` and `len` describe the same buffer.
        unsafe { PathCreateFromUrlW(PCWSTR(wide.as_ptr()), PWSTR(buf.as_mut_ptr()), &mut len, 0) }
            .ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        (!path.trim().is_empty()).then_some(path)
    }
}

/// What each Explorer window shows, from the folders the Shell says are open.
///
/// A window with one folder gets it, whatever its title says. A window with
/// several tabs gets the tab named in its title, if exactly one is. A window
/// with no folder (This PC, a search) gets none; it is never guessed.
pub(crate) fn detect(windows: &[OpenWindow<'_>], sources: &Sources) -> Option<State> {
    sources
        .explorer
        .then(|| detect_in(windows, &open_folders(), &is_dir))
}

/// `exists` says whether a folder is really there (a parameter for tests).
fn detect_in(
    windows: &[OpenWindow<'_>],
    open: &[OpenFolder],
    exists: &dyn Fn(&str) -> bool,
) -> State {
    let usable = |f: &&OpenFolder| is_safe_path(&f.path) && exists(&f.path);
    let windows = windows
        .iter()
        .map(|w| {
            let name = name_from_title(w.title);
            let tabs: Vec<&OpenFolder> = open.iter().filter(|f| f.handle == w.handle).collect();
            let chosen = match tabs.as_slice() {
                [only] => Some(*only),
                [] => None,
                many => {
                    let front: Vec<&&OpenFolder> = many
                        .iter()
                        .filter(|f| {
                            name.as_deref()
                                .is_some_and(|n| f.name.eq_ignore_ascii_case(n))
                        })
                        .collect();
                    match front.as_slice() {
                        [one] => Some(**one),
                        _ => None,
                    }
                }
            };
            let chosen = chosen.filter(usable);
            Window {
                window_title: w.title.to_string(),
                // What a window of this folder is called in its title, so a
                // restore can tell the folder is already open.
                name: name.or_else(|| chosen.map(|f| f.name.clone())),
                path: chosen.map(|f| f.path.clone()),
            }
        })
        .collect();
    State {
        schema_version: STATE_SCHEMA,
        windows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;

    fn folder(name: &str, path: &str, handle: isize) -> OpenFolder {
        OpenFolder {
            name: name.into(),
            path: path.into(),
            handle,
        }
    }

    fn windows<'a>(list: &[(&'a str, isize)]) -> Vec<OpenWindow<'a>> {
        list.iter()
            .map(|&(title, handle)| OpenWindow { title, handle })
            .collect()
    }

    fn paths(state: &State) -> Vec<Option<&str>> {
        state.windows.iter().map(|w| w.path.as_deref()).collect()
    }

    #[test]
    fn names_come_from_explorer_titles() {
        assert_eq!(
            name_from_title("Downloads - File Explorer").as_deref(),
            Some("Downloads")
        );
        assert_eq!(
            name_from_title("Local").as_deref(),
            Some("Local"),
            "Windows 10 titles"
        );
        assert_eq!(name_from_title("File Explorer"), None);
        assert_eq!(name_from_title("  "), None);
    }

    #[test]
    fn tab_counts_in_a_title_are_not_part_of_the_folder_name() {
        for (title, want) in [
            (
                "Downloads and 2 more tabs - File Explorer",
                Some("Downloads"),
            ),
            (
                "Downloads and 1 more tab - File Explorer",
                Some("Downloads"),
            ),
            ("This PC and 2 more tabs - File Explorer", Some("This PC")),
            // A folder that really is called this is left alone.
            ("Salt and pepper - File Explorer", Some("Salt and pepper")),
            (
                "Rock and 2 more things - File Explorer",
                Some("Rock and 2 more things"),
            ),
        ] {
            assert_eq!(name_from_title(title).as_deref(), want, "{title}");
        }
    }

    /// Two windows with the same name get their own folders, because the
    /// Shell says which window each is in. The titles are not even in English.
    #[test]
    fn windows_are_matched_by_handle_in_any_language() {
        let open = vec![
            folder("Téléchargements", r"C:\Users\me\Downloads", 100),
            folder("src", r"C:\work\a\src", 200),
            folder("src", r"C:\work\b\src", 300),
        ];
        let w = windows(&[
            ("Téléchargements - Explorateur de fichiers", 100),
            ("src - Explorer", 300),
            ("src - Explorer", 200),
            ("Ce PC - Explorateur de fichiers", 400),
        ]);
        let s = detect_in(&w, &open, &|_| true);
        assert_eq!(
            paths(&s),
            [
                Some(r"C:\Users\me\Downloads"),
                Some(r"C:\work\b\src"),
                Some(r"C:\work\a\src"),
                None
            ],
            "a place with no folder gets no path"
        );
    }

    #[test]
    fn a_window_with_tabs_gets_the_tab_its_title_names() {
        let open = vec![
            folder("Downloads", r"C:\Users\me\Downloads", 100),
            folder("Documents", r"C:\Users\me\Documents", 100),
            folder("This PC", "", 100),
        ];
        // Only real folders reach this list in practice; the third is dropped
        // by the Shell query. Here it shows that the title picks the tab.
        let w = windows(&[("Documents and 2 more tabs - File Explorer", 100)]);
        assert_eq!(
            paths(&detect_in(&w, &open, &|_| true)),
            [Some(r"C:\Users\me\Documents")]
        );
        // The title names a tab that isn't a folder: nothing is guessed.
        let w = windows(&[("This PC and 2 more tabs - File Explorer", 100)]);
        assert_eq!(paths(&detect_in(&w, &open, &|_| true)), [None]);
        // The title names nothing in the list: nothing is guessed either.
        let w = windows(&[("Elsewhere and 2 more tabs - File Explorer", 100)]);
        assert_eq!(paths(&detect_in(&w, &open, &|_| true)), [None]);
    }

    #[test]
    fn a_folder_that_is_gone_or_unsafe_is_not_saved() {
        let open = vec![folder("a", r"C:\work\gone", 1), folder("b", "relative", 2)];
        let w = windows(&[("a - File Explorer", 1), ("b - File Explorer", 2)]);
        // "a" is gone from disk; "b" is not an absolute path.
        let exists = |path: &str| !path.ends_with("gone");
        let state = detect_in(&w, &open, &exists);
        assert_eq!(paths(&state), [None, None]);
        // The name is still kept, so a restore can tell what the window was.
        assert_eq!(state.windows[0].name.as_deref(), Some("a"));
    }

    #[cfg(windows)]
    #[test]
    fn file_addresses_become_paths_and_other_places_do_not() {
        use super::shell::path_from_url;
        assert_eq!(
            path_from_url("file:///C:/My%20Docs/a%23b").as_deref(),
            Some(r"C:\My Docs\a#b")
        );
        assert_eq!(
            path_from_url("file://server/share/photos").as_deref(),
            Some(r"\\server\share\photos")
        );
        assert_eq!(
            path_from_url("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"),
            None
        );
        assert_eq!(path_from_url("https://example.com"), None);
        assert_eq!(path_from_url(""), None);
    }

    /// Asks the real Shell. Depends on what is open, so it only prints; run it
    /// with `cargo test -p cf-detect real_explorer -- --ignored --nocapture`.
    #[cfg(windows)]
    #[test]
    #[ignore = "depends on the open Explorer windows"]
    fn real_explorer_windows() {
        let began = std::time::Instant::now();
        let folders = open_folders();
        println!("{} folder(s) in {:?}", folders.len(), began.elapsed());
        for f in &folders {
            println!("{}\t{}\twindow {:#x}", f.name, f.path, f.handle);
        }
    }
}
