# ContextFlow: Implementation Plan

**Status:** Phases 0 and 1 complete. The first slice of Phase 2 is done: `restore` and the desktop icon (see below). Remaining Phase 2: stable monitor ids, process identity, a real-window test app. See [TESTING.md](TESTING.md) to test what exists.
**Spec:** ContextFlow Comprehensive Project Specification v2.0 (save it as `docs/SPEC.md`).
**Audience:** whoever builds ContextFlow next. Every phase below lists concrete tasks, the files they touch, acceptance tests, and an exit gate.

---

## 1. How to read this plan

* **Phases** follow the spec's roadmap (§11). Each phase is split into **work packages** (WP) that are small enough to ship as one PR.
* Each WP has a **Done when** line made of automated tests or a documented manual scenario. A WP is not finished until that line holds.
* **Exit gates** are the conditions for starting the next phase. Do not start Phase N+1 work until the Phase N gate passes, except where a dependency note says otherwise.
* Estimates assume **one developer working part-time (~15 h/week)**. Halve them for full-time.

---

## 2. Non-negotiables

These come from the spec and override convenience every time:

1. **No destructive defaults.** Never force-terminate. Never checkout, reset or stash Git. Never overwrite newer project files.
2. **Snapshots are immutable.** This is enforced in two places: application logic (append-only API) and SQLite triggers (`UPDATE` aborts).
3. **No silent failure.** Every restore step reports one of `Success | PartialSuccess | Failed | Skipped | RequiresUserIntervention`.
4. **Fault isolation.** One component failing never blocks unrelated components.
5. **Local-first.** No network calls with workspace data. No cookies, passwords, tokens or private-browsing data, ever.
6. **No admin rights** for ordinary capture and restore.
7. **Bounded everything.** Every wait has a timeout, every retry has a limit, and every long operation can be cancelled.
8. **Reliability before breadth.** A new adapter only lands once the existing ones pass their reliability tests.

---

## 3. Architecture decisions (refining the spec)

| # | Decision | Rationale |
|---|----------|-----------|
| AD-1 | **Cargo workspace of small crates**: `cf-core` (pure model), `cf-storage`, `cf-adapters` (contract), `cf-git`, `cf-diff`, `cf-windows`, `cf-cli`, then `cf-engine` (Phase 2) and `app/` (Tauri, Phase 5). | Enforces the spec rule that the engine is independent of the UI. Each crate is testable on its own, and platform code is isolated behind `cfg(windows)`. |
| AD-2 | **`cf-core` has no I/O and no async.** | Keeps the domain logic deterministic and quick to test, including the hashing, validation and geometry code. |
| AD-3 | **Storage is synchronous `rusqlite`.** Async callers use one dedicated DB thread with a command channel (Phase 2, `cf-engine::db_actor`). | Follows spec §3.2. One writer thread also serialises version allocation. |
| AD-4 | **Canonical payload JSON is the source of truth.** It is hashed with SHA-256 and stored in `context_snapshots.payload_json`. The `snapshot_*` projection tables were dropped in migration `0006`: nothing read them. `0007` adds `context_snapshots.origin` (`manual` or `auto`). | Verification re-hashes the stored bytes, so it stays valid when later builds add optional fields. |
| AD-5 | **Schema evolution:** DB migrations are tracked in `PRAGMA user_version`. The payload has a `schema_version`, and each adapter's state has its own `schema_version`. An older build refuses to open a newer database. | Prevents silent data loss after a downgrade. |
| AD-6 | **Process identity = (PID, process creation time, image path).** Window handles live for one session only and are never stored. | Spec §4.1. PIDs and HWNDs get reused. |
| AD-7 | **Coordinates are physical pixels.** The process is per-monitor-DPI-aware v2 (`init_dpi_awareness`, plus an app manifest in Phase 5). | Without this, Windows virtualises coordinates on scaled displays and restore lands in the wrong place. |
| AD-8 | **Monitor identity:** v0 uses the GDI device name (`\\.\DISPLAY1`). Phase 2 switches to `QueryDisplayConfig` + `DisplayConfigGetDeviceInfo` (monitor device path / EDID manufacturer + product). The device name is kept as a fallback. | GDI names are not stable across reconnects or docking. |
| AD-9 | **Window placement uses `SetWindowPlacement`** (normal rect + show state), with `SetWindowPos` as a fallback. Placement is computed by the pure `cf_core::geometry::placement_for`. | `SetWindowPlacement` restores maximised windows correctly and keeps their normal-state rect. The geometry stays unit-testable. |
| AD-10 | **Browser integration uses Chrome/Edge Native Messaging.** No localhost WebSocket. | Native messaging is per-user, needs no open port, and is the browser-sanctioned channel. The host manifest is written to HKCU, so no admin is needed. |
| AD-11 | **The VS Code integration is an extension that talks over a per-user named pipe** (`\\.\pipe\contextflow-<user-SID>`, ACL limited to the current user). The CLI fallback is `code --folder-uri / --file-uri`. | Open editors and the active file are only reachable from inside VS Code. The spec forbids claiming more than the CLI can restore. |
| AD-12 | **One process at first.** The Tauri app hosts the engine and tray. The CLI opens the DB directly (WAL allows concurrent readers plus one writer). A separate daemon is deferred until it is needed (background capture, Phase 6). | Less IPC surface to secure and test in v1. |
| AD-13 | **External commands are spawned without a shell**, with argument vectors and `CREATE_NO_WINDOW`. Git always runs with `--no-optional-locks`. | Prevents injection, flashing console windows, and lock contention with the user's own Git usage. |
| AD-14 | **Async adapters use `async-trait`** for now. Moving to native `async fn` in traits needs `dyn` support, so revisit once that is stable for trait objects. | `AdapterRegistry` stores `Arc<dyn ContextAdapter>`. |

