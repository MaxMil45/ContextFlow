# ContextFlow

**Capture your environment. Preserve your progress. Resume exactly where you left off.**

ContextFlow is a local-first workspace continuity engine for Windows. It captures windows, monitors, Git state and progress notes into immutable, versioned snapshots. It can compare any snapshot with another one or with the live environment, and (from Phase 2) restore them.

> Status: **Phase 0/1.** Core engine, SQLite storage, Git inspection, Context Diff, Win32 discovery and the CLI all work. Restoration, adapters and the Tauri UI are planned. See [docs/PLAN.md](docs/PLAN.md).

## Quick start

Requires Rust 1.80+ (MSVC toolchain) and Git on `PATH`.

```bash
cargo build --release
```

```bash
target/release/contextflow workspace create "University Development"
target/release/contextflow capture "University Development" --repo C:/src/my-project --note "Finished the storage layer; next: restore pipeline"
target/release/contextflow snapshots "University Development"
target/release/contextflow diff "University Development" --from 1 --live
```

| Command | Purpose |
|---|---|
| `workspace create/list/archive/unarchive` | Manage workspaces |
| `capture <ws> [--repo P]... [--note N] [--no-windows] [--dry-run]` | Capture a new snapshot version (repos default to those in the previous snapshot) |
| `snapshots <ws>` | Version history |
| `show <ws> [--version N] [--json]` | Inspect a snapshot |
| `diff <ws> --from N [--to M \| --live] [--json]` | Deterministic Context Diff |
| `verify <ws> [--version N]` | Re-hash snapshots and check database health |
| `windows [--json]` | Debug: windows ContextFlow can see |

The database lives in `%APPDATA%\ContextFlow\ContextFlow\data\contextflow.db`. Override it with `--db` or `CONTEXTFLOW_DB`.

## Workspace layout

| Crate | Role |
|---|---|
| `cf-core` | Domain model, canonical SHA-256 hashing, validation, placement geometry (no I/O) |
| `cf-storage` | SQLite (WAL, FK, migrations), immutable snapshots enforced by triggers |
| `cf-adapters` | `ContextAdapter` integration contract and registry |
| `cf-git` | Read-only Git inspection (`status --porcelain=v2`, `--no-optional-locks`) |
| `cf-diff` | Context Diff across apps, windows, monitors, Git and notes |
| `cf-windows` | Win32 window and monitor discovery (per-monitor DPI aware) |
| `cf-cli` | The `contextflow` binary |

## Development

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

## Safety principles

ContextFlow never force-closes applications, never mutates Git repositories, never overwrites newer files and never stores credentials or cookies. Every restore step reports an explicit status.
