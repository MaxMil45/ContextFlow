# ContextFlow desktop app (Phase 5)

Tauri 2 + React 19 + TypeScript (Vite). The Rust side (`src-tauri`, crate
`contextflow-app`) is a thin shell: every behaviour lives in the `cf-*` crates
and is shared with the CLI, so the app and `contextflow` read the same config
and database (`CONTEXTFLOW_DB` / `CONTEXTFLOW_CONFIG` work for both).

## Run

Requires Node 20+ and the Rust toolchain from the repo root README.

```bash
cd app
npm install
npm run tauri dev
```

`npm run tauri build` produces per-user NSIS and MSI installers in
`target/release/bundle/`.

## Layout

| Path | Role |
|---|---|
| `src-tauri/src/commands.rs` | Tauri commands. Thin wrappers; blocking work runs on the blocking pool |
| `src-tauri/src/dto.rs` | IPC payloads, separate from the `cf-core` model |
| `src-tauri/src/error.rs` | `CommandError { code, message }`, the only error the UI sees |
| `src-tauri/src/state.rs` | Paths and config, resolved exactly like the CLI |
| `src-tauri/src/tray.rs` | Tray icon (Open / Quit) |
| `src/bindings/` | **Generated** TypeScript types. Do not edit |
| `src/ipc/client.ts` | Typed `invoke` wrappers and the `restore://progress` listener |
| `src/views/` | Sidebar, workspace detail (capture, history, snapshot), restore panel |

## IPC contract

| Command | Purpose |
|---|---|
| `app_info` | Version and data locations |
| `list_workspaces`, `create_workspace` | Workspaces, most recently used first |
| `list_snapshots`, `get_snapshot` | Version history and one snapshot's contents |
| `capture_workspace` | Same flow as `contextflow capture` (reuses repos, auto-prune) |
| `restore_workspace` | Restore or dry run; emits `restore://progress` lines |
| `cancel_restore` | Stops before the next app starts; never closes anything |

After changing a struct in `dto.rs` or `error.rs`, regenerate the TS types and
commit them (CI fails if they are stale):

```bash
npm run bindings
```

## Not done yet (see docs/PLAN.md, Phase 5)

Tray capture/restore shortcuts and autostart (5.1), capture review checklist
(5.4), diff view (5.7), settings (5.8), toasts (5.9), code signing and HKCU
native-messaging registration in the installer (5.10). `app-icon.png` is a
placeholder; replace it and run `npx tauri icon app-icon.png`.