---

## 4. Repository layout

```text
ContextFlow/
├── Cargo.toml                 workspace manifest, shared dependency versions
├── crates/
│   ├── cf-core/               model, canonical hashing, validation, geometry   [done]
│   ├── cf-storage/            SQLite, migrations, immutable snapshots          [done]
│   │   └── migrations/0001_initial.sql
│   ├── cf-adapters/           ContextAdapter trait, capabilities, registry     [done]
│   ├── cf-git/                read-only git inspection (porcelain v2)          [done]
│   ├── cf-diff/               deterministic Context Diff                       [done: git/apps/windows/monitors/notes]
│   ├── cf-windows/            Win32 window/monitor discovery                   [discovery done; restore in Phase 2]
│   ├── cf-config/             config.toml loading and validation               [done]
│   ├── cf-cli/                `contextflow` binary                             [done for Phase 1]
│   ├── cf-engine/             (Phase 2) capture/restore orchestration, DB actor
│   ├── cf-adapter-vscode/     (Phase 3)
│   ├── cf-adapter-terminal/   (Phase 3)
│   ├── cf-adapter-browser/    (Phase 3) native-messaging host
│   └── cf-testwin/            (Phase 2) test application that opens windows with known titles
├── extensions/
│   ├── vscode/                (Phase 3) TypeScript VS Code extension
│   └── browser/               (Phase 3) Manifest V3 extension for Chrome/Edge
├── app/                       (Phase 5) Tauri 2 + React + TypeScript
└── docs/
    ├── PLAN.md                this file
    └── SPEC.md                the product specification
```

---

## 5. Phase-by-phase plan

### Phase 0: Scaffold ✅ (done)

* Cargo workspace, shared dependencies, release profile.
* `cf-core`: IDs, `Context`, `Snapshot`, `SnapshotPayload`, canonical JSON + SHA-256, validation, monitor-fallback geometry.
* `cf-storage`: WAL + foreign keys, migration runner, full spec §6 schema, immutability triggers, IMMEDIATE-transaction version allocation, integrity verification.
* `cf-git`: porcelain-v2 parser and a real-repo integration test.
* `cf-diff`: Git, application, window, monitor and notes comparison, with notices.
* `cf-windows`: `EnumWindows` discovery (visible, unowned, non-cloaked), process image path, display state, `EnumDisplayMonitors` with DPI.
* `cf-adapters`: adapter contract, capabilities, registry.
* `cf-cli`: `workspace create|list|archive|unarchive`, `capture`, `snapshots`, `show`, `diff --from/--to/--live`, `verify`, `windows`.

### Phase 1: Core engine and persistence ✅ (done)

