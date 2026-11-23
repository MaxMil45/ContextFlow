# Roadmap: what is left, what to improve, and ideas

This is the one list of work still to do. Other documents link here instead of
keeping lists of their own. Written 16 November 2026; update it when an item is
done or a decision changes.

How the project is built is in [ARCHITECTURE.md](ARCHITECTURE.md) (short) and
[SOFTWARE_ARCHITECTURE.md](SOFTWARE_ARCHITECTURE.md) (full).

---

## 1. Where the project stands

* The six-step plan of November 2026 is finished, and so is a list of fixes
  made after it (see ARCHITECTURE.md, section 3).
* `cargo fmt`, `cargo clippy -D warnings` and the test suite pass, and CI runs
  them on every push.
* Distribution is through GitHub for now: an unsigned installer attached to a
  Release, and a browser extension installed with Developer mode.
* Version in the code is `0.1.0`. No release has been tagged yet.

---

## 2. Tasks only the owner can do

In order.

| # | Task | How | Done when |
|---|---|---|---|
| 1 | **Try the current build on a real desk** | `powershell -File scripts\build-installer.ps1`, quit ContextFlow from the tray, run the installer. Then go through section O of [TESTING.md](TESTING.md). | Every line of section O behaves as written, or a problem is noted here. |
| 2 | **Record the switch** | 15 to 20 seconds, about 1000 px wide, a few MB at most, an animated GIF (ScreenToGif exports one). Save as `docs/switch.gif` and replace the placeholder comment near the top of `README.md` with `![A one-key switch](docs/switch.gif)`. | The README shows a moving switch. |
| 3 | **Tag the first release** | `git tag v0.1.0` then `git push origin v0.1.0`. The release workflow builds the installer and attaches it to a GitHub Release. This is the workflow's first run, so check the result. | The Releases page has `ContextFlow_0.1.0_x64-setup.exe`. |
| 4 | **Replace the placeholder app icon** | Put the real picture in `app/app-icon.png` and run `npx tauri icon app-icon.png` in `app/`. | The installer and tray show the real icon. |
| 5 | **Install and uninstall once on a clean Windows** (a virtual machine is enough) | Run the installer, do the main flow, uninstall. | Nothing is left behind and nothing needed a developer tool. |

Deliberately later, when the project goes beyond GitHub: publish the extension
to the Chrome and Edge stores (their extension ids then go in
`crates/cf-browser/src/install.rs`, which today allows only the unpacked id),
and sign the installer (`CONTEXTFLOW_SIGN_THUMBPRINT` is already supported by
the build script and both workflows).

---

## 3. Built but not yet seen working on a real desk

These pass their tests with a fake desktop, and where possible a small real
probe. None has been through a real switch, because development never ran one
against the live desktop.

| Feature | What was checked | What to look for |
|---|---|---|
| Stacking order and focus after a switch | Fake desktop; a harmless call on an invalid window | The window that was on top is on top, and typing goes into it. |
| Cleanup running alongside the restore, both cleanups together | Unit tests of the threading | With cleanup rules, the new desk appears without waiting for old programs to close. |
| Force close leaves a program that is asking to save | Fake tests, and a real message box was detected | An editor showing "Save changes?" is not ended and is reported. |
| Windows Terminal tabs | A real capture and a real launch of a test window | A full switch reopens the window with a tab per folder, in place. |
| Windows saved on a larger display | Unit and engine tests | Side-by-side windows stay side by side on the laptop screen. |
| Explorer windows with several tabs | Tests with made-up data | Each tab of one window reports the same window; check with `cargo test -p cf-detect real_explorer -- --ignored --nocapture`. |
| Pinned tabs come back pinned | Engine tests, the extension against a fake browser, and the real helper program passing a request on | Restore a window with pinned tabs, once with Chrome closed and once with it open: the same tabs are pinned again. Needs the extension reloaded (version 0.3.0) and the new build installed. |
| "Delete older versions" in the History tab | Typecheck and the storage tests behind it | The button appears with two or more versions and keeps the newest and the pinned one. |

---

## 4. Improvements, in order of value

Sizes are estimates. "Done when" is the test for finishing.

### Worth doing next

| # | Improvement | Why | Size | Done when |
|---|---|---|---|---|
| 1 | **Tab groups** | Uses the channel built for pinned tabs (see SOFTWARE_ARCHITECTURE.md, section 5.4). Needs the `tabGroups` permission and the group's name, colour and members saved at capture. | 2 to 3 days | Groups come back with name and colour. |
| 2 | **Browser profiles** | With work and personal profiles, restored tabs open in whichever was used last. The id the extension sends is its own, not Chrome's profile folder, so a mapping is needed (read Chrome's `Local State`, find the profile folders that have the extension). Only matters with more than one profile. | 3 days | Each window reopens in the profile it was saved from. |
| 3 | **PowerShell's current folder** | PowerShell doesn't tell Windows when you `cd`, so a tab is saved with the folder it started in. Offer an opt-in line for the PowerShell profile that reports the folder to ContextFlow. Windows Terminal keeps this information to itself, so there is no way without the shell's help. | 1 to 2 days | With the line installed, a tab that moved is saved where it is. Without it, nothing changes. |
| 4 | **Telling windows apart by more than their title** | Two windows with the same title, or a title that changed, can be saved without a folder or paired with the wrong position. Combine title, position and the order the windows were created; for browsers, the extension's own window id. Never guess on a tie. | 3 days | Two same-named windows of one app come back in their own places. |

