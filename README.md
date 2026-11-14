# ContextFlow

**Capture your environment. Preserve your progress. Resume exactly where you left off.**

ContextFlow is a local-first workspace continuity engine for Windows. It captures windows, monitors, Git state and progress notes into immutable, versioned snapshots. It can compare any snapshot with another one or with the live environment, and (from Phase 2) restore them.

> Status: **Phase 1 complete, Phase 2 and the first Phase 3 integrations working.** Save your open apps, then restore them, their window layout, browser tabs and VS Code folders from a desktop icon. Capture, versioned storage, Context Diff, retention, export/import, backup, config and logging also work. The Tauri desktop app is in [app/](app/README.md). See [docs/PLAN.md](docs/PLAN.md), and [docs/TESTING.md](docs/TESTING.md) to try it out.

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

**3. Double-click the icon.** Like every switch, it first saves the workspace you were in (see *Switch with one keystroke* below). ContextFlow starts each saved app that isn't already running, waits for its window, and moves every window back to its saved position, falling back to a connected monitor if one has gone. Nothing opens twice if you click it again, and anything it couldn't do is shown in one message box.

You can also run it from a terminal: `contextflow restore "University Development"` (add `--dry-run` to preview).

Store apps such as Teams and Claude are started by their package id. **Chrome and Edge also come back with their tabs** (see below), and **VS Code windows reopen the folder they had open**. For other apps ContextFlow reopens the windows, not what was inside them.

**Visual Studio solutions.** Each Visual Studio window's solution or folder name is taken from its title and looked up in Visual Studio's own list of recent solutions (`ApplicationPrivateSettings.xml`); on restore each one reopens with `devenv.exe <path>` in its own window, put back where it was. If two entries share the name, the one opened most recently is used; if they are equally recent, none is saved rather than guessing.

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
| `restore <ws> [--version N] [--only a,b] [--dry-run] [--no-save]` | Switch to the workspace: save the one you're in (unless `--no-save`), then reopen the saved applications and put their windows back (uses the pinned version if there is one) |
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
| `cleanup add/remove/show/apps` | Programs a workspace closes when you enter or leave it |
| `leave` | Leave the active workspace: save it, then run its "when leaving" cleanup |
| `info` | Where the database, config, logs and backups live |
| `windows [--json]` | Debug: windows ContextFlow can see |

The database lives in `%APPDATA%\ContextFlow\ContextFlow\data\contextflow.db`, and logs and backups sit next to it. Override the location with `--db` or `CONTEXTFLOW_DB`, and the config file with `--config` or `CONTEXTFLOW_CONFIG`.

## The desktop app

The app (see [app/README.md](app/README.md)) does everything the command line does, and more:

* **Switch with one keystroke:** press **Ctrl+Alt+W** (change it in Settings) from anywhere, type a few letters and press Enter. ContextFlow saves the workspace you're in (skipped if none of its programs are open, so an emptied desktop never overwrites it), runs its leave rules, restores the other one and runs its enter rules. A card in the corner says how long it took ("“Focus” is ready · 6.2 s"). In the app, the main button on any workspace you're not in is **Switch to this**, and the restore ends with a "Welcome back" card: your note, how long ago it was saved and the Git branch with changed files. Turn auto-save off in Settings → General.
* **Pick up after a restart:** if Windows restarts (or ContextFlow stops without Quit), the next start offers to bring back the workspace you were in. A quiet checkpoint every 20 minutes keeps that restore point fresh (Settings → General).
* **Leave a note:** the switcher and Leave have an optional note box; the note is saved with the workspace you leave and shown first when you come back (`contextflow leave --note "..."`).
* **Always-on apps:** apps you use everywhere (music, chat) are never saved into a workspace and never closed by Focus rules (Settings → Capture).
* **A layout per display setup:** capture once docked and once on the laptop screen; restore uses the newest layout saved with the screens connected now. A pinned version always wins.
* **File Explorer folders** reopen where they were, like VS Code folders and Visual Studio solutions.
* **Health check:** Settings → Advanced (or `contextflow doctor`) checks the database, backups, the browser extension, shortcuts and more, and can save a diagnostics report for bug reports. The database is copied next to itself before any update changes its schema.
* **Keyboard first:** **Ctrl+K** finds any workspace or action, **Ctrl+N** creates one, **Ctrl+1…9** jumps between them, **Ctrl+Enter** restores, **F2** renames, **Ctrl+,** opens Settings.
* **Dashboard and restore:** your workspaces with their last version, a Preview restore (preflight), live progress, Cancel, and Retry for anything that didn't come back. A Windows notification tells you when it finishes.
* **Quick capture:** press **Ctrl+Alt+X** (change it in Settings, or turn it off) from anywhere and the workspace you're in is saved in the background, with no window in the way. A small card in the corner of the screen confirms it at once ("Saving…", then "Saved “Focus” v3 · 7 programs") and fades away; it never takes focus or blocks clicks. The tray's **Capture now** does the same. With no active workspace it saves into the one you used last.
* **Capture review:** a checklist of everything a capture would save (programs, windows, browser tabs, VS Code folders, Git repositories), so you choose what goes in. From the tray, **Capture current…** is two clicks to a saved snapshot.
* **History and compare:** tick two versions to compare them, or compare one with your screen right now. Advice about what restoring would do comes first.
* **Workspace management:** edit the name and description (its desktop icon is renamed too), archive with undo, delete (optionally exporting a copy first, and with its desktop icon), pin a version, delete a version, see recent restores, approve imported workspaces, export and import.
* **Settings:** what is captured and excluded, browser tabs and the extension setup, timeouts, how long history is kept, notifications, close-to-tray and start with Windows. Saved into `config.toml`.
* **Your data:** back up, check integrity, export and import, from the Settings screen.

