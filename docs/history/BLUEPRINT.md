# ContextFlow: Architecture Audit and Execution Blueprint

Audited against the working tree on 2026-11-13: 9 crates, about 9,700 lines of Rust, 100 passing tests, last commit `047671d` plus 22 uncommitted files.

How to use this document: read sections 1 to 3 once, then run the tasks in section 4 **in order**, one per session of the execution model. Each task prompt is self-contained when pasted together with the shared preamble in 4.0.

---

## 1. Critical flaws and edge cases

Severity: **S1** = can harm the user or their data, **S2** = wrong behaviour users will hit, **S3** = debt that slows future work. "Fix" names the task in section 4, or says why it is accepted.

### Security

| # | Sev | Flaw | Where | Fix |
|---|-----|------|-------|-----|
| F1 | S1 | **Importing a workspace is arbitrary code execution on restore.** `import` verifies hashes, but the attacker computes those. `executable_path` and `launch_arguments` in an imported snapshot are launched as-is by `restore`. A shared export file can run `powershell.exe -enc ...`. | `cf-storage/src/export.rs`, `cf-engine/src/lib.rs` (`plan_launch`) | T5 |
| F2 | S2 | Restore will launch any path that exists, including relative paths and non-`.exe` files. | `plan_launch` | T5 |
| F3 | S2 | Snapshots, exports and live tab files hold full URLs (query strings can carry tokens) and window titles in plain text. Only opt-in `excluded_url_patterns` protects them. | `cf-browser`, `cf-storage` | Accepted for now (local-first, per-user profile). Documented. Revisit with DPAPI if exports become a sharing feature. |
| F4 | S3 | Any local extension that copies the public `key` gets the same extension ID and can feed the host fake tabs. Same-user attacker only; the host accepts http/https only and writes one file. | `cf-browser/src/install.rs` | Accepted. Publishing to the Web Store (signed ID) closes it. |

### Logic errors and races

| # | Sev | Flaw | Where | Fix |
|---|-----|------|-------|-----|
| F5 | S2 | **Double double-click races.** Two `contextflow-launch.exe` processes both see "not running" and both start every app. | `cf-cli/src/launch.rs`, `session.rs` | T4 |
| F6 | S2 | **The icon restores the latest snapshot, so one bad capture silently breaks it.** This already happened (v2 of UniWork saved three apps minimized). | `session::restore_workspace` | T6 (pin a version, warn on minimized) |
| F7 | S2 | **Version-stamped install folders break restore after an app updates.** Discord is saved as `...\Discord\app-1.0.9267\Discord.exe`; after its next update that path is gone and restore reports `EXE_NOT_FOUND`. Same for Slack and other Squirrel apps. | `plan_launch` | T7 |
| F8 | S2 | **A slow browser delays everything.** `restore_tabs` runs inline inside the per-app loop and blocks up to 30 s for the first window and 10 s for each further one, before later apps are even started. Total restore time is unbounded (the sum of all waits). No cancellation. | `cf-engine/src/lib.rs`, `browser.rs` | T8 |
| F9 | S2 | VS Code reopens without its folder. No launch context is captured for any app. | `cf-cli/src/capture.rs` | T9a, T9b |
| F10 | S2 | Two browser windows with the same title can swap tabs and positions, because tabs are tied to desktop windows by title only. | `cf-browser/src/saved.rs` (`match_windows`) | T10 |
| F11 | S2 | After starting a browser, the "first new window" is assumed to be ours. If Chrome also restores its previous session, the wrong window can be placed. | `browser.rs` | Accepted. Documented; needs the extension to confirm tab contents, which is out of scope. |
| F12 | S2 | Application identity is the lower-case exe stem. Different apps that share a stem (`javaw`, `electron`, `python`, `update`) collide: windows of one are treated as the other. | `application_id()` | Accepted for v1. Fix needs process identity (path + start time), listed under "Deferred". |
| F13 | S3 | Monitor ids are GDI device names (`\\.\DISPLAY1`), which can change on docking. Mixed-DPI moves are not rescaled. Fallback keeps windows on screen, so the failure is mis-placement, not loss. | `cf-windows/src/imp.rs` | Deferred (PLAN WP 2.2). |
| F14 | S3 | Windows on other virtual desktops are cloaked and silently skipped. UWP apps hosted by `ApplicationFrameHost` (Calculator, Settings) cannot be captured at all. | `cf-windows/src/imp.rs` | Accepted and documented. |

### Maintainability and scale

| # | Sev | Flaw | Where | Fix |
|---|-----|------|-------|-----|
| F15 | S3 | `cf-cli/src/main.rs` is 1,185 lines holding every command. Every feature edits it, which is where a small model is most likely to break something. | `cf-cli` | T2 |
| F16 | S3 | `cf-adapters` defines an async `ContextAdapter` trait that nothing implements. The engine special-cases browsers and Store apps directly. The crate is used only for the `RestoreStatus` enum and drags in `async-trait` and `tokio`. | `cf-adapters` | T1 (delete it; move the enum) |
| F17 | S3 | The CLI has no automated tests. Every CLI behaviour so far was checked by hand. | `cf-cli` | T3 |
| F18 | S3 | `host.log` and `restore_runs` grow without limit. | `browser_host.rs`, `history.rs` | T11 |
| F19 | S3 | The `snapshot_*` projection tables are written on every capture and never read. | `cf-storage/src/repo.rs` | Reversed (migration `0006`): dropped. Context Diff reads the hashed payload, not these tables, so they were only extra writes. The crash test now checks payload completeness instead. |
| F20 | S3 | No real-window integration test. Restore is unit-tested against a fake desktop and was checked by hand with Character Map and a throwaway Chrome profile. | `cf-engine` | Deferred (`cf-testwin`, PLAN WP 2.4). |

---

## 2. High-impact improvements

Ordered by value. Each is one task in section 4. Nothing here adds a daemon, a GUI or a new process model.

1. **Trust gate for imports (T5).** One boolean per workspace. Imported workspaces cannot be restored until the user reviews the executables and approves. Closes F1 and F2.
2. **Pin the version the icon restores, and warn on minimized captures (T6).** A nullable `pinned_version` column. Restore uses it when set. Closes F6.
3. **Single-instance lock for restore (T4).** A named Windows mutex. Closes F5.
4. **Resolve version-stamped folders (T7).** When the saved exe is missing and its folder looks like `app-<version>`, use the newest sibling that contains the same exe. Closes F7.
5. **Bound and reorder restore (T8).** Start ordinary apps first, then do the browser work, under one overall deadline and a cancel flag. Closes F8.
6. **VS Code folders without an extension (T9a, T9b).** Read the folder name from the window title and resolve it through VS Code's own recently-opened list. Reopen with `--folder-uri`. Closes most of F9.
7. **Match browser windows by position as a tie-break (T10).** The extension adds each window's bounds. Closes F10.
8. **Structural cleanup first (T1, T2, T3).** Delete the unused adapter crate, split `main.rs` into one file per command group, add a CLI test harness. These make every later task smaller and verifiable.