| WP | Task | Status |
|----|------|--------|
| 1.1 | **CI.** [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) on `windows-latest`: fmt, clippy `-D warnings`, tests, and the release CLI uploaded as a build artifact. [`scripts/check.ps1`](../scripts/check.ps1) runs the same checks locally. | ✅ Passes locally. To block a failing PR, mark the CI job as a required check under the GitHub repo's branch-protection settings. |
| 1.2 | **Configuration.** New crate `cf-config`. `config.toml` sections: storage, capture exclusions (processes, title patterns), retention, git timeout, logging. Unknown keys and out-of-range values are rejected with file and line. CLI: `config path/show/init`. | ✅ 7 tests |
| 1.3 | **Retention and deletion.** `delete-snapshot`, `workspace delete`, and `prune` (dry-run unless `--yes`; optional `auto_prune`). A snapshot is pruned only if it breaks every configured rule, and the latest is never pruned. Migration `0002` makes sure version numbers are never reused. | ✅ |
| 1.4 | **Export, import and backup.** The export carries the canonical payload bytes and hashes. Import verifies everything before writing anything and keeps version numbers. Backup uses `VACUUM INTO` (consistent and compact), then reopens and integrity-checks the copy. | ✅ |
| 1.5 | **Crash safety.** [`tests/crash.rs`](../crates/cf-storage/tests/crash.rs) kills a writer process mid-write, 25 times by default. Set `CF_CRASH_ITERATIONS=100` for the full run. | ✅ |
| 1.6 | **Logging.** Daily rolling file next to the database (`tracing-appender`), level from config. `cf_core::redact` strips URL credentials, query strings and fragments, and replaces the home directory with `~`. Window titles and notes are never logged. | ✅ |

**Exit gate:** ✅ All of spec Phase 1's acceptance criteria are covered by automated tests. For manual checks, see [TESTING.md](TESTING.md).

### Phase 2: Windows discovery and layout restoration (≈ 4–5 weeks)

New crate **`cf-engine`**: the restoration pipeline and a DB actor.

| WP | Task | Notes and done when |
|----|------|------------------|
| 2.1 | **Process identity.** Add `GetProcessTimes` (creation time) and the parent PID (`CreateToolhelp32Snapshot`) to `DiscoveredWindow`. Introduce a `ProcessIdentity` type. | Unit test: the same PID with a different start time is treated as a different process. |
| 2.2 | **Stable monitor IDs** (AD-8) via `QueryDisplayConfig`. Map each target to its GDI device name to keep `MonitorFromWindow` working. | Manual check on a docked/undocked laptop: IDs stay the same across reconnects. |
| 2.3 | **Window placement capture.** Switch to `GetWindowPlacement` for the normal rect and show state. Also store the DWM extended frame bounds to diagnose invisible-border offsets. | Restoring a maximised window brings back its pre-maximise size when un-maximised. |
| 2.4 | **`cf-testwin`.** A tiny Win32 binary that opens N windows with given titles and rects (CLI args) and prints its PID and HWNDs as JSON. | Used by every Phase 2 integration test. No real apps are needed in CI. |
| 2.5 | **Launcher.** Resolve the executable, validate the path exists and is a file, reject args containing NUL, and spawn without a shell. Record PID and creation time. Optionally use a launch policy (Start-menu AppID for packaged/UWP apps via `IApplicationActivationManager`). | Test: a missing executable gives `Failed` with code `EXE_NOT_FOUND`, and the other components continue. |
| 2.6 | **Window identification.** After launch, poll with exponential backoff (50 ms → 1 s, total timeout per adapter, default 15 s) for top-level windows. Score candidates: PID match (or child of the launched PID, for launcher-stub apps like VS Code and Chrome) > exe path > window class > title similarity. Never block forever. | Tests with `cf-testwin` include a delayed window (3 s) and no window at all (timeout gives `Failed: WINDOW_TIMEOUT`). |
| 2.7 | **Spatial restore.** `placement_for` → `SetWindowPlacement`. Handle per-monitor DPI: if the target monitor's DPI differs from the saved one, scale the size by the DPI ratio before fitting. Windows from elevated processes cannot be moved (UIPI), so report `RequiresUserIntervention`. | Spec E2E Scenario 1 and Scenario 3 pass (§7). |
| 2.8 | **Restore pipeline and reporting.** Stages: Preflight → Launch → Identify → Place → Adapter state → Verify. Each component runs as an independent Tokio task under a `JoinSet` with its own timeout. A `CancellationToken` stops everything cleanly. Every step is written to `restore_runs` / `restore_steps`. | Spec Scenario 2 passes. Cancelling mid-restore leaves no orphaned "running" rows. |
| 2.9 | **Idempotency.** Before launching, check whether a matching window already exists (same identity rules). If it does, reuse it and only reposition. | Running restore twice does not open duplicate windows. |
| 2.10 | **CLI.** `contextflow restore <ws> [--version N] [--dry-run] [--only app1,app2]`. Dry-run prints the preflight report. | Manual run with Notepad, File Explorer and Windows Terminal. |

