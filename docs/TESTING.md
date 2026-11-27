# Testing ContextFlow

There are two ways to test: the **automated suite**, which runs in a few seconds and is the same as CI, and a **manual checklist** that runs the real CLI against your own desktop.

---

## 1. Automated tests

Run everything CI runs (format, clippy, tests):

```powershell
powershell -File scripts\check.ps1
```

Or just the tests:

```powershell
cargo test --workspace
```

| Suite | What it proves |
|---|---|
| `cf-core` unit tests | Canonical hashing is stable, validation rejects bad data, monitor-fallback geometry, URL/path redaction |
| `cf-config` | Defaults, overrides, unknown keys reported with line numbers, out-of-range values rejected |
| `cf-storage/tests/storage.rs` | Versioning, immutability triggers, tamper detection, survives reopen, refuses newer schema |
| `cf-storage/tests/lifecycle.rs` | Retention rules, the latest snapshot is never pruned, deletes cascade, versions are never reused, v1→v2 migration, export/import round-trip, a tampered import writes nothing, backups are complete |
| `cf-storage/tests/crash.rs` | Kills a writer process 25 times mid-write. Afterwards there are no partial snapshots, no gaps and no corruption |
| `cf-git` | Porcelain parsing, a real temporary repo, and a git command that overruns its timeout is stopped |
| `cf-diff` | Detects the spec's example divergence, window matching, deterministic output |
| `cf-windows` | Monitors and windows can be enumerated on this machine |
| `cf-engine` | Restore: launches apps and places windows; a missing app, a failed launch, a window that never appears, an elevated app and a missing monitor are each isolated; running it twice opens nothing twice |
| `cf-detect` | Folders and projects of VS Code, Visual Studio, JetBrains IDEs, File Explorer (by window) and Windows Terminal (by its shells): matching, ties never guessed, unsafe or missing paths dropped |
| `cf-browser` | The extension's messages are validated, the live tab files, matching tabs to windows (with their profile folder), which profile folder holds an extension id, and pin and group requests: written once, passed on once per helper, expired and broken ones deleted; groups checked for colour, title and addresses |
| `app/src/App.test.tsx` | The interface against its sample backend (Vitest, jsdom): where "Where you left off" sits, no save setting for an empty workspace, the "save when I leave" switch, no message when leaving a workspace that is expected not to save, History's "Delete older…", the restore preview's wording, the retry button, and Tab staying inside the Ctrl+K palette (`npm test` in `app/`) |
| `extensions/browser/test` | The extension's pinning against a fake browser: the right tabs pinned and grouped, in order, once, in one window, ignoring other browsers, and groups reported at capture (`node extensions/browser/test/pinning.test.js`) |
| `cf-app` terminal hook | The PowerShell hook text: added once, removed exactly, a custom prompt kept (run in a real Windows PowerShell); `cf-windows` reports: believed only when fresh, from a file named by process id |
| `cf-cleanup` | Protected programs are never closed, programs are asked together and waited for once, force close never ends a program that is showing a dialog |
| `cf-app` | Capture, auto-save rules, the switch session (cleanup alongside the restore), start and stop commands, the service the app serves |
| `cf-ipc` | The pipe: protocol, access checks, a server dying mid-request |