Explicitly **not** doing (over-engineering for the current stage): the Tauri UI, a background daemon, named-pipe IPC, encryption at rest, a plugin SDK, Firefox, File Explorer folder restore, replacing title-based matching with process identity.

---

## 3. File and data structure

### 3.1 Target folder hierarchy (after all tasks)

```text
ContextFlow/
├── Cargo.toml
├── scripts/check.ps1                      fmt + clippy -D warnings + tests (the only gate)
├── extensions/browser/
│   ├── manifest.json                      version 0.2.0 after T10; "key" never changes
│   └── background.js
├── docs/  PLAN.md  TESTING.md  BLUEPRINT.md
└── crates/
    ├── cf-core/src/
    │   ├── model.rs                       + RestoreStatus (T1)
    │   ├── geometry.rs  hash.rs  redact.rs  validate.rs  error.rs  lib.rs
    ├── cf-config/src/lib.rs               + timeouts.restore_overall_secs (T8)
    ├── cf-storage/
    │   ├── migrations/
    │   │   ├── 0001_initial.sql           NEVER EDIT
    │   │   ├── 0002_version_counter.sql   NEVER EDIT
    │   │   ├── 0003_trust.sql             (T5)
    │   │   └── 0004_pinned_version.sql    (T6)
    │   └── src/  repo.rs  export.rs  history.rs  migrations.rs  error.rs  lib.rs
    ├── cf-git/  cf-diff/                  unchanged
    ├── cf-windows/src/
    │   ├── imp.rs  control.rs  lib.rs
    │   └── lock.rs                        single-instance mutex (T4)
    ├── cf-browser/src/                    protocol.rs  saved.rs  state.rs  host.rs  install.rs  lib.rs
    ├── cf-vscode/src/lib.rs               (T9a) new crate
    ├── cf-engine/src/
    │   ├── lib.rs                         restore(), plan_launch(), finish()
    │   ├── desktop.rs                     Desktop trait + WindowsDesktop
    │   ├── resolve.rs                     versioned-folder resolution (T7)
    │   ├── window_open.rs                 shared "open, find new window, place" helper (T9b)
    │   ├── browser.rs  vscode.rs (T9b)
    │   └── tests.rs  tests_browser.rs  tests_vscode.rs (T9b)
    └── cf-cli/
        ├── src/
        │   ├── main.rs                    clap types + dispatch only (T2)
        │   ├── lib.rs  capture.rs  logging.rs  session.rs  shortcut.rs
        │   ├── launch.rs  browser_host.rs  (the two extra binaries)
        │   └── commands/                  (T2)
        │       ├── mod.rs  util.rs
        │       ├── workspace.rs  capture_cmd.rs  restore.rs
        │       ├── history.rs  maintenance.rs  browser.rs  config.rs
        └── tests/cli.rs                   (T3)
```

`crates/cf-adapters/` is deleted in T1.

### 3.2 Database schema changes

Rules: migrations are append-only, numbered, and registered in `cf-storage/src/migrations.rs` in order. `PRAGMA user_version` ends at 4.

```sql
-- 0003_trust.sql  (T5)
-- Imported workspaces must be approved before they can be restored.
ALTER TABLE contexts ADD COLUMN is_trusted INTEGER NOT NULL DEFAULT 1 CHECK (is_trusted IN (0, 1));

-- 0004_pinned_version.sql  (T6)
-- The snapshot version that restore uses; NULL means "latest".
ALTER TABLE contexts ADD COLUMN pinned_version INTEGER CHECK (pinned_version IS NULL OR pinned_version > 0);
```

No other table changes. The `cf_core::Context` struct is **not** changed; the two columns are read through dedicated `Database` methods.

### 3.3 Snapshot payload contract

`SnapshotPayload.schema_version` **stays 1**. `validate::payload` rejects anything else, and old snapshots are verified by hashing their stored bytes, so the payload may only grow through data that old readers ignore. All new per-app data goes in the existing `ApplicationState.adapter_id` / `adapter_state` pair:

| `adapter_id` | `adapter_state` JSON | Written by | Read by |
|---|---|---|---|
| `"packaged-app"` | `{"aumid": "Family!AppId"}` | capture (exists) | `plan_launch` (exists) |
| `"browser"` | `cf_browser::BrowserAppState` `{schema_version:1, browser, windows:[{window_title, tabs:[{url,title,pinned}], active_tab}]}` | capture (exists) | `browser.rs` (exists) |
| `"vscode"` | `cf_vscode::VsCodeState` `{schema_version:1, windows:[{window_title, root_name, folder_uri}]}` | T9a | T9b |

`folder_uri` is `null` when the folder could not be resolved. Only `file:///` URIs are ever stored or reopened.

### 3.4 Extension → host message (after T10)

```json
{ "type": "state", "browser": "chrome|msedge", "profile_id": "<[A-Za-z0-9-]{1,64}>", "sent_at": 0,
  "windows": [ { "focused": true, "state": "normal",
                 "bounds": { "left": 0, "top": 0, "width": 0, "height": 0 },
                 "tabs": [ { "index": 0, "url": "https://…", "title": "…", "pinned": false, "active": true } ] } ] }
```

`bounds` is optional on input (older extension) and is in CSS pixels. Framing is unchanged: 4-byte little-endian length, then UTF-8 JSON, 4 MiB maximum.

### 3.5 Fixed names and paths

| Thing | Value |
|---|---|
| Restore lock (T4) | Windows mutex `Local\ContextFlow.Restore` |
| Data dir | `%APPDATA%\ContextFlow\ContextFlow\data\` (`contextflow.db`, `logs\`, `backups\`, `browser\live\`, `browser\extension\`, `browser\host.log`) |
| Native host name / extension ID | `dev.contextflow.host` / `dklnklfompfninkhpanjobnohejplfnj` |
| VS Code state (T9a) | `%APPDATA%\Code\User\globalStorage\state.vscdb`, table `ItemTable`, key `history.recentlyOpenedPathsList` (**to be verified on the machine in T9a before coding**) |
| New restore codes | `UNSAFE_PATH` (T5), `CANCELLED`, `OUT_OF_TIME` (T8), `FOLDER_UNKNOWN` (T9b) |

---

## 4. Step-by-step execution prompts

### 4.0 Before you start (human)

1. Commit the current working tree so every task starts from a clean base:
   `git add -A` then `git commit -m "Browser tabs, Store apps, Phase 1 hardening"`.
2. Run `powershell -File scripts\check.ps1`. It must print `All checks passed.` If not, stop.

**Shared preamble.** Paste this block before every task prompt:

```text
You are working in the Rust workspace at C:\Users\mmurp\ContextFlow on Windows 11 (PowerShell).
ContextFlow saves the user's open applications and windows into versioned SQLite snapshots and restores them.

