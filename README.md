# ContextFlow

**Capture your environment. Preserve your progress. Resume exactly where you left off.**

ContextFlow is a local-first workspace continuity engine for Windows. It captures windows, monitors, Git state and progress notes into immutable, versioned snapshots. It can compare any snapshot with another one or with the live environment, and (from Phase 2) restore them.

> Status: **Phase 1 complete, Phase 2 and the first Phase 3 integrations working.** Save your open apps, then restore them, their window layout, browser tabs and VS Code folders from a desktop icon. Capture, versioned storage, Context Diff, retention, export/import, backup, config and logging also work. A first Tauri desktop app is in [app/](app/README.md). See [docs/PLAN.md](docs/PLAN.md), and [docs/TESTING.md](docs/TESTING.md) to try it out.

## Quick start

Requires Rust 1.80+ (MSVC toolchain). Install both programs (`contextflow` and the windowless `contextflow-launch`):

```bash
cargo install --path crates/cf-cli
```

**1. Open the apps you work with, arranged how you like, then save them:**

```bash
contextflow workspace create "University Development"
contextflow capture "University Development" --note "Working on the storage layer"
```

**2. Put an icon on your Desktop:**

```bash
contextflow shortcut "University Development"
```

**3. Double-click the icon.** ContextFlow starts each saved app that isn't already running, waits for its window, and moves every window back to its saved position, falling back to a connected monitor if one has gone. Nothing opens twice if you click it again, and anything it couldn't do is shown in one message box.

You can also run it from a terminal: `contextflow restore "University Development"` (add `--dry-run` to preview).

Store apps such as Teams and Claude are started by their package id. **Chrome and Edge also come back with their tabs** (see below), and **VS Code windows reopen the folder they had open**. For other apps ContextFlow reopens the windows, not what was inside them.

**VS Code folders.** At capture, each VS Code window's folder name is taken from its title and looked up in VS Code's own `storage.json`; on restore the folder is reopened with `code --folder-uri`. If a name could mean more than one folder, no folder is saved for that window (capture says so) rather than guessing.

**Pinning.** The icon restores the *latest* snapshot, so one bad capture would change it. `contextflow pin <workspace> --version N` makes restore (and the icon) always use version N until you `unpin`. A pinned version can't be deleted or pruned. `capture --unminimize` saves minimized windows as normal windows, so they don't reopen minimized.

**Imported workspaces must be approved.** A workspace brought in with `import` could name any program, so it can't be restored until you review it with `contextflow workspace trust <name>` (it lists every program in every version) and approve it with `--yes`. Dry runs are always allowed.

### Reopening browser tabs (one-time setup)

```bash
contextflow browser install
```

Then in Chrome or Edge open `chrome://extensions` (or `edge://extensions`), turn on **Developer mode**, click **Load unpacked** and choose the folder the command printed. `contextflow browser status` should then say the extension is connected. After that, `capture` saves each browser window's tabs and the desktop icon reopens them. The extension sees only tab addresses and titles; it cannot read cookies, history, passwords or page content, and it ignores private windows. Keep a tab out of saves with `excluded_url_patterns` in the config.

| Command | Purpose |
|---|---|
| `workspace create/list/archive/unarchive/delete` | Manage workspaces |
| `workspace trust <name> [--yes]` | Review and approve an imported workspace so it can be restored |
| `capture <ws> [--only a,b] [--note N] [--unminimize] [--dry-run]` | Save the open applications and windows as a new snapshot version (`--unminimize` saves minimized windows as normal; `--repo P` optionally adds Git info) |
| `restore <ws> [--version N] [--only a,b] [--dry-run]` | Reopen the saved applications and put their windows back (uses the pinned version if there is one) |
| `pin <ws> [--version N]` / `unpin <ws>` | Make restore always use one snapshot version / go back to the latest |
| `browser install/status/uninstall` | Set up and check the Chrome/Edge extension |
| `shortcut <ws> [--name N] [--folder DIR] [--force]` | Create a desktop icon that restores the workspace |
| `snapshots <ws>` | Version history |
| `show <ws> [--version N] [--json]` | Inspect a snapshot |
| `diff <ws> --from N [--to M \| --live] [--json]` | Deterministic Context Diff |
| `delete-snapshot <ws> --version N --yes` | Permanently delete one version |
| `prune [<ws> \| --all] [--keep-last N] [--max-age-days D] [--yes]` | Apply retention (dry run without `--yes`; the latest is always kept) |
| `export <ws>... \| --all --out FILE` / `import FILE [--as NAME]` | Portable, hash-verified export and import |
| `backup [--out FILE]` | Consistent, verified copy of the whole database |
| `verify [<ws>] [--version N]` | Re-hash snapshots and check database health |
| `config path/show/init` | Configuration file |
| `info` | Where the database, config, logs and backups live |
| `windows [--json]` | Debug: windows ContextFlow can see |

The database lives in `%APPDATA%\ContextFlow\ContextFlow\data\contextflow.db`, and logs and backups sit next to it. Override the location with `--db` or `CONTEXTFLOW_DB`, and the config file with `--config` or `CONTEXTFLOW_CONFIG`.

## Workspace layout

| Crate | Role |
|---|---|
| `cf-core` | Domain model, canonical SHA-256 hashing, validation, placement geometry (no I/O) |
| `cf-storage` | SQLite (WAL, FK, migrations), immutable snapshots enforced by triggers |
| `cf-config` | `config.toml` loading and validation |
| `cf-browser` | Browser extension protocol, native-messaging host, tab matching and install |
| `cf-vscode` | Works out which folder each VS Code window has open |
| `cf-git` | Read-only Git inspection (`status --porcelain=v2`, `--no-optional-locks`) |
| `cf-diff` | Context Diff across apps, windows, monitors, Git and notes |
| `cf-windows` | Win32 window and monitor discovery (per-monitor DPI aware) |
| `cf-engine` | The restore pipeline: launch, wait for windows, place them, report per app |
| `cf-cli` | `contextflow` (CLI) and `contextflow-launch` (windowless, started by desktop icons) |
| `app/src-tauri` | Tauri desktop app (`contextflow-app`): thin commands over the crates above, React UI in `app/src` |

## Development

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

## Safety principles

ContextFlow never force-closes applications, never mutates Git repositories, never overwrites newer files and never stores credentials or cookies. Every restore step reports an explicit status.
