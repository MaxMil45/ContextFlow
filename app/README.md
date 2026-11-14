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

## Installer

```bash
powershell -File scriptsuild-installer.ps1
```

Produces `targeteleaseundle
sis\ContextFlow_<version>_x64-setup.exe`: a
per-user installer (no admin prompt, installs under `%LOCALAPPDATA%\ContextFlow`).
It puts four programs side by side: `contextflow-app.exe` (the app),
`contextflow.exe` (the CLI), `contextflow-launch.exe` (what desktop icons run)
and `contextflow-browser-host.exe`. After installing it runs
`contextflow browser install`, and the uninstaller runs `browser uninstall`, so
the Chrome/Edge host is registered under HKCU. Loading the extension in the
browser ("Load unpacked") is still the user's one manual step.

The bundled programs are listed in `src-tauri/tauri.bundle.conf.json`, not in
`tauri.conf.json`, so ordinary dev builds and CI don't need them. The first build
downloads NSIS (cached in `%LOCALAPPDATA%	auri`). The installer is not code
signed yet, so SmartScreen will warn on first run.

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

## Settings and data

Settings are saved to `config.toml` (the same file the CLI reads, found with
`CONTEXTFLOW_CONFIG`). The app rewrites the file when you save, so comments in it
are not kept. Backups, exports, imports and the integrity check use the same code
as `contextflow backup/export/import/verify` (`cf_cli::maintenance`).

## Signing

Set `CONTEXTFLOW_SIGN_THUMBPRINT` (the SHA-1 thumbprint of a code-signing
certificate in your Windows certificate store) before running
`scripts\build-installer.ps1`; optionally `CONTEXTFLOW_SIGN_TIMESTAMP_URL`. In CI,
add the thumbprint as the `CONTEXTFLOW_SIGN_THUMBPRINT` secret on a runner that has
the certificate installed. Without it the installer is unsigned.

## Not done yet

* A signing certificate (the build is ready for one).
* A clean-VM install and uninstall run of the installer.
* The installer does not add `contextflow.exe` to your PATH; run it from the install folder.
* `app-icon.png` is a placeholder; replace it and run `npx tauri icon app-icon.png`.