## Workspace Cleanup ("Focus rules" in the app)

Each workspace can close the programs that distract from it, and free their memory and CPU:

* **When I enter this workspace** (when you restore it): for example Discord, Steam, the Epic launcher.
* **When I leave this workspace** (when you restore a different one, or choose *Leave workspace*): for example Docker Desktop.

Each program is either **Close nicely** (ask it to close like its close button; if it stays open, for example because it hides to the tray, it is left running and reported) or **Force close** (ask first, wait a few seconds, then end it if it is still running). Before anything closes, the app shows what is running now, how many processes, how much memory and CPU closing it would free, and asks you to confirm when you close programs by hand. Programs the workspace itself restores are never closed by its own cleanup.

```bash
contextflow cleanup apps                              # what is running, with memory and CPU
contextflow cleanup add Focus discord                 # close Discord when entering Focus
contextflow cleanup add Focus steam --force           # ...and end Steam if it doesn't close
contextflow cleanup add Focus docker --on-exit        # close Docker when leaving Focus
contextflow cleanup show Focus                        # rules and what they'd free right now
contextflow restore Focus --dry-run                   # includes the cleanup preview
contextflow restore Focus --no-cleanup                # restore without closing anything
contextflow leave                                     # leave the active workspace
```

Cleanup also runs when a desktop icon restores the workspace. Matching is by program name, so every running copy of that program (all of Chrome's processes, say) is affected.

## Workspace layout

| Crate | Role |
|---|---|
| `cf-core` | Domain model, canonical SHA-256 hashing, validation, placement geometry (no I/O) |
| `cf-storage` | SQLite (WAL, FK, migrations), immutable snapshots enforced by triggers |
| `cf-config` | `config.toml` loading and validation |
| `cf-browser` | Browser extension protocol, native-messaging host, tab matching and install |
| `cf-vscode` | Works out which folder each VS Code window has open |
| `cf-vstudio` | Works out which solution or folder each Visual Studio window has open |
| `cf-explorer` | Works out which folder each File Explorer window shows |
| `cf-git` | Read-only Git inspection (`status --porcelain=v2`, `--no-optional-locks`) |
| `cf-diff` | Context Diff across apps, windows, monitors, Git and notes |
| `cf-windows` | Win32 window and monitor discovery (per-monitor DPI aware) |
| `cf-engine` | The restore pipeline: launch, wait for windows, place them, report per app |
| `cf-cleanup` | Workspace cleanup: running programs with memory and CPU, the preview, protected programs, close and force close |
| `cf-app` | Application services shared by every front end: capture, restore sessions, shortcuts, icons, maintenance, logging |
| `cf-cli` | `contextflow` (CLI) and `contextflow-launch` (windowless, started by desktop icons): thin adapters over `cf-app` |
| `app/src-tauri` | Tauri desktop app (`contextflow-app`): thin commands over `cf-app` and the crates above, React UI in `app/src` |

## Development

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

## Safety principles

ContextFlow never mutates Git repositories, never overwrites newer files and never stores credentials or cookies. Every restore step reports an explicit status.

It closes a program only if you listed it in a workspace's cleanup rules (see below). Closing always asks the program first; ending one that ignores the request ("force close") is a separate choice you make per program, and the preview and confirmation say that unsaved work in it is lost. Critical Windows programs, and any program running ContextFlow itself, are never closed.