For a longer crash run (the plan's 100-iteration target):

```powershell
$env:CF_CRASH_ITERATIONS = 100; cargo test -p cf-storage --test crash; Remove-Item Env:CF_CRASH_ITERATIONS
```

---

## 2. Set up a sandbox for manual testing

Install the CLI so `contextflow` is on your PATH:

```powershell
cargo install --path crates\cf-cli
```

Point ContextFlow at a throwaway folder so testing never touches your real data. This lasts for the current PowerShell window only:

```powershell
$env:CONTEXTFLOW_DB = "$env:TEMP\cf-test\cf.db"
$env:CONTEXTFLOW_CONFIG = "$env:TEMP\cf-test\config.toml"
contextflow info
```

`info` should show all paths under `%TEMP%\cf-test`. Logs and backups go next to the database.

To start over at any time:

```powershell
Remove-Item -Recurse -Force "$env:TEMP\cf-test"
```

---

## 3. Manual checklist

Tick each one off. "Expect" is what a correct build does.

### 0. The main flow: desktop icon (start here)
1. Open the apps you want (for example VS Code, a browser, Character Map) and arrange them. Put at least one on a non-default spot or size.
2. Run:
   ```powershell
   contextflow workspace create "Uni"
   contextflow windows                      # see the app names ContextFlow uses
   contextflow capture Uni --only code,chrome,charmap --note "Working on storage"
   contextflow shortcut Uni
   ```
3. **Expect:** an icon called `Uni` on your Desktop.
4. Close those apps. Double-click the icon.
5. **Expect:** no console window. After a few seconds each app is open again, in its saved position and size. A message box appears only if something couldn't be restored, and it says which app and why.
6. Double-click the icon again.
7. **Expect:** nothing opens twice. The windows are just moved back.
8. Move one window somewhere else, double-click again. **Expect:** it goes back.
9. Preview without changing anything: `contextflow restore Uni --dry-run`.
10. Uninstall one app's path (or capture with a made-up path in a test copy), restore, and **expect** that app to be reported as `EXE_NOT_FOUND` while the others still restore.
11. Unplug a monitor (if you have two) and restore. **Expect:** windows from that monitor appear on the remaining one, fully on screen.

Known limits: apps that run as administrator can't be moved (reported as needing your action). A Microsoft Store app is started by its package id; if that can't be worked out it is skipped and reported. For apps without an integration (see the README's "What comes back"), only the windows come back, not what was open inside.

### 00. Browser tabs
1. Run `contextflow browser install`, then load the extension in Chrome/Edge (see the README), and confirm with `contextflow browser status`.
2. Open a browser window with a few tabs (for example a Google Drive folder and two docs). Capture: `contextflow capture UniWork --only chrome,code --note "tabs"`. **Expect:** `chrome  1 window(s), N tab(s) saved`. `contextflow show UniWork` lists the addresses, `*` marking the front tab.
3. Close Chrome completely, then double-click the icon. **Expect:** Chrome opens with the same tabs in the same window position.
4. Double-click again while Chrome is open. **Expect:** no duplicate tabs or windows.
5. Disable the extension and capture again. **Expect:** a warning that tabs were not saved, and the windows still saved.
6. Add `excluded_url_patterns = ["drive.google.com"]` under `[browser]` in the config and capture. **Expect:** those tabs are not saved.

To try browser restore without touching your own Chrome: `contextflow show <workspace> --json > snapshot.json`, then `cargo run -p cf-engine --example real_chrome -- <empty-folder> snapshot.json`.

### 000. Approval, pinning, minimized windows, double-clicks and VS Code
Use a throwaway database for the first three (see section 2).

**Approving an imported workspace**
1. Create a workspace, capture it (`contextflow capture A --no-windows`), then `contextflow export A --out %TEMP%\a.json` and `contextflow import %TEMP%\a.json --as B`.
2. Run `contextflow workspace list`. **Expect:** `B` is marked `[needs approval]`.
3. Run `contextflow restore B`. **Expect:** it is refused with "has not been approved". `restore B --dry-run` still works.
4. Run `contextflow workspace trust B`. **Expect:** it lists every program from every snapshot version, approves nothing and exits with an error.
5. Run `contextflow workspace trust B --yes`, then `contextflow restore B`. **Expect:** approved, and the restore now runs.

**Pinning a version**
1. Capture workspace `A` twice with notes "one" and "two". Run `contextflow pin A --version 1`.
2. Run `contextflow snapshots A`. **Expect:** v1 is marked `(pinned)`. `contextflow restore A --dry-run` shows v1 and the note "one".
3. Capture a third time. **Expect:** a note saying restore and the icon still use v1.
4. Run `contextflow delete-snapshot A --version 1 --yes`. **Expect:** refused because v1 is pinned. `contextflow prune A --keep-last 1 --yes` also leaves v1.
5. Run `contextflow unpin A`. **Expect:** `restore A --dry-run` now shows v3.

**Minimized windows**
1. Minimize one of your apps, then run `contextflow capture A --dry-run`. **Expect:** a warning that N window(s) are minimized and will reopen minimized.
2. Run `contextflow capture A --unminimize` and `contextflow show A --json`. **Expect:** none of the saved windows has `"display_state": "minimized"`.

**Double-clicking the icon twice quickly**
1. Close the apps in a workspace, then double-click its icon twice within a second or two.
2. **Expect:** only one set of apps starts, and no error message appears. (The second click exits silently because a restore is already running.)

**VS Code reopening its folder**
1. Open VS Code on a project folder and capture: `contextflow capture UniWork --only code`. **Expect:** `code ... 1 folder(s) saved`, and `contextflow show UniWork` lists a `file:///` address under "Folders in code".
2. Close VS Code completely and double-click the icon. **Expect:** VS Code opens that folder in a window at its saved position.
3. Double-click again with VS Code open. **Expect:** no second window for the same folder.
4. If two of your folders share a name, capture the one that is not open in a window. **Expect:** a warning that the folder couldn't be worked out, and the window reopens without a folder.

### A. Capture and inspect
1. Open a few apps (VS Code, File Explorer, a browser) across your monitors.
2. Run these:
   ```powershell
   contextflow workspace create "Uni2"
   contextflow capture Uni2 --note "Started the storage layer"
   contextflow show Uni2
   ```
3. **Expect:** your apps, windows, both monitors, and your note. Positions should match where the windows actually are.
4. Run `contextflow windows` to see exactly which windows ContextFlow can see.
5. Optional: add `--repo C:\path\to\a\git\repo` to also record a repository's branch and change counts. It isn't needed to restore apps.

### B. Exclusion rules (config)
1. Create a config file with `contextflow config init`, then open the file shown by `contextflow config path`.
2. Set `excluded_processes = ["chrome"]` and `excluded_title_patterns = ["private"]`.
3. Run `contextflow capture Uni --dry-run`.
4. **Expect:** no Chrome windows, and a line saying how many windows the rules skipped.
5. Misspell a key (for example `include_windowz = true`) and run `contextflow info`.
6. **Expect:** an error naming the file, the line, the bad key, and the valid keys.

### C. History and Context Diff
1. Capture again with a different note. Repos are reused automatically from the last snapshot.
2. In the repo, switch branch (`git switch -c test-branch`) and edit a file.
3. Run:
   ```powershell
   contextflow snapshots Uni
   contextflow diff Uni --from 1 --live
   ```
4. **Expect:** the branch change, the new modified-file count, any moved, opened or closed windows, and a notice that ContextFlow never switches branches.
5. **Expect:** your repo is untouched. ContextFlow only reads Git.

### D. Integrity
1. Run `contextflow verify`.
2. **Expect:** `database: ok` and every snapshot `ok`.

### E. Retention
1. Make five or more captures.
2. Run `contextflow prune Uni --keep-last 2`.
3. **Expect:** a list of what *would* be deleted. Nothing is deleted yet.
4. Run `contextflow prune Uni --keep-last 2 --yes`.
5. **Expect:** only the 2 newest remain (`contextflow snapshots Uni`).

### F. Deletion never reuses a version number
1. Run `contextflow delete-snapshot Uni --version <latest>`.
2. **Expect:** it refuses without `--yes`.
3. Run the same command with `--yes`, then capture again.
4. **Expect:** the new snapshot gets the next number, not the deleted one.

### G. Export and import
1. Run:
   ```powershell
   contextflow export Uni --out "$env:TEMP\cf-test\uni.json"
   contextflow import "$env:TEMP\cf-test\uni.json"
   ```
2. **Expect:** the import is refused because the name already exists.
3. Run `contextflow import "$env:TEMP\cf-test\uni.json" --as "Uni copy"`.
4. **Expect:** it is imported with the same version numbers. `contextflow diff "Uni copy" --from <a> --to <b>` matches the original.
5. Edit any note text inside `uni.json` and import it with `--as Tampered`.
6. **Expect:** a hash-check failure, and no "Tampered" workspace created.

### H. Backup
1. Run `contextflow backup`.
2. **Expect:** "Backup written and verified" with a path.
3. Run `contextflow --db <that path> workspace list`.
4. **Expect:** the same workspaces.

### I. Logs
1. Open the log folder shown by `contextflow info`.
2. **Expect:** a `contextflow.<date>.log` file with capture and snapshot events.
3. **Expect:** no window titles or note text, and paths under your user folder shown as `~\...`.

### J. Archive and workspace deletion
1. Run `contextflow workspace archive Uni`, then `contextflow capture Uni`.
2. **Expect:** the capture is refused while archived.
3. Run `contextflow workspace unarchive Uni`.
4. Run `contextflow workspace delete "Uni copy"`.
5. **Expect:** it asks for `--yes`.

---

### K. Switching and auto-save (desktop app)

1. Create workspaces **A** and **B**, each with a couple of apps, and capture both.
2. Restore **A** so it is active. Move a window. Press **Ctrl+Alt+W**, pick **B**, press Enter.
   * The corner card shows progress, then "“B” is ready · N s · saved “A” vN".
   * **A**'s History shows a new version marked **Auto**, with A's last note.
3. Switch back to **A** and straight to **B** again without changing anything: no new Auto version (nothing changed).
4. Close every program of **B**, then switch to **A**: **B** is *not* saved ("None of its programs were open").
5. Switch many times: only the newest five Auto versions stay, and versions you captured yourself are never removed by them.
6. Press **Ctrl+Alt+W** again while a switch is running: it cancels before the next app starts.
7. Make a switch partly fail (rename an app's exe): the window opens on the report with **Retry**.
8. Double-click B's desktop icon while in A, and run `contextflow restore A` while in B: both save the workspace being left. `--no-save` skips it.
9. **Leave** asks first, shows what its rules close, and saves the workspace before leaving.

### L. Visual Studio solutions

1. Open a solution (or a folder) in Visual Studio, and another in a second Visual Studio window.
2. Capture. The snapshot lists Visual Studio with "2 folders".
3. Close Visual Studio and restore: each solution reopens in its own window, where it was.
4. Restore again with both open: nothing opens twice.

### M. Restart recovery, notes, always-on, display layouts, Explorer

1. While in a workspace, end ContextFlow from Task Manager (or restart Windows). Start it again: it offers to bring that workspace back. Quit from the tray instead: no offer next time.
2. Press the switcher shortcut, type a note, switch. Come back: the Welcome back card shows the note.
3. Add `spotify` to Always-on apps. Capture: Spotify isn't in the snapshot. A Focus rule for Spotify never closes it.
4. Capture docked, undock, capture again, dock again and restore: the docked layout is used (the report says so).
5. Open two folders in File Explorer, capture, close them, restore: both reopen where they were.
6. Settings → Advanced → Run health check; Save diagnostics report.

### N. Park, timeline, commands, search, JetBrains, privacy

1. In a workspace, Park with a note and switch to another. The sidebar and tray offer Resume; it brings you back and the note shows.
2. Timeline lists the switch, park and resume, with the note and a Go back button.
3. Focus rules → add a start command (`notepad`) and a stop command that waits (`cmd /c exit 0`). Switch in and out: Notepad opens; the report lists both commands.
4. Ctrl+K, type part of a saved tab's address: it finds the workspace that has it.
5. Open a project in PyCharm or CLion, capture, close it, restore: it reopens.
6. Add a private title pattern, capture: that window's title shows as "(hidden)".
7. Export with a passphrase; import it: you're asked for the passphrase, and a wrong one is refused.
8. Turn on Windows High Contrast: every edge and focus ring stays visible. Tab to the workspace tabs and use the arrow keys.

### O. Added in November 2026: check these on a real desk

These were built with tests and small probes only. Each line is one check.

1. **Stacking and focus.** Overlap three windows, note which is on top, capture. Rearrange them, restore. **Expect:** the same window is on top, and typing goes into it.
2. **Cleanup alongside the restore.** Add a Focus rule for a program that hides to the tray (for example Discord). Switch in. **Expect:** the new desk appears straight away; the report still lists the program, and the timing line in `contextflow doctor` shows a `cleanup` phase.
3. **Force close and unsaved work.** Open Notepad (or another editor that asks to save), type something, and add it to a workspace's Focus rules with **Force close**. Switch in. **Expect:** the "save?" prompt stays open, the editor is not ended, and the report says it is showing a dialog.
4. **Windows Terminal.** Open a terminal window with two tabs in different folders (start them with `wt -d <folder>` or "Open in Terminal"). Capture, close it, restore. **Expect:** one window with a tab per folder, where it was. Restore again: no second window. A PowerShell tab that was moved with `cd` comes back in the folder it started in unless the hook is installed (check 12).
5. **A smaller screen.** Capture two windows side by side on a large display, then restore with only the laptop screen. **Expect:** they are still side by side, each about half the screen.
6. **Explorer with tabs.** Open one Explorer window with two or three tabs, capture, close, restore. **Expect:** one window showing the tab that was in front. To check the assumption behind it, run `cargo test -p cf-detect real_explorer -- --ignored --nocapture` with the window open: every tab should print the same window handle.
7. **Delete older versions.** In a workspace with several versions, open History and press the bin above the list. **Expect:** it asks first, then only the newest version (and a pinned one) remain.
8. **Pinned tabs.** Reload the extension in `chrome://extensions` (it should say version 0.4.0 and ask to approve the new "tab groups" permission) after installing the new build. Pin two tabs in a Chrome window, capture, close Chrome, restore. **Expect:** the same two tabs are pinned, in the same order. Then close only that window with Chrome still open and restore again: the same. The helper's log (`browser\host.log` in the data folder that `contextflow info` shows) says "asked the extension to pin 2 tab(s)".
9. **Tab groups.** In a Chrome window put some tabs in a group (give it a title and a colour) and leave others loose, capture, close the window, restore. **Expect:** the same tabs are grouped with the same title and colour, the loose ones are not, and a tab you already had open is not pulled into a group. `contextflow show <workspace> --json` shows a `group` on each grouped tab.
10. **Browser profiles.** With the extension loaded in two Chrome profiles (or Chrome and Edge), open a window in each, capture, close both, restore. **Expect:** each window opens in its own profile, not both in the last-used one. `contextflow show <workspace> --json` shows `"profile": "Profile 2"` (or `Default`) on each window; a window with no `profile` was in a profile that couldn't be worked out. To see which profile folder each connected extension is in: `cargo test -p cf-browser real_profiles -- --ignored --nocapture`.
11. **Opening a vault or project.** Add a start command such as `"...\Obsidian.exe" "obsidian://open?vault=Work"` to a workspace and switch in. **Expect:** the app shows that vault.
12. **PowerShell folder hook.** Run `contextflow terminal hook status`, then `contextflow terminal hook install` (it names the profile file it changes). Open a **new** PowerShell tab in Windows Terminal, `cd` somewhere else, capture, close the window, restore. **Expect:** the tab comes back in the folder you moved to, and the prompt looks exactly as before. `contextflow terminal hook uninstall` removes it again (your own profile lines stay). To try it without touching your profile: `contextflow terminal hook install --profile %TEMP%\test-profile.ps1` and start a tab with `powershell -NoProfile -NoExit -File <a script that dot-sources that file and does the cd>`.
13. **Saving when you leave.** Open a workspace you never need to save (a game, say) and turn **Save this workspace when I leave it** off. Switch into it from another workspace and out again. **Expect:** no "not saved" message either time, and no new version in its History. Switch into a workspace whose programs are all closed: the same, no message. Turn the switch on for a normal workspace: switching away still saves it and says "Saved … as vN" in the report.

Three checks start real programs and are skipped in the normal run. Run them by hand when changing that code:

```powershell
cargo test -p cf-windows real_terminal -- --ignored --nocapture   # opens and closes a terminal window
cargo test -p cf-windows real_shells -- --ignored --nocapture     # what PowerShell and cmd report after a cd
cargo test -p cf-cleanup real_dialog -- --ignored --nocapture     # shows a message box for a moment
```

## 4. Not built yet

VS Code's open files, the other tabs of an Explorer window, and the contents of any app without an integration (only its windows return). Tab and folder comparison in `diff` is also not built. The full list, in order, is in [ROADMAP.md](ROADMAP.md).

## 5. Reporting a problem

Include the output of `contextflow info`, the failing command and its output, and the relevant lines from the log file.