**Exit gate:** spec Phase 2 acceptance (capture multiple windows, save geometry, close/reopen, restore layout, survive a missing monitor) demonstrated by automated `cf-testwin` tests plus one documented manual run on real multi-monitor hardware.

### Phase 3: Developer integrations (≈ 6–8 weeks)

| WP | Task | Notes and done when |
|----|------|------------------|
| 3.1 | **Git adapter** wrapping `cf-git`, with capabilities `capture + validate` only (no restore: it never mutates). Validation reports a missing repo or a changed branch as `Warning`. Auto-discover repos from the working directories of captured terminals and VS Code folders. | Spec Scenario 4. |
| 3.2 | **Progress notes.** A note per snapshot is already supported. Add a "resume card": on restore, the last note plus a Git summary is shown first (CLI prints it; the UI shows it in Phase 5). | Note shown before any app launches. |
| 3.3 | **VS Code adapter, CLI level.** Detect `Code.exe` windows. Parse the workspace from the window title only as a hint. Restore with `code --new-window --folder-uri <uri>` or the `.code-workspace` file. Capabilities are honest: *folder/workspace only*. | Restores the right folder(s) in a new window. Does not claim to restore tabs. |
| 3.4 | **VS Code extension** (`extensions/vscode`, TypeScript). On activation it connects to the named pipe and reports workspace folders, open text editors (URI, view column, selection), the active editor, and the dirty state. On restore request it opens editors in order. A protocol schema is versioned in `extensions/protocol/vscode.schema.json`. | Open 5 files across 2 columns, capture, close VS Code, restore: the same 5 files come back in the same columns. Dirty editors are reported as unsaved. |
| 3.5 | **Windows Terminal adapter.** Capture the profile and starting directory. Shell process state cannot be observed reliably, so take directories from the user's saved per-workspace terminal config and from the repos in the snapshot. Restore with `wt.exe -w new new-tab -p <profile> -d <dir> ; new-tab ...`. Running commands are opt-in: user-configured per workspace, never inferred. | Restores N tabs in the right directories. Docs state clearly that shell history and processes are not restored. |
| 3.6 | **Browser extension** (`extensions/browser`, MV3, Chrome and Edge). Uses `chrome.tabs` / `chrome.windows` / `chrome.tabGroups`. Skips incognito windows always. Sends windows, tabs (URL, title, index, pinned, active) and groups over Native Messaging. A URL filter drops `chrome://`, `edge://`, `file://` (configurable) and any URL matching user exclusion rules. Credentials, cookies and form data are never read: the extension requests no `cookies` permission. | Capture 2 windows × 5 tabs with a group, then restore. Order, pinned state and active tab all match. An incognito window is not captured (test). |
| 3.7 | **Native messaging host** (`cf-adapter-browser`). A small Rust binary registered under HKCU for Chrome and Edge. It forwards to the engine over the same named pipe. Message size limits and schema validation apply on both sides. | Extension disconnect gives `Failed: BROWSER_DISCONNECTED`, and the other components still restore. |
| 3.8 | **Unsupported state reporting.** Each adapter's capabilities feed the restore preview. Example: "VS Code: folders and editors ✔, terminals ✖, unsaved buffers ✖". | Snapshot test of the preview text for each adapter. |

**Dependency:** 3.4 and 3.6 share the IPC server (named pipe + JSON-RPC-style framing). Build it once in `cf-engine::ipc` before both.

**Exit gate:** spec Phase 3 acceptance: VS Code workspace restored, Git divergence identified, browser tabs restored, notes displayed, unsupported state reported accurately.

### Phase 4: Context intelligence (≈ 3 weeks)

| WP | Task | Done when |
|----|------|-----------|
| 4.1 | **File metadata capture** for configured project directories only: path, size, mtime. Optional content hash (BLAKE3 or SHA-256), off by default. Respect `.gitignore` (use the `ignore` crate) and the configured exclusions. Hard limits: max files, max bytes hashed, and a time budget, so a capture reports `Partial` instead of hanging. | 50k-file repo captured under the time budget. Exclusions verified. |
| 4.2 | **File diff.** Added, removed, modified (mtime/size, or hash when available), directory changes. Stored in `snapshot_files`. | Deterministic report tests. |
| 4.3 | **Browser diff.** Tabs added, removed and changed, and the active tab changed (needs 3.6). Match by URL first, then by position. | Unit tests mirroring `diff_windows`. |
| 4.4 | **Application availability diff.** The executable still exists, the version changed (read `VS_FIXEDFILEINFO`), the saved adapter schema is unsupported. | Missing-app scenario reported in preflight. |
| 4.5 | **Restore recommendations.** Rule-based and deterministic. Each notice has a code, a severity and a suggested safe action ("Repository is on `feature/db`; snapshot was `main`. ContextFlow will not switch branches."). Already started in `cf_diff::notices`. Formalise it into typed `Recommendation`s. | Every recommendation code has a test. |
| 4.6 | **`verify --all`** across every workspace, plus a scheduled integrity check in the app. | Corrupted snapshot detected and reported, never "repaired" silently. |

