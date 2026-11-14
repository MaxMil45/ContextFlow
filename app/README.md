# ContextFlow desktop app

Tauri 2 + React 19 + TypeScript (Vite). The Rust side (`src-tauri`, crate
`contextflow-app`) is a thin shell over the `cf-app` service crate and the other
`cf-*` crates, so the app, the CLI and desktop icons behave the same and share
one config and database (`CONTEXTFLOW_DB` / `CONTEXTFLOW_CONFIG` work for all).

## Run

Requires Node 20+ and the Rust toolchain from the repo root README.

```bash
cd app
npm install
npm run tauri dev
```

The window uses the native Windows frame (Snap Layouts, Alt+Space menu). The
first `tauri dev` compiles the whole workspace and can take a few minutes before
the window appears.

## What is in the window

* **Sidebar:** workspaces with their picture or colour, Ctrl+K search, an
  Archived group, Settings and Keyboard shortcuts.
* **Workspace page:** one main action. *Switch to this* on a workspace you're
  not in (save where you are, run its leave rules, bring this one back) or
  *Restore* on the one you're in. Tabs: **Overview** (where you left off, what
  comes back), **Focus rules** (programs closed on entering or leaving),
  **History** (versions, auto-saves marked, recent restores).
* **Settings:** General (switcher, auto-save, quick capture), Capture, Restore,
  Your data, Advanced (About, logging).

Two small extra pages, each without React so they stay light:
`toast.html` (the corner card, created hidden at start-up) and
`switcher.html` (the Ctrl+Alt+W picker, created on first use).

## Layout

| Path | Role |
|---|---|
| `src-tauri/src/commands.rs` | Workspace, snapshot, capture and restore commands |
| `src-tauri/src/cleanup_commands.rs` | Focus rules: running programs, previews, leave |
| `src-tauri/src/review.rs` | Capture review and compare |
| `src-tauri/src/settings.rs` | Settings, browser setup, backup, export, import, retention |
| `src-tauri/src/switcher.rs` | The global switcher window and quick switch |
| `src-tauri/src/hotkey.rs` | Global shortcuts: quick capture and the switcher |
| `src-tauri/src/overlay.rs` | The corner card |
| `src-tauri/src/shell.rs` | Tray and notifications |
| `src-tauri/src/dto/` | IPC payloads (`workspace`, `cleanup`, `settings`), separate from `cf-core` |
| `src-tauri/src/error.rs` | `CommandError { code, message }`, the only error the UI sees |
| `src/bindings/` | **Generated** TypeScript types. Do not edit |
| `src/ipc/client.ts` | Typed `invoke` wrappers and event listeners |
| `src/views/`, `src/components/` | The React UI |

After changing a struct in `dto/` or `error.rs`, regenerate the TS types and
commit them (CI fails if they are stale):

```bash
npm run bindings
```

## Window permissions

`build.rs` lists every command, so each becomes a permission, and each window is
granted only what it uses (`capabilities/`): the main window everything except
the small windows' own commands, the switcher `list_workspaces`, `quick_switch`
and `hide_switcher`, the corner card only `current_toast`. Add a new command to
`build.rs` and to the right capability, or no window can call it.

## Switching and auto-save

Every real restore of another workspace is a switch, wherever it starts (the
app, the switcher, the tray, a desktop icon, `contextflow restore`). Before the
leave rules run, `cf_app::autosave` saves the workspace being left, unless:

* none of its saved programs are open (an emptied desktop never replaces a save),
* nothing that matters changed (window titles and capture metadata don't count),
* *Save the workspace I'm leaving* is off in Settings, or `--no-save` is passed.

Auto-saves keep the last note and are counted separately by retention: the
newest five are kept, and they never push out versions you saved on purpose.

## Installer

```bash
powershell -File scripts\build-installer.ps1
```

Produces `target\release\bundle\nsis\ContextFlow_<version>_x64-setup.exe`: a
per-user installer (no admin prompt, installs under `%LOCALAPPDATA%\ContextFlow`).
It puts five programs side by side: `contextflow-app.exe` (the app),
`contextflow.exe` (the CLI), `contextflow-launch.exe` (what desktop icons run),
`contextflow-agent.exe` (the optional background agent, see
[docs/AGENT.md](../docs/AGENT.md)) and `contextflow-browser-host.exe`. After installing it runs
`contextflow browser install`, and the uninstaller runs `browser uninstall`, so
the Chrome/Edge host is registered under HKCU. Loading the extension in the
browser ("Load unpacked") is still the user's one manual step.

The bundled programs are listed in `src-tauri/tauri.bundle.conf.json`, not in
`tauri.conf.json`, so ordinary dev builds and CI don't need them. The first build
downloads NSIS (cached in `%LOCALAPPDATA%\tauri`).

## Signing

Set `CONTEXTFLOW_SIGN_THUMBPRINT` (the SHA-1 thumbprint of a code-signing
certificate in your Windows certificate store) before running
`scripts\build-installer.ps1`; optionally `CONTEXTFLOW_SIGN_TIMESTAMP_URL`. In CI,
add the thumbprint as the `CONTEXTFLOW_SIGN_THUMBPRINT` secret on a runner that has
the certificate installed. Without it the installer is unsigned and SmartScreen
warns on first run.

## Settings and data

Settings are saved to `config.toml` (the same file the CLI reads). The app
rewrites the file when you save, so comments in it are not kept. Backups,
exports, imports and the integrity check use the same code as
`contextflow backup/export/import/verify` (`cf_app::maintenance`).

## Not done yet

* A signing certificate, an auto-updater and a store-listed browser extension.
* A clean-VM install and uninstall run of the installer.
* Programs are told apart by executable name, so Focus rules show which files a
  rule matches and warn when one name matches several programs.
* `app-icon.png` is a placeholder; replace it and run `npx tauri icon app-icon.png`.