Rules for every task:
1. Do only the task below. Do not refactor, rename or "improve" anything it does not name.
2. The gate is `powershell -File scripts\check.ps1` (cargo fmt check, clippy -D warnings, all tests). It must print "All checks passed." before you finish. Run `cargo fmt --all` first.
3. Never edit an existing file under crates\cf-storage\migrations\. Add new numbered files only.
4. Never change PAYLOAD_SCHEMA_VERSION or the fields of SnapshotPayload / ApplicationState / WindowState in crates\cf-core\src\model.rs unless the task says so.
5. Add no new dependencies unless the task lists them.
6. When you run the CLI by hand, always set a throwaway database first:
   $env:CONTEXTFLOW_DB="$env:TEMP\cf-task\cf.db"; $env:CONTEXTFLOW_CONFIG="$env:TEMP\cf-task\none.toml"
   Never run `capture`, `restore`, `shortcut` or `browser install` against the real database, and never close or kill applications you did not start.
7. Do not write window titles, URLs or note text to logs.
8. If something in the task does not match the code you find (a function is missing, a signature differs), STOP and report the mismatch. Do not improvise a different design.
9. When done: run the gate, then `git add -A` and `git commit -m "<the commit message given in the task>"`. Do not push. Finish with a short report: files changed, tests added, gate result.
```

---

### T1. Delete `cf-adapters`; move `RestoreStatus` into `cf-core`

```text
TASK T1: remove the unused cf-adapters crate.

Read first: crates\cf-adapters\src\lib.rs (the RestoreStatus enum and its as_str), crates\cf-core\src\model.rs, crates\cf-core\src\lib.rs.