**Exit gate:** spec Phase 4 acceptance: compare two versions, new/removed files, branch changes, missing apps, deterministic report.

### Phase 5: Desktop application ✅ (done, apart from certificate signing)

`app/` is Tauri 2 with React, TypeScript and Vite. The Rust side depends on the `cf-*` crates; Tauri commands are thin wrappers. TypeScript types are generated from the Rust data types with `ts-rs` (not `specta`, whose Tauri 2 support is still a release candidate), and CI fails if they are stale.

| WP | Task | Status |
|----|------|--------|
| 5.1 | **Tauri shell.** Single-instance plugin, tray icon (Open / Capture current / Restore recent), autostart opt-in. | ✅ Tray menu with a live "Restore recent" list. Start with Windows (opt-in in Settings) starts hidden in the tray. Closing to the tray is a setting. The process is made per-monitor-DPI aware at start-up. |
| 5.2 | **Typed IPC.** Generated TS types, progress events, cancel. | ✅ `ts-rs` types; `restore://progress` events; Cancel stops a restore before the next app starts. Long work runs on the blocking pool, never the UI thread. |
| 5.3 | **Dashboard.** Recent workspaces, last snapshot, status, quick restore. | ✅ Sidebar of workspaces with last version, age and an "active" badge; workspace page with Restore. |
| 5.4 | **Capture review.** Checklist of apps, windows, repos, tabs, a note, then save. | ✅ Tray "Capture current…" opens the review for the active workspace: two interactions to a saved snapshot. Beyond the plan: a global **quick-capture shortcut** (default Ctrl+Alt+X, configurable) and the tray's "Capture now" save the active workspace in the background with one key press and an on-screen confirmation card. |
| 5.5 | **Workspace detail and history.** Timeline, notes, select-two-to-compare. | ✅ Tick two versions to compare, or "Compare with now". Pin, delete, export. |
| 5.6 | **Restore view.** Preflight, live status, final report with retry. | ✅ Preview restore is the preflight and has a Start restore button; live log while running; Retry restores only what failed. |
| 5.7 | **Diff view.** Grouped by category, recommendations on top. | ✅ Applications, windows, monitors, repositories and notes; advice from `cf-diff` first. |
| 5.8 | **Settings.** Exclusions, retention, timeouts, adapters, extension helpers. | ✅ Round-trips into `config.toml` (tested). Also: backup, integrity check, export/import and retention review, so nothing needs the CLI. |
| 5.9 | **Notifications.** Restore finished, partial or failed. | ✅ Windows notification (a setting). |
| 5.10 | **Packaging.** Per-user installer, signing, native-messaging host under HKCU. | ✅ NSIS installer, no admin, registers the browser host. `scripts/build-installer.ps1` signs when `CONTEXTFLOW_SIGN_THUMBPRINT` is set; CI builds and uploads the installer. ⏳ A certificate is needed to sign it. ⏳ A clean-VM install/uninstall run is still to do. |

**Also delivered in Phase 5:** workspace management (rename, archive, delete with its desktop icon, pin, approve imports), custom desktop icons, and Workspace Cleanup (README).

**Exit gate:** everything is possible without the CLI, failures are clear, the UI stays responsive, and no admin is needed. Met, except that signing needs a certificate and the clean-VM run has not been done.

### Phase 6: Optimisation and advanced capabilities (ongoing)

Start only after the v1.0 definition of success (§8) is met. Candidates, in suggested order:

1. Lifecycle modes from spec §5: **Hide** (minimise all workspace windows), **Preserve** (capture and keep running), **Close safely** (`WM_CLOSE` only, then wait. If the adapter reports unsaved work or the window stays open, report it and never kill).
   *Partly delivered early as Workspace Cleanup (README): per-workspace close-on-enter and close-on-leave lists, preview with memory and CPU, graceful close and an opt-in force close per program. Still open from this item: Hide and Preserve, and closing a workspace's own windows when leaving it.*
