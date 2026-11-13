# ContextFlow

**Capture your environment. Preserve your progress. Resume exactly where you left off.**

ContextFlow is a local-first workspace continuity engine for Windows. It captures windows, monitors, Git state and progress notes into immutable, versioned snapshots. It can compare any snapshot with another one or with the live environment, and (from Phase 2) restore them.

> Status: **Phase 1 complete, first Phase 2 slice working.** Save your open apps, then restore them (and their window layout) from a desktop icon. Capture, versioned storage, Context Diff, retention, export/import, backup, config and logging also work. VS Code/browser contents and the Tauri UI come next. See [docs/PLAN.md](docs/PLAN.md), and [docs/TESTING.md](docs/TESTING.md) to try it out.

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

Not every app can be restored. Microsoft Store apps (such as the new Notepad, Calculator and Windows Terminal) are skipped and reported. ContextFlow also reopens each app's windows, not what was inside them (which folder, which tabs). That comes with the VS Code and browser integrations in Phase 3.

| Command | Purpose |
|---|---|
| `workspace create/list/archive/unarchive/delete` | Manage workspaces |
| `capture <ws> [--only a,b] [--note N] [--dry-run]` | Save the open applications and windows as a new snapshot version (`--repo P` optionally adds Git info) |
| `restore <ws> [--version N] [--only a,b] [--dry-run]` | Reopen the saved applications and put their windows back |
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
| `cf-adapters` | `ContextAdapter` integration contract and registry |
| `cf-git` | Read-only Git inspection (`status --porcelain=v2`, `--no-optional-locks`) |
| `cf-diff` | Context Diff across apps, windows, monitors, Git and notes |
| `cf-windows` | Win32 window and monitor discovery (per-monitor DPI aware) |
| `cf-engine` | The restore pipeline: launch, wait for windows, place them, report per app |
| `cf-cli` | `contextflow` (CLI) and `contextflow-launch` (windowless, started by desktop icons) |

## Development

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

## Safety principles

ContextFlow never force-closes applications, never mutates Git repositories, never overwrites newer files and never stores credentials or cookies. Every restore step reports an explicit status.
