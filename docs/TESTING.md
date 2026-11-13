# Testing ContextFlow (Phase 1)

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

Known limits: Microsoft Store apps (new Notepad, Calculator, Windows Terminal) are skipped and reported. Apps that run as administrator can't be moved (reported as needing your action). Only each app's windows come back, not what was open inside (which folder, which tabs).

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

## 4. Not built yet

Restoring what is *inside* an app (VS Code folders and tabs, browser tabs, terminal directories) comes with the Phase 3 integrations. Today only the apps and their windows come back.

## 5. Reporting a problem

Include the output of `contextflow info`, the failing command and its output, and the relevant lines from the log file.