2. Automated background capture (on workspace switch, on idle, before shutdown via `WM_QUERYENDSESSION`).
3. Workspace import/export sharing format.
4. Adapter SDK: an out-of-process adapter protocol so third parties can ship adapters without recompiling.
5. More adapters: JetBrains, Visual Studio, Docker Desktop, Firefox.
6. Resource reporting (CPU and memory per workspace). Priority adjustment as an opt-in. Process suspension is experimental and needs an application allowlist.
7. Optional AI summaries of a diff and notes. Local model, or explicit opt-in with a clear data notice.

---

## 6. Data model notes

* Schema v1 is in [`crates/cf-storage/migrations/0001_initial.sql`](../crates/cf-storage/migrations/0001_initial.sql). It covers every spec §6.1 table plus immutability triggers.
* Additions to the spec schema: `context_snapshots.payload_json` (canonical source of truth), and `snapshot_windows.title` / `window_class` (needed for identification).
* `0002` (shipped): `contexts.last_version` counter, so deleting the newest snapshot never lets its version number be reused. Also adds an index for age-based retention.
* Planned migrations:
  * `0003`: `restore_steps.error_code` index, plus `context_snapshots.label` (user-named versions such as "Database implementation").
  * `0004`: browser tab projection table (`snapshot_browser_tabs`) for Phase 4 diffing.
* Every adapter's `adapter_state` has a `schema_version`. Adapters must read N−1 versions.

---

## 7. Testing strategy

| Level | Where | Runs in CI |
|-------|-------|-----------|
| Unit | `#[cfg(test)]` in each crate: hashing, validation, geometry, porcelain parsing, diff | Yes |
| Storage integration | `crates/cf-storage/tests/`: in-memory and temp-file DBs, trigger enforcement, tamper detection, schema refusal | Yes |
| Git integration | Temp repos built with the real `git` | Yes (skips if git is missing) |
| Win32 integration | `cf-windows` tests, plus Phase 2 `cf-testwin`-driven tests | Yes on `windows-latest` (it has an interactive desktop session) |
| Adapter contract | A shared test suite every adapter must pass (capabilities honoured, timeouts respected, no panics on garbage state) | Yes |
| End-to-end | Spec §10.3 scenarios 1–5 as `#[ignore]` tests run on a real desktop, plus a documented manual checklist per release | Nightly / pre-release |
| Fault injection | Spec §10.4: missing exe, invalid path, DB interruption (kill during write), extension disconnect, window timeout, monitor removal (simulated by feeding a reduced monitor list to `placement_for`) | Yes, except physical monitor removal |
| Performance | `criterion` benches for snapshot creation and diffing. A capture-time benchmark on documented hardware. Results go in `docs/BENCHMARKS.md`. | Benches on demand. Regression check before release |

Spec §9 targets map to benchmarks: idle CPU < 1 %, memory < 100 MB, metadata capture < 3 s, DB snapshot write < 500 ms, layout restore < 5 s after apps are ready.

---

## 8. Definition of done for v1.0

Spec §14 mapped to the work packages:

| # | Capability | Delivered by |
|---|-----------|--------------|
| 1 | Create a named workspace | Phase 0 ✅ |
| 2 | Capture supported applications | Phase 0 (discovery) ✅ + 2.1, 3.x |
| 3 | Preserve window arrangements | Phase 0 (capture) ✅ + 2.3–2.7 (restore) |
| 4 | Capture project and Git metadata | Phase 0 ✅ + 3.1, 4.1 |
| 5 | Progress notes | Phase 0 ✅ + 3.2 |
| 6 | Multiple historical snapshots | Phase 0 ✅ |
| 7 | Close and reopen the environment | 2.8, 6.1 (close safely) |
| 8 | Restore supported application state | 3.3–3.7 |
| 9 | Identify missing apps and failed components | 2.5, 2.8, 4.4 |
| 10 | Compare previous and current state | Phase 0 (`diff --live`) ✅ + 4.x |
| 11 | Restore without destroying newer data | Non-negotiables 1–2, enforced by tests in 3.1 |
| 12 | Repeatable | 2.9 idempotency + E2E suite |

---