Do:
1. Copy the `RestoreStatus` enum (variants Success, PartialSuccess, Failed, Skipped, RequiresUserIntervention; derives Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize; #[serde(rename_all = "snake_case")]) and its `impl RestoreStatus { pub fn as_str(self) -> &'static str }` to the end of crates\cf-core\src\model.rs, unchanged. It is re-exported automatically by `pub use model::*`.
2. Replace every `cf_adapters::RestoreStatus` with `cf_core::RestoreStatus` in:
   crates\cf-engine\src\lib.rs, crates\cf-engine\src\browser.rs, crates\cf-engine\src\tests.rs, crates\cf-cli\src\main.rs.
3. Remove the `cf-adapters.workspace = true` line from crates\cf-engine\Cargo.toml and crates\cf-cli\Cargo.toml.
4. Delete the folder crates\cf-adapters.
5. In the root Cargo.toml [workspace.dependencies], remove `cf-adapters`. Also remove `async-trait` and `tokio` if (and only if) `git grep -n "async-trait\|tokio" -- "*.toml"` shows no other crate uses them.
6. Fix the stale comment in crates\cf-storage\src\lib.rs only if it fails to compile; otherwise leave it.

Do NOT: change any behaviour, touch cf-storage code, or edit docs.

Done when:
- `git grep -n cf_adapters` and `git grep -n "cf-adapters"` return nothing outside docs\.
- `cargo tree -p cf-cli` does not contain `tokio`.
- The gate passes.
Commit message: "Remove unused cf-adapters crate; RestoreStatus lives in cf-core"
```

### T2. Split `cf-cli/src/main.rs` into command modules (no behaviour change)

```text
TASK T2: mechanical refactor of crates\cf-cli\src\main.rs (about 1,185 lines) into modules. Zero behaviour change.

Before changing anything, record the current help text:
  cargo build -p cf-cli
  $h="$env:TEMP\cf-help-before.txt"; Remove-Item $h -ErrorAction SilentlyContinue
  foreach ($c in @("","workspace","capture","restore","shortcut","snapshots","show","diff","delete-snapshot","prune","export","import","backup","verify","config","browser","info","windows")) { target\debug\contextflow.exe $c.Split(" ",[StringSplitOptions]::RemoveEmptyEntries) --help | Out-File -Append $h }

Read: crates\cf-cli\src\main.rs, crates\cf-cli\src\lib.rs, crates\cf-cli\src\session.rs.

Create crates\cf-cli\src\commands\ as a module of the BINARY (declare `mod commands;` in main.rs, not in lib.rs) with these files, moving the existing functions verbatim:
- commands\mod.rs        : `pub mod` lines for the files below.
- commands\util.rs       : require_yes, latest, fmt_versions, policy_from, normalize_app_names
- commands\workspace.rs  : workspace
- commands\capture_cmd.rs: capture_cmd
- commands\restore.rs    : restore_cmd, shortcut_cmd
- commands\history.rs    : snapshots, show, diff
- commands\maintenance.rs: delete_snapshot, prune, export, import, backup, verify, info
- commands\browser.rs    : browser_cmd
- commands\config.rs     : config_cmd, windows
Make moved functions `pub(crate)`. The clap types (Cli, Command, WorkspaceCmd, BrowserCmd, ConfigCmd) stay in main.rs and become `pub(crate)` so the modules can name them. main.rs keeps only: the clap types, `main`, and `run` (the dispatch match).

Do NOT: change any text printed to the user, any clap attribute, any function body, or any file outside crates\cf-cli\src\main.rs and the new commands\ folder.

Done when:
- main.rs is under 400 lines.
- Regenerating the help text the same way into cf-help-after.txt gives a file identical to cf-help-before.txt (`Compare-Object (Get-Content before) (Get-Content after)` prints nothing).
- The gate passes.
Commit message: "Split CLI commands into modules (no behaviour change)"
```

### T3. CLI integration tests

```text
TASK T3: add an integration test file for the `contextflow` binary. No production code changes.

Read: crates\cf-cli\src\main.rs (the clap definitions), crates\cf-cli\Cargo.toml.

Create crates\cf-cli\tests\cli.rs. Add `tempfile.workspace = true` under [dev-dependencies] in crates\cf-cli\Cargo.toml.

Helper: `fn cf(dir: &Path, args: &[&str]) -> std::process::Output` that runs `env!("CARGO_BIN_EXE_contextflow")` with env CONTEXTFLOW_DB=<dir>\cf.db and CONTEXTFLOW_CONFIG=<dir>\none.toml and returns the output. Each test creates its own `tempfile::tempdir()`.
IMPORTANT: every `capture` in these tests must pass `--no-windows`, so tests never read the real desktop and are deterministic. Never call `restore` without `--dry-run`, never call `shortcut` or `browser install`.

Tests (one #[test] each, names exactly):
1. workspace_create_list_and_duplicate: create "A" succeeds; create "a" fails (non-zero exit, stderr contains "already exists"); list shows "A".
2. capture_versions_and_identical_note: two `capture A --no-windows --note x` give "v1" then "v2"; the second output contains "identical to the previous version".
3. delete_snapshot_requires_yes_and_never_reuses_versions: capture twice; `delete-snapshot A --version 2` fails and says "--yes"; with --yes succeeds; next capture prints "v3".
4. prune_is_dry_run_without_yes: 4 captures with different notes; `prune A --keep-last 2` prints "would delete" and `snapshots A` still lists 4; with --yes lists 2.
5. export_import_round_trip_and_tamper: export A to a file; `import` fails ("already exists"); `import --as B` succeeds; edit the file replacing the note text and `import --as C` fails with "hash check"; `workspace list` does not contain "C".
6. verify_reports_ok: after captures, `verify` exits 0 and stdout contains "database: ok".
7. bad_config_reports_line: write a config file containing "[capture]\ninclude_windowz = true\n", point CONTEXTFLOW_CONFIG at it, run `info`; exit is non-zero and stderr contains "line 2".
8. restore_dry_run_on_empty_snapshot: capture --no-windows; `restore A --dry-run` exits 0 and stdout contains "nothing to restore".

Do NOT: modify any file under crates\cf-cli\src\.

Done when: `cargo test -p cf-cli --test cli` shows 8 passed, and the gate passes.
Commit message: "Add CLI integration tests"
```

### T4. Single-instance lock for restore

```text
TASK T4: make sure only one restore runs at a time (two quick double-clicks on the desktop icon currently start every app twice).

Read: crates\cf-windows\src\lib.rs, crates\cf-windows\src\control.rs, crates\cf-windows\Cargo.toml, crates\cf-cli\src\session.rs, crates\cf-cli\src\launch.rs, crates\cf-cli\src\commands\restore.rs.

Do:
1. crates\cf-windows\Cargo.toml: add "Win32_Security" to the windows-sys feature list.
2. Create crates\cf-windows\src\lock.rs (compiled only on Windows, `#[cfg(windows)] mod lock;` in lib.rs):
   - `pub struct InstanceLock(isize);` with `impl Drop` calling CloseHandle.
   - `pub fn try_lock(name: &str) -> std::io::Result<Option<InstanceLock>>`: call CreateMutexW(null, 0, wide(name)). If the handle is null return the last OS error. If GetLastError() == ERROR_ALREADY_EXISTS (183), close the handle and return Ok(None). Otherwise Ok(Some(lock)).
3. crates\cf-windows\src\lib.rs: `pub struct InstanceLock` re-export on Windows, and a portable `pub fn try_lock_instance(name: &str) -> Result<Option<InstanceLock>>` (on non-Windows define a zero-sized InstanceLock and always return Ok(Some(..))).
4. crates\cf-cli\src\session.rs:
   - add `pub const RESTORE_LOCK: &str = r"Local\ContextFlow.Restore";`
   - add `#[derive(Debug)] pub struct RestoreBusy;` implementing Display ("another ContextFlow restore is already running") and std::error::Error.
   - in `restore_workspace`, when `!args.dry_run`, acquire the lock right after loading the snapshot; if `None`, `return Err(RestoreBusy.into())`. Hold the guard until the function returns.
5. crates\cf-cli\src\launch.rs: in `main`, if the error downcasts to `RestoreBusy`, exit silently (no message box).
   The CLI needs no change: it prints the error like any other.

Tests:
- crates\cf-windows\src\lib.rs test (Windows only) `instance_lock_is_exclusive_until_dropped`: lock "Local\ContextFlow.Test.<process id>" → Some; second call → None; drop the first; third call → Some.

Do NOT: change cf-engine, change dry-run behaviour, or add a lock to capture.

Done when: the new test passes, the gate passes, and by hand (throwaway DB, workspace with one `--no-windows` capture) `contextflow restore X` still prints "nothing to restore".
Commit message: "Allow only one restore at a time"
```

### T5. Trust gate for imported workspaces, and safe executable paths

```text
TASK T5: close a code-execution hole. An imported workspace's executable paths and arguments are attacker-controlled and are launched by `restore`. Imported workspaces must be reviewed and approved first.

Read: crates\cf-storage\src\migrations.rs, crates\cf-storage\src\repo.rs, crates\cf-storage\src\export.rs (import_workspace), crates\cf-storage\tests\lifecycle.rs, crates\cf-cli\src\session.rs, crates\cf-cli\src\main.rs (WorkspaceCmd), crates\cf-cli\src\commands\workspace.rs, crates\cf-engine\src\lib.rs (plan_launch), crates\cf-cli\tests\cli.rs.

Do:
A. Storage
1. Create crates\cf-storage\migrations\0003_trust.sql containing exactly:
   ALTER TABLE contexts ADD COLUMN is_trusted INTEGER NOT NULL DEFAULT 1 CHECK (is_trusted IN (0, 1));
   Register it as the third entry in MIGRATIONS in migrations.rs.
2. In export.rs `import_workspace`, add `is_trusted` to the INSERT column list with value 0.
3. In repo.rs add to `impl Database`:
   pub fn is_trusted(&self, id: ContextId) -> Result<bool>      // NotFound if the workspace does not exist
   pub fn set_trusted(&self, id: ContextId, trusted: bool) -> Result<()>   // NotFound if 0 rows updated
   Do not add a field to cf_core::Context.
B. Engine (crates\cf-engine\src\lib.rs, function plan_launch, the ordinary-executable branch only, before the exe_exists check):
   if the path is not absolute (std::path::Path::is_absolute) or its extension is not "exe" (ASCII case-insensitive) return
   Err((RestoreStatus::Failed, "UNSAFE_PATH", "not started: only full paths to .exe files can be launched".into()))
C. CLI
1. session.rs `restore_workspace`: after finding the workspace, if `!args.dry_run && !db.is_trusted(ctx.id)?` bail with:
   "'{name}' was imported from a file and has not been approved. Review it with: contextflow workspace trust \"{name}\""
2. main.rs: add `Trust { name: String, #[arg(long)] yes: bool }` to WorkspaceCmd with doc comment "Review and approve an imported workspace so it can be restored."
3. commands\workspace.rs: implement it. Print, for the LATEST snapshot, one line per application: application id, executable path, and its launch arguments if any. Then: without --yes, print "Nothing approved. Re-run with --yes if you trust every program listed." and return an error (non-zero exit); with --yes call set_trusted(true) and print "Approved '{name}'." If the workspace is already trusted, print "'{name}' is already approved." and return Ok.
4. `workspace list`: append "  [needs approval]" for untrusted workspaces.

Tests:
- crates\cf-storage\tests\lifecycle.rs `imported_workspaces_start_untrusted`: created workspace → is_trusted true; export + import_workspace(.., Some("copy")) → false; set_trusted(true) → true.
- crates\cf-engine\src\tests.rs `relative_or_non_exe_paths_are_never_launched`: apps with executable_path "evil.exe" (relative) and r"C:\Apps\script.bat" → both steps Failed with code Some("UNSAFE_PATH"); `d.launched` is empty.
- crates\cf-cli\tests\cli.rs `imported_workspace_needs_approval`: export A, import --as B; `restore B` fails and stderr contains "has not been approved"; `restore B --dry-run` succeeds; `workspace trust B` fails; `workspace trust B --yes` succeeds; `restore B` then succeeds with "nothing to restore".

Do NOT: change export file format, change hashing, or gate `capture`, `show` or `diff`.

Done when: the three new tests pass and the gate passes.
Commit message: "Require approval before restoring imported workspaces; refuse unsafe launch paths"
```

### T6. Pinned restore version, and a warning for minimized windows

```text
TASK T6: (a) let the user pin which snapshot version restore uses, so a bad capture cannot silently change the desktop icon; (b) warn when a capture saves minimized windows and offer to save them as normal.

Read: crates\cf-storage\src\migrations.rs, crates\cf-storage\src\repo.rs (delete_snapshot, retention_candidates, prune, list_snapshots), crates\cf-cli\src\session.rs, crates\cf-cli\src\capture.rs, crates\cf-cli\src\main.rs, crates\cf-cli\src\commands\capture_cmd.rs, commands\workspace.rs, commands\history.rs.

Do:
A. Storage
1. Create crates\cf-storage\migrations\0004_pinned_version.sql containing exactly:
   ALTER TABLE contexts ADD COLUMN pinned_version INTEGER CHECK (pinned_version IS NULL OR pinned_version > 0);
   Register it as the fourth migration.
2. repo.rs, `impl Database`:
   pub fn pinned_version(&self, id: ContextId) -> Result<Option<u32>>
   pub fn set_pinned_version(&self, id: ContextId, version: Option<u32>) -> Result<()>
      // Some(v): NotFound("snapshot version {v} does not exist") if that snapshot is missing.
3. `delete_snapshot`: if `version` is the pinned version return StorageError::Conflict("v{n} is pinned; run `contextflow unpin` first").
4. `retention_candidates`: never return the pinned version (in addition to never returning the latest).
B. Restore
   session.rs `restore_workspace`: the version to restore is `args.version`, else the pinned version, else the latest.
C. CLI (main.rs + commands)
1. New top-level commands:
   Pin { workspace: String, #[arg(long)] version: Option<u32> }   // doc: "Make restore (and the desktop icon) always use one snapshot version (default: the latest)."
   Unpin { workspace: String }                                      // doc: "Go back to restoring the latest snapshot."
   Put their functions in commands\history.rs. Output: "'{name}' is pinned to v{n}." / "'{name}' restores the latest snapshot again."
2. `snapshots <ws>`: append "  (pinned)" to the pinned version's line.
3. `capture`: after the "Saved ..." line, if a pin exists and differs from the new version, print:
   "Note: restore and the desktop icon still use v{pinned} (pinned). To switch: contextflow pin \"{name}\""
D. Minimized windows (crates\cf-cli\src\capture.rs)
1. Add `pub unminimize: bool` to CaptureOptions and `pub minimized_windows: usize` to CaptureResult.
2. When building each WindowState: if the window's display_state is Minimized and `unminimize` is true, store DisplayState::Normal. Count windows that are stored as Minimized in `minimized_windows`.
3. main.rs Capture: add `#[arg(long)] unminimize: bool` with doc "Save minimized windows as normal windows." Pass it through. For the `diff --live` call site pass false.
4. capture_cmd: if `minimized_windows > 0` print to stderr:
   "  warning: {n} window(s) are minimized and will reopen minimized. Use --unminimize to save them as normal windows."

Tests:
- lifecycle.rs `pinned_version_is_protected`: 3 versions; pin 1; delete_snapshot(1) → Conflict; prune keep_last 1 leaves [3, 1]; set_pinned_version(Some(9)) → NotFound; set None; delete_snapshot(1) → Ok.
- cli.rs `pin_controls_which_version_restore_uses`: two `--no-windows` captures with notes "one" and "two"; `pin A --version 1`; `restore A --dry-run` stdout contains "v1" and "one"; `snapshots A` contains "(pinned)"; a third capture prints "still use v1"; `unpin A`; `restore A --dry-run` contains "v3".

Do NOT: change export/import (pins are not exported), or the shortcut format.

Done when: both new tests pass and the gate passes.
Commit message: "Pin the restored version; warn about and optionally unminimize captured windows"
```

### T7. Find apps whose versioned install folder changed

```text
TASK T7: apps such as Discord install to a folder named after their version (...\Discord\app-1.0.9267\Discord.exe). After the app updates, the saved path no longer exists and restore reports EXE_NOT_FOUND. Resolve the newest sibling folder instead.

Read: crates\cf-engine\src\lib.rs (LaunchPlan, plan_launch, and where LaunchPlan::Exe is launched), crates\cf-engine\src\desktop.rs, crates\cf-engine\src\browser.rs (uses app.executable_path in open_browser_window), crates\cf-engine\src\tests.rs (the Fake desktop).

Do:
1. Create crates\cf-engine\src\resolve.rs:
   /// `app-1.2.3` style folder name -> its numeric version parts.
   fn version_of(dir_name: &str) -> Option<Vec<u64>>     // requires prefix "app-" then digits separated by dots, nothing else
   /// If `exe_path` is `<base>\app-<ver>\<file>` return `<base>\<newest app-* folder that contains <file>>\<file>`.
   pub(crate) fn newest_versioned_sibling(exe_path: &str, list_dirs: &dyn Fn(&str) -> Vec<String>, exists: &dyn Fn(&str) -> bool) -> Option<String>
   Compare versions numerically, component by component (1.0.10 is newer than 1.0.9). Use backslash as the separator. Return None when the folder is not version-shaped or nothing qualifies.
2. desktop.rs: add to the Desktop trait `fn list_dirs(&self, dir: &str) -> Vec<String>;` (names of immediate sub-directories; empty on error). Implement for WindowsDesktop with std::fs::read_dir.
3. lib.rs: change `LaunchPlan::Exe` to `LaunchPlan::Exe(String)` carrying the path to launch.
   In plan_launch: if exe_exists(saved path) → Exe(saved path). Otherwise try newest_versioned_sibling with the desktop's list_dirs and exe_exists; if Some(p) → Exe(p). Otherwise the existing EXE_NOT_FOUND error. (If T5 is done, apply its absolute-path/.exe check to the saved path first, as it is today.)
   Use the carried path when launching, and pass it to `browser::restore_tabs` in place of `app.executable_path` (add a `exe: &str` parameter there and use it for `open_browser_window`).
4. tests.rs Fake: add `dirs: Vec<(String, Vec<String>)>` and implement list_dirs from it. Add `existing_paths: Option<HashSet<String>>`; when Some, `exe_exists` returns whether the exact path is in the set (keep current behaviour when None).

Tests:
- resolve.rs unit tests `picks_newest_numeric_version`, `ignores_non_version_folders`, `none_when_file_missing_everywhere`.
- tests.rs `updated_app_is_found_in_its_new_version_folder`: saved path r"C:\Users\u\AppData\Local\Discord\app-1.0.9\Discord.exe"; dirs: base → ["app-1.0.9" (gone), "app-1.0.10", "packages"]; only the 1.0.10 exe exists; restore launches and the step is Success; record the launched path in the Fake and assert it contains "app-1.0.10".

Do NOT: change how windows are matched to apps, or touch Store-app (Packaged) handling.

Done when: new tests pass and the gate passes.
Commit message: "Resolve version-stamped install folders when the saved executable has moved"
```

### T8. Bound total restore time, start ordinary apps first, support cancel

```text
TASK T8: restore currently does browser work inline, so a slow browser delays every app listed after it, and total time is unbounded.

Read all of: crates\cf-engine\src\lib.rs, crates\cf-engine\src\browser.rs, crates\cf-engine\src\tests.rs, crates\cf-engine\src\tests_browser.rs, crates\cf-config\src\lib.rs, crates\cf-cli\src\session.rs, crates\cf-cli\src\launch.rs, crates\cf-cli\src\commands\restore.rs.

Do:
A. cf-engine
1. RestoreOptions: add `pub overall_timeout: Duration` (default 120 s) and `pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>` (default None).
2. Add in lib.rs: `struct Budget { end: Instant, cancel: Option<Arc<AtomicBool>> }` with `fn cancelled(&self) -> bool`, `fn expired(&self) -> bool` (cancelled or past end), `fn remaining(&self) -> Duration`.
3. Ordering inside `restore`: in the per-application loop, a browser that is NOT running and has saved tabs is not started immediately. Push it onto a `deferred` list. After the loop has started every other application, process `deferred` in order (call restore_tabs as today), then run the existing wait loop.
4. Budget use:
   - Per-application loop: if `budget.cancelled()` push a step {status: Skipped, code: Some("CANCELLED"), message: "restore was cancelled"} for that app and every remaining one, start nothing more.
   - Wait loop: when `budget.expired()`, treat every pending app as timed out immediately (the existing timed_out path).
   - restore_tabs: take `&Budget`. Before opening each window, if expired push the failure "not opened: the restore ran out of time" and stop. The wait for a new window is `min(current wait, budget.remaining())`.
5. Deferred browsers that fail because the budget expired use code Some("OUT_OF_TIME") instead of "WINDOW_TIMEOUT" when no window appeared.
B. cf-config: add `restore_overall_secs: u64` to TimeoutConfig (default 120, valid 10..=1800, error text "timeouts.restore_overall_secs must be between 10 and 1800"). Add `restore_overall_secs = 120` under [timeouts] in TEMPLATE and one invalid case to the `invalid_values_are_rejected` test.
C. cf-cli: add `pub overall_timeout: Duration` to session::RestoreArgs, set from `cfg.timeouts.restore_overall_secs` in both launch.rs and commands\restore.rs, and pass into RestoreOptions.

Tests (tests.rs / tests_browser.rs). Add `events: RefCell<Vec<String>>` to the Fake; push "launch:<id>" in launch and "open:<n urls>" in open_browser_window.
- `ordinary_apps_start_before_browser_tabs`: payload order [chrome with tabs, code]; assert the index of "launch:code" is lower than the first "open:" event.
- `overall_timeout_bounds_the_whole_restore`: three apps whose windows never appear, per-app timeout 10 s, overall_timeout 200 ms; the call returns in under 2 s and every step is Failed.
- `cancel_before_start_launches_nothing`: cancel flag already true; no launches, every step has code Some("CANCELLED"), overall is Success (nothing failed).

Do NOT: make the engine async, add threads, or change messages of existing steps (existing tests assert on them).

Done when: new tests pass, all existing engine tests still pass unchanged, and the gate passes.
Commit message: "Bound total restore time, start ordinary apps before browser tabs, add cancel"
```

### T9a. `cf-vscode`: work out which folder each VS Code window has open (capture side)

```text
TASK T9a: save which folder each VS Code window has open, without a VS Code extension.

Approach: a VS Code window title ends with "<folder name> - Visual Studio Code" (default title format). VS Code keeps its recently opened folders, newest first, in a SQLite file. Take the folder name from the title and look up the newest recent folder with that base name.

STEP 0, verify the assumption before writing code (read-only):
  The file is "$env:APPDATA\Code\User\globalStorage\state.vscdb". It should contain a table ItemTable(key TEXT, value) with a row whose key is 'history.recentlyOpenedPathsList' and whose value is JSON like {"entries":[{"folderUri":"file:///c%3A/Users/..."},{"workspace":{"configPath":"..."}},{"fileUri":"..."}]}.
  Write a throwaway Rust test or use any SQLite tool to print ONLY the table names, whether that key exists, and the set of property names used inside `entries` items. Do not print or commit any paths from it.
  If the file, table, key or `folderUri` property is missing, STOP and report what you found. Do not continue.

Read: crates\cf-browser\src\saved.rs (the pattern to copy), crates\cf-cli\src\capture.rs (attach_browser_tabs), crates\cf-storage\Cargo.toml (rusqlite usage), root Cargo.toml.

Do:
1. New crate crates\cf-vscode (add `cf-vscode = { path = "crates/cf-vscode" }` to root [workspace.dependencies]). Dependencies: rusqlite, serde, serde_json (all `.workspace = true`). Dev: tempfile.
2. crates\cf-vscode\src\lib.rs:
   pub const ADAPTER_ID: &str = "vscode";
   pub const STATE_SCHEMA: u32 = 1;
   pub struct VsCodeWindow { pub window_title: String, pub root_name: Option<String>, pub folder_uri: Option<String> }
   pub struct VsCodeState { pub schema_version: u32, pub windows: Vec<VsCodeWindow> }   // Serialize, Deserialize, Debug, Clone, PartialEq, Eq
   impl VsCodeState { pub fn to_value(&self) -> serde_json::Value; pub fn from_value(v: &serde_json::Value) -> Option<Self> /* None unless schema_version == 1 */ }
   pub fn is_vscode(application_id: &str) -> bool          // "code" only
   pub fn root_name_from_title(title: &str) -> Option<String>
      // Strip the suffix " - Visual Studio Code" (return None if absent). Take the text after the last " - " of what remains, or all of it if there is no " - ". Trim. Strip a trailing " (Workspace)". None if empty.
   pub fn is_safe_folder_uri(uri: &str) -> bool
      // starts with "file:///", at most 2048 chars, no whitespace or control characters
   pub fn folder_base_name(uri: &str) -> Option<String>
      // last non-empty path segment of the URI, percent-decoded (%20, %3A etc.), compared case-insensitively by callers
   pub fn parse_recent_folders(json: &str) -> Vec<String>
      // the `folderUri` strings of `entries`, in order, keeping only is_safe_folder_uri ones
   pub fn read_recent_folders(state_db: &std::path::Path) -> Vec<String>
      // open READ-ONLY (OpenFlags::SQLITE_OPEN_READ_ONLY), query the key, parse; return empty on any error
   pub fn default_state_db() -> Option<std::path::PathBuf>     // %APPDATA%\Code\User\globalStorage\state.vscdb
   pub fn build_state(window_titles: &[String], recent_folders: &[String]) -> VsCodeState
      // one VsCodeWindow per title; folder_uri = first recent folder whose folder_base_name equals root_name (case-insensitive)
3. crates\cf-cli: add the dependency. In capture.rs add `pub vscode_state_db: Option<PathBuf>` to CaptureOptions and a function `attach_vscode_folders(payload, state_db, warnings, summary)` modelled on attach_browser_tabs: for the application with id "code", collect its window titles, call read_recent_folders + build_state, and if at least one window has a folder_uri set `adapter_id = Some("vscode")` and `adapter_state = state.to_value()`. For windows whose folder could not be resolved push the warning "VS Code: the folder for {n} window(s) could not be worked out; they will reopen without one". Call it next to attach_browser_tabs. Production call sites pass `cf_vscode::default_state_db()`; the unit test and `diff --live` pass None.
4. capture_cmd: for an application whose adapter_state parses as VsCodeState, append ", {n} folder(s) saved" to its summary line. `show`: print "Folders in code:" then one line per window with a folder_uri.

Tests in cf-vscode (use a temp SQLite file you create with the same table/key):
  root_names_from_titles ("main.rs - ContextFlow - Visual Studio Code" → "ContextFlow"; "ContextFlow - Visual Studio Code" → "ContextFlow"; "● a.rs - my proj (Workspace) - Visual Studio Code" → "my proj"; "Notepad" → None),
  safe_uris_only, base_names_are_percent_decoded, recent_folders_are_read_in_order, build_state_matches_newest_folder_case_insensitively, missing_database_gives_empty_list.

Do NOT: write to VS Code's files, read any other key, touch cf-engine, or change restore.

Done when: tests pass; by hand, with VS Code open on this repo and a throwaway DB, `contextflow capture T --only code` prints "1 folder(s) saved" and `contextflow show T` lists a file:/// URI ending in "ContextFlow"; the gate passes.
Commit message: "Save the folder open in each VS Code window"
```

### T9b. Reopen VS Code folders on restore

```text
TASK T9b: when restoring VS Code, reopen each saved folder in its own window at its saved position. Requires T9a.

Read all of: crates\cf-engine\src\browser.rs, crates\cf-engine\src\lib.rs, crates\cf-engine\src\desktop.rs, crates\cf-engine\src\tests.rs, crates\cf-engine\src\tests_browser.rs, crates\cf-vscode\src\lib.rs.

Do:
1. Extract, without changing behaviour, the reusable parts of browser.rs into a new crates\cf-engine\src\window_open.rs:
   pub(crate) fn place_saved(...)                      // moved as is
   pub(crate) fn wait_for_new_window(desktop: &dyn Desktop, app_id: &str, known: &HashSet<isize>, wait: Duration) -> Option<isize>   // the polling loop currently inside restore_tabs
   browser.rs must call these. All existing browser tests must pass unchanged.
2. desktop.rs: add to the trait `fn open_app_window(&self, exe: &str, args: &[String]) -> Result<(), String>;`. WindowsDesktop: cf_windows::launch_process(exe, args, None).
3. Create crates\cf-engine\src\vscode.rs with
   pub(crate) fn folder_state(app: &ApplicationState) -> Option<VsCodeState>     // adapter_id == "vscode", parses, has at least one folder_uri
   pub(crate) fn restore_folders(desktop, app, exe: &str, state, saved: &[&WindowState], monitors, current_titles: &[String], opts, budget, log) -> TabRestore   // reuse browser::TabRestore
   For each saved window with a folder_uri that passes cf_vscode::is_safe_folder_uri:
     - skip it if any title in `current_titles` has the same root name (cf_vscode::root_name_from_title, case-insensitive): that folder is already open and the generic step will reposition it;
     - call open_app_window(exe, ["--new-window", "--folder-uri", <uri>]);
     - wait_for_new_window (first window: opts.timeout; later ones: min(opts.timeout, 15 s); also bounded by the budget);
     - place it with the saved WindowState whose title equals window_title; record it in `claimed`.
   Windows without a folder_uri are left to the existing generic behaviour.
4. lib.rs: mirror the browser wiring exactly:
   - not running + has folders → defer like browsers (T8), run restore_folders with empty current_titles, report with browser::started_report but with the message wording "started; {n} folder(s) opened in {m} window(s), {p} arranged";
   - running + has folders → call restore_folders with the titles of its current windows before the generic step, exclude `claimed` windows from the generic step, merge with browser::merge_into wording "added {n} missing folder(s) in {m} new window(s)".
   - dry run: "would be started with {n} folder(s)" / "already running; any of its {n} saved folder(s) that aren't open would be added".
5. tests.rs Fake: implement open_app_window (record the args in `app_window_opens: RefCell<Vec<Vec<String>>>`, and make the next queued window from `browser_windows` appear, same as open_browser_window).

Tests in a new crates\cf-engine\src\tests_vscode.rs (include it from tests.rs the way tests_browser.rs is included):
  closed_vscode_reopens_each_folder_in_place, running_vscode_only_opens_folders_that_are_not_open, unsafe_folder_uris_are_never_passed_to_the_command_line (uri "--extensionDevelopmentPath=x" and "file:///c:/a b"), vscode_without_saved_folders_behaves_like_any_other_app.

Do NOT: change browser behaviour or messages, or add VS Code support for tabs/editors.

Done when: new and existing engine tests pass, the gate passes.
Commit message: "Reopen saved VS Code folders on restore"
```

### T10. Match browser windows by position when titles are equal

```text
TASK T10: two browser windows with the same title can currently swap tabs. Use each window's position as a tie-break.

Read: extensions\browser\background.js, extensions\browser\manifest.json, crates\cf-browser\src\protocol.rs, crates\cf-browser\src\saved.rs, crates\cf-browser\src\install.rs (tests), crates\cf-cli\src\capture.rs (attach_browser_tabs).

Do:
1. background.js `collect()`: add to each window object `bounds: { left: w.left ?? 0, top: w.top ?? 0, width: w.width ?? 0, height: w.height ?? 0 }`.
2. manifest.json: change "version" to "0.2.0". Change NOTHING else in it (the "key" and "permissions" must stay byte-identical; tests check them).
3. protocol.rs: add `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] pub struct Bounds { pub left: i32, pub top: i32, pub width: i32, pub height: i32 }`. Add `#[serde(default)] pub bounds: Option<Bounds>` to IncomingWindow and LiveWindow, copied through in `sanitize`. Old messages without bounds must still be accepted.
4. saved.rs: change match_windows to
   pub fn match_windows(browser: &str, desktop: &[DesktopWindow], live: &[LiveState], excluded: &[String]) -> MatchResult
   with `pub struct DesktopWindow { pub title: String, pub x: i32, pub y: i32, pub scale: f64 }` (scale = monitor DPI / 96, 1.0 if unknown).
   For each desktop window, among unused live windows whose title matches (existing rule), choose the one with the smallest |left*scale - x| + |top*scale - y|. A live window without bounds counts as distance i64::MAX (so behaviour for old extensions is "first match", as today).
5. capture.rs attach_browser_tabs: build DesktopWindow from each captured WindowState of that browser: x/y from rect, scale from the MonitorInfo in payload.monitors whose id equals the window's monitor_id (dpi/96.0), else 1.0.

Tests:
- protocol.rs: `bounds_are_optional_and_preserved`.
- saved.rs: update existing tests to the new signature, and add `identical_titles_are_matched_by_position` (two live windows titled "Same" at left 0 and left 1000 with different URLs; desktop windows given in the opposite order; each gets the tabs of the live window nearest to it) and `scale_is_applied_to_browser_coordinates` (live left 400, scale 1.5, desktop x 600 beats a live window at left 600).
- The existing install.rs tests (extension id, permissions) must pass untouched.

Do NOT: change the host framing, the state file names, or restore.

Done when: tests pass and the gate passes. Report to the user: "Run `contextflow browser install` again and click Reload on the ContextFlow card at chrome://extensions to get the new extension."
Commit message: "Match browser windows to their tabs by position when titles are equal"
```

### T11. Housekeeping and documentation

```text
TASK T11: stop two files/tables growing forever, and bring the docs up to date.

Read: crates\cf-cli\src\browser_host.rs, crates\cf-storage\src\history.rs, crates\cf-storage\tests\lifecycle.rs, README.md, docs\TESTING.md, docs\PLAN.md, docs\BLUEPRINT.md (sections 1 and 2).

Do:
1. browser_host.rs: before the message loop, if host.log exists and is larger than 1 MiB, truncate it to empty.
2. history.rs `record_restore_run`: inside the same transaction, after inserting, delete this snapshot's runs beyond its 50 newest:
   DELETE FROM restore_runs WHERE snapshot_id = ?1 AND id NOT IN (SELECT id FROM restore_runs WHERE snapshot_id = ?1 ORDER BY started_at DESC LIMIT 50)
   (restore_steps rows go with them through ON DELETE CASCADE).
3. Test in lifecycle.rs `restore_history_is_capped_per_snapshot`: record 55 runs with increasing started_at; `SELECT COUNT(*) FROM restore_runs` is 50 and the oldest remaining run is the 6th.
4. README.md: in the command table add rows for `workspace trust`, `pin` / `unpin`, `capture --unminimize`; add one short paragraph "Imported workspaces must be approved (`contextflow workspace trust`) before they can be restored."; mention that VS Code windows reopen their folder.
5. docs\TESTING.md: add manual checks, each with an "Expect" line, for: approving an imported workspace; pinning a version then capturing again; `--unminimize`; double-clicking the icon twice quickly (only one set of apps starts); VS Code reopening its folder.
6. docs\PLAN.md: append a section "15. Blueprint tasks T1 to T11 delivered" with a one-line summary per task. Do not rewrite earlier sections.

Do NOT: change behaviour other than items 1 and 2.

Done when: the new test passes and the gate passes.
Commit message: "Cap host log and restore history; document trust, pinning and VS Code folders"
```

---

## 5. After the tasks

Human steps the execution model must not do:

1. `cargo install --path crates/cf-cli --force` to update the installed programs.
2. `contextflow browser install`, then Reload the extension at `chrome://extensions` (needed after T10).
3. Arrange your apps, then `contextflow capture UniWork --only chrome,claude,code,discord,ms-teams --unminimize --note "..."`, check `contextflow show UniWork`, and `contextflow pin UniWork`.

Deferred, in suggested order: `cf-testwin` real-window tests (PLAN 2.4); process identity with start time (F12); stable monitor ids and mixed-DPI rescaling (F13); Chrome profile selection; publishing the extension; the Tauri UI (Phase 5).

Assumptions an execution model must verify, not trust: the VS Code state key and JSON shape (T9a step 0); that `CreateMutexW` needs the `Win32_Security` feature in windows-sys 0.59 (T4); the exact current signatures named in each "Read" list.