### Good, but after the above

| # | Improvement | Why | Size |
|---|---|---|---|
| 6 | **Virtual desktops** | Windows on another virtual desktop are ignored, which can start a second copy. Needs a decision first: bring them to the current desktop, or leave them where they are. | 2 to 3 days |
| 7 | **Environment variables per workspace** | For terminals and editors that ContextFlow starts (not ones already running, which keep theirs). Values may be secrets, so they must stay out of logs and exports. | 2 days |
| 8 | **Every tab of an Explorer window** | Only the front tab is saved. Reopening tabs needs a reliable way to drive Explorer, which is not known yet. | Unknown |
| 9 | **More apps** | `cf-detect` takes a new app as a table entry and a small module: Notepad++, Sublime Text, Office documents (from their recent lists). | About a day each |
| 10 | **An updater** | People on an old build stay on it. Tauri has an updater; it needs signed update files. | 2 days, after signing |
| 11 | **Tests for the user interface** | The Rust side has several hundred tests; the React side has none beyond the typecheck. `npm run preview:mock` (sample data in a browser) is the starting point: the same sample answers can drive automated checks of the main screens. | 2 days for the main flows |

### Only with evidence

Do these when a measurement or a report shows the need, not before.

| Improvement | The evidence to wait for |
|---|---|
| Event-driven window detection instead of polling | A measured switch where polling is a real share of the time. The first poll is already at 25 ms. |
| One database connection owned by the tray app | Database contention or slow start-up actually seen. |
| Progress streamed to the plain command line | People using long restores from a terminal and asking. The on-screen card already shows progress for icon launches. |
| One placement instead of two for new windows | Someone seeing a window visibly jump. |
| Encrypting the database | A real requirement. It needs an encrypted SQLite build, not just a key store; a stolen laptop is already covered by disk encryption, and exports can already be encrypted. |

---

## 5. Ideas, not yet weighed

Each needs a short look before it becomes a plan.

* **Switch automatically**: when docking or undocking, on a known Wi-Fi
  network, at a time of day, or before a calendar meeting. Only once a manual
  switch is trusted, because an automatic one that goes wrong is more
  surprising.
* **Sound and power per workspace**: mute or pause on entering a meeting
  workspace, a power plan for heavy work. Useful, but a different kind of
  feature from putting windows back.
* **A workspace per virtual desktop**: bind a workspace to one of Windows'
  own desktops and switch both together.
* **A first-run demo**: a sample workspace that shows a switch before the user
  has saved anything.
* **Switch timings over time**: `doctor` shows the last switch; a short
  history would show whether a change made things faster or slower.
* **Share a workspace as a template**: export exists; a template would leave
  out personal paths and ask for them on import.
* **Other languages**: Explorer's window titles are read in English to tell
  whether a folder is already open. Matching by window handle is language
  free; this last use of the title could go the same way.

---

## 6. Looked at and turned down

So they are not proposed again without new facts.

| Idea | Why not |
|---|---|
| A helper with `uiAccess` to move administrator windows | Needs a signed program in a protected folder; the installer is per-user and unsigned. The report already says which window needs moving by hand. |
| Reading all processes in one kernel call | A full window scan takes about 65 ms and happens a few times per switch. No visible gain for unsafe, poorly documented calls. |
| Replacing the two reader threads in the Git helper | They are what make the timeout possible, and cost almost nothing. |
| Matching VS Code windows by command line or window class | One process hosts every VS Code window, and every window of an app has the same class. |
| A new feature for "open this vault or project" | A workspace's start command already does it; see [COMMANDS.md](COMMANDS.md). |

---

## 7. Housekeeping notes

* Four small helpers exist only for tests (`content_hash`, `dacl_entries`,
  `stop_handle`, `raw_connection`). They are small and used; leave them.
* The pipe and its protocol still carry the name "Agent" from an earlier
  design. Renaming would change the protocol for no gain.
* `docs/history/` holds the original plan and blueprint, unchanged. They
  describe the past.
* Every capture starts from `CaptureOptions::from_config` (in `cf-app`) and
  changes only what differs. Don't build the options by hand again: five
  hand-built copies had drifted (the CLI's `diff --live` looked up no editor
  folders).
* Known duplication left on purpose: the "close on outside click or Escape"
  code in `IconPicker.tsx` and the menu in `ui.tsx` (the menu also handles
  arrow keys, so a shared hook would split its key handling), and
  `WorkspaceView.tsx` at about 1,150 lines is the next file worth splitting
  (its dialogs and the History tab could be their own files).

---

## 8. Working on this project

1. Read [ARCHITECTURE.md](ARCHITECTURE.md), then this page.
2. Before saying anything is done, run `cargo fmt --all`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, and `npm run typecheck` in `app/` when the
   interface or its generated types changed. `powershell -File scripts\check.ps1`
   runs all of them the way CI does.
3. Never test with a real restore, switch or leave on the desktop you are
   working on. Use tests, `--dry-run`, or a separate database (`--db`).
4. Prefer removing and reusing to adding. Check whether the feature already
   exists before building it.
5. Update the documents in the same change, and tick the item off here.