## 9. Risk register (additions to spec §13)

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Launcher-stub apps (VS Code, Chrome, Teams) exit immediately and their window belongs to another or child process | High | Wrong or no window matched | Identification by exe path and process tree, not only the launched PID (WP 2.6) |
| UIPI: windows of elevated apps cannot be moved | Medium | Partial restore | Detect and report `RequiresUserIntervention`. Never self-elevate |
| Packaged (MSIX/UWP) apps have no normal exe launch path | Medium | Launch failure | `IApplicationActivationManager` with the AUMID (WP 2.5) |
| Virtual desktops: cloaked windows on other desktops | Medium | Missed windows | v1 captures the current desktop only and documents it. Evaluate `IVirtualDesktopManager` later |
| Browser extension store review delays | Medium | Phase 3 slip | Sideload or developer-mode for early users. Submit early |
| VS Code API changes | Low | Adapter breakage | Pin `engines.vscode`. Contract tests on VS Code Insiders in CI |
| Sensitive data in window titles and URLs | Medium | Privacy | Exclusion rules (process, title regex, URL pattern), redaction in logs, everything local, delete/export controls |
| Snapshot bloat from file metadata | Medium | Slow DB | Configured directories only, limits, retention (1.3, 4.1) |

---

## 10. Open decisions for the project owner

1. **Licence.** The manifests say `MIT OR Apache-2.0` as a placeholder. Confirm, or pick commercial/closed.
2. **Product name and bundle ID.** `dev.ContextFlow.ContextFlow` is currently used for data directories. Change it before the first public build, because it determines where user data lives.
3. **Browser scope for v1.** Chrome + Edge (same extension) is recommended. Firefox is deferred.
4. **Telemetry.** None is recommended. If crash reports are ever wanted, make them strictly opt-in and send them through a separate, reviewable channel.
5. **Minimum OS.** Windows 10 21H2+ is recommended (per-monitor v2 DPI is fully supported there).

---

## 11. Suggested timeline (part-time, ~15 h/week)

| Weeks | Milestone |
|-------|-----------|
| 1–2 | Phase 1 hardening (CI, config, retention, export/backup) |
| 3–7 | Phase 2: `cf-engine`, layout restoration, `contextflow restore` |
| 8–15 | Phase 3: IPC, Git/VS Code/Terminal adapters, browser extension + native host |
| 16–18 | Phase 4: file and browser diff, recommendations |
| 19–26 | Phase 5: Tauri app, installer |
| 27+ | v1.0 release candidate, then Phase 6 |

A usable **developer preview** (CLI capture, restore and diff for windows, VS Code folders and Git) is realistic at the end of week 7.

---

## 12. Immediate next steps

1. Save the specification as `docs/SPEC.md` and make an initial commit.
2. WP 1.1: add CI.
3. WP 2.4: build `cf-testwin` first. Every Phase 2 test depends on it.
4. WP 2.5–2.8: the restore pipeline, leading to `contextflow restore`.

---

## 13. Update: apps-first restore and the desktop icon

Decision: the main way to save is **applications and windows**, not Git repositories. Repositories remain an optional extra (`capture --repo`).

Delivered ahead of the rest of Phase 2:

| Piece | Where | Covers plan item |
|---|---|---|
| `cf-windows`: `launch_process`, `place_window` (`SetWindowPlacement`), `message_box`; maximized/minimized windows now store their restored rectangle | [crates/cf-windows](../crates/cf-windows/src) | 2.3, 2.5, 2.7 |
| `cf-engine`: preflight, launch, bounded wait with settle, window pairing, monitor fallback, per-app status, idempotency. The OS sits behind a `Desktop` trait so it is fully unit-tested | [crates/cf-engine](../crates/cf-engine/src) | 2.5–2.9 |
| `contextflow restore` with `--only`, `--dry-run`, `--version` | [main.rs](../crates/cf-cli/src/main.rs) | 2.10 |
| Restore history in `restore_runs` / `restore_steps` | [history.rs](../crates/cf-storage/src/history.rs) | 2.8 |
| `contextflow-launch.exe` (no console window) and `contextflow shortcut` | [launch.rs](../crates/cf-cli/src/launch.rs), [shortcut.rs](../crates/cf-cli/src/shortcut.rs) | new |
| `capture --only` and built-in skipping of Store-app host windows | [capture.rs](../crates/cf-cli/src/capture.rs) | new |

Known gaps, in priority order:

1. **App contents are not restored.** Windows come back, but not which folder VS Code had open or which tabs a browser had. Launch arguments are not captured yet. This is the Phase 3 adapters (3.3 to 3.7), and is the next most valuable step.
2. **Microsoft Store apps** (new Notepad, Calculator, Windows Terminal) are skipped. They need launch by app id (`IApplicationActivationManager`), WP 2.5.
3. **Several windows of one app:** only as many windows come back as the app opens by itself.
4. Still open from Phase 2: stable monitor ids (2.2), process identity with start time (2.1), the `cf-testwin` test app (2.4), and DPI scaling between monitors of different scale (2.7).
5. The icon has no custom image yet.

---

## 14. Update: browser tabs (Phase 3, WP 3.6 and 3.7)

Chrome and Edge windows now come back with their tabs.

```text
Capture:  extension ──▶ contextflow-browser-host ──▶ live state file ──▶ capture ──▶ snapshot
Restore:  snapshot ──▶ chrome.exe --new-window <urls> ──▶ new window found and placed
```

Differences from the original plan: no named pipe. The host just keeps one validated JSON file per browser profile while the browser runs, and capture reads it. Restore needs no extension at all.

| Piece | Where |
|---|---|
| MV3 extension (`tabs`, `nativeMessaging`, `storage`, `alarms` only; no cookies, history, page access or incognito) | [extensions/browser](../extensions/browser) |
| Message validation (http/https only, size and count limits), live state files, tab-to-window matching, install and registry | [crates/cf-browser](../crates/cf-browser/src) |
| `contextflow-browser-host.exe` and the `browser install/status/uninstall` commands | [crates/cf-cli](../crates/cf-cli/src) |
| Per-window restore: open, find the new window, place it. A running browser only gets missing tabs | [crates/cf-engine/src/browser.rs](../crates/cf-engine/src/browser.rs) |
| Config: `[browser] capture_tabs`, `excluded_url_patterns`, `max_tabs_per_window` | [cf-config](../crates/cf-config/src/lib.rs) |

Known limits:

1. Chrome can't be told which tab to bring to the front, nor pin tabs or recreate tab groups, from the command line.
2. Tabs open in the browser profile last used, not necessarily the one they came from (no `--profile-directory` yet).
3. If Chrome is set to "Continue where you left off", it may add its own old windows on startup.
4. Two windows with identical titles can have their tabs matched to each other's position.
5. If Chrome is already running and the extension isn't connected, tabs are left alone (reported) so nothing duplicates.
6. Context Diff doesn't compare tabs yet (Phase 4, WP 4.3).
7. The extension is loaded as unpacked ("Developer mode"). Publishing to the Chrome Web Store and Edge Add-ons would remove that step.

---

## 15. Blueprint tasks T1 to T11 delivered

The audit and task prompts are in [BLUEPRINT.md](BLUEPRINT.md). All eleven tasks are done and passed the gate (format, clippy `-D warnings`, tests: 140 before T11).

| Task | Delivered |
|---|---|
| T1 | Deleted the unused `cf-adapters` crate; `RestoreStatus` moved into `cf-core`. |
| T2 | Split the 1,185-line `cf-cli/src/main.rs` into `commands/*`; help text byte-identical. |
| T3 | Added 8 CLI integration tests (`cf-cli/tests/cli.rs`). |
| T4 | Only one real restore at a time, through a named Windows lock; a second double-click exits silently. |
| T5 | Imported workspaces start untrusted and need `workspace trust`; the engine refuses relative or non-`.exe` launch paths (`UNSAFE_PATH`). The review lists programs from every version, not just the latest (a deliberate change to the blueprint). |
| T6 | `pin` / `unpin` a restore version (protected from delete and prune); `capture --unminimize` and a warning for minimized windows. |
| T7 | Apps in `app-<version>` folders (Discord and similar) are found again after an update. |
| T8 | An overall restore time limit (`timeouts.restore_overall_secs`), ordinary apps start before browser tabs, and the engine honours a cancel flag. |
| T9a, T9b | VS Code folders are saved and reopened. T9a used VS Code's `storage.json` instead of the blueprint's `state.vscdb` key, which does not exist on current VS Code. |
| T10 | Browser windows with identical titles are matched to their tabs by position (extension 0.2.0 sends window bounds). |
| T11 | `host.log` is emptied above 1 MiB and each snapshot keeps its newest 50 restore runs. Documentation updated. |

Known gaps that remain (see BLUEPRINT.md section 5 for the order):

1. Cancel has no trigger yet: the engine honours the flag, but the CLI and the icon don't set it.
2. Capture duration is part of the hashed payload, so "identical to the previous version" almost never appears with real windows.
3. The launcher's log file loses errors reported after its logger shuts down.
4. VS Code folder lookup depends on VS Code's internal `storage.json` layout, and a folder name shared by two folders is saved as unresolved.
5. Still open from the earlier plan: stable monitor ids and mixed-DPI rescaling, process identity (apps sharing an exe name), a real-window integration test app, and the Tauri UI.
