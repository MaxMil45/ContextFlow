# Command reference

Every `contextflow` command, what it does and its options. The program's own
help is always the final word: `contextflow --help`, and
`contextflow <command> --help` for one command. This page was written from that
help.

**Everyday use** needs only four things: create a workspace, capture, restore
(switch) and the desktop icon. Everything under "Advanced" is there when you
need it, and works the same; `contextflow --help` lists the everyday commands
and only names the advanced ones at the end.

Options every command accepts:

| Option | Meaning |
|---|---|
| `--db <file>` | Use this database file instead of the default (or set `CONTEXTFLOW_DB`). Handy for trying things out without touching your real workspaces. |
| `--config <file>` | Use this config file instead of the default (or set `CONTEXTFLOW_CONFIG`). |
| `-h`, `--help` | Show help for the command. |
| `-V`, `--version` | Show the version (on `contextflow` itself). |

A workspace is named by its name, as in `contextflow restore cli`.

---

## Everyday

### `workspace create <NAME>`

Make a new, empty workspace. Add `--description "text"` to describe it.

```bash
contextflow workspace create cli --description "CLI work"
```

### `capture <WORKSPACE>`

Save what is open right now as a new version of the workspace: the applications
and their windows, the tabs open in Chrome or Edge (with the extension), the
folders open in VS Code, Visual Studio, JetBrains IDEs and File Explorer, and
the folder of each tab in Windows Terminal. Earlier versions are kept.

| Option | Meaning |
|---|---|
| `--only a,b` | Save only these applications (names as shown by `windows`). |
| `--note "text"` | A note stored with this version, shown first when you come back. |
| `--repo <path>` | Also record a Git repository's branch and changes (repeat for several). Not needed to restore your applications. |
| `--no-windows` | Skip window and monitor capture. |
| `--unminimize` | Save minimized windows as normal windows. |
| `--dry-run` | Show what would be saved, without saving. |

```bash
contextflow capture cli --note "before lunch"
```

### `restore <WORKSPACE>`

Switch to the workspace: save the one you are leaving, close what the rules say
to close, start the applications that aren't running, and put every window back
where it was.

| Option | Meaning |
|---|---|
| `--version <n>` | Restore this version instead of the latest. |
| `--only a,b` | Restore only these applications. Cleanup is skipped. |
| `--dry-run` | Say what would happen without starting or moving anything. |
| `--timeout <seconds>` | How long to wait for each application's window (default 30). |
| `--no-cleanup` | Close nothing, whatever the cleanup rules say. |
| `--no-save` | Don't save the workspace you are leaving first. |

```bash
contextflow restore cli
contextflow restore cli --dry-run
```

Running it again when everything is already in place changes nothing: nothing
opens twice.

### `shortcut <WORKSPACE>`

Create a desktop icon that switches to the workspace with a double-click.

| Option | Meaning |
|---|---|
| `--name <name>` | The icon's name (default: the workspace name). |
| `--folder <path>` | Where to put it (default: your Desktop). |
| `--force` | Replace an existing icon. |
| `--icon <file>` | Use a picture (PNG, JPEG, BMP or ICO) as the icon. Kept for this workspace. |
| `--default-icon` | Go back to the standard ContextFlow icon. |

### `leave`

Leave the active workspace without entering another: save it, then run its
"when leaving" cleanup. `--note "text"` adds a note shown first when you come
back.

---

## Looking after workspaces

### `workspace …`

| Command | What it does |
|---|---|
| `workspace list` | List workspaces. |
| `workspace rename <NAME> <NEW_NAME>` | Change the name. Snapshots, pin and icon are kept. |
| `workspace archive <NAME>` / `unarchive <NAME>` | Hide a workspace without deleting it, and bring it back. |
| `workspace delete <NAME>` | Permanently delete a workspace and all its snapshots. |
| `workspace trust <NAME>` | Review and approve an imported workspace so it can be restored. `--yes` approves without asking. |
| `workspace save-mode <NAME> [MODE]` | What happens when you switch away: `auto` saves its changes, `notify` saves and says so, `fixed` never saves, so it always restores as captured. |

### `cleanup …`

Programs a workspace closes when you enter or leave it, so distractions go away.
Protected system programs are never touched.

| Command | What it does |
|---|---|
| `cleanup show <WORKSPACE>` | Show the rules and what they would close right now. |
| `cleanup add <WORKSPACE> <PROGRAM>` | Add a program, like `discord`. By default it is closed when the workspace is **entered**. `--on-exit` closes it when the workspace is left instead. `--force` ends it if it doesn't close when asked (unsaved work in it is lost). |
| `cleanup remove <WORKSPACE> <PROGRAM>` | Remove a program from the list. |
| `cleanup apps` | List the programs running now, with their memory and CPU use. |

```bash
contextflow cleanup add writing discord
contextflow cleanup show writing
```

### `pin <WORKSPACE>` and `unpin <WORKSPACE>`

`pin --version <n>` makes restore (and the desktop icon) always use one version.
Without `--version` it pins the latest. `unpin` goes back to using the latest.

### Browser tabs: `browser …`

Saving tabs needs the ContextFlow extension in Chrome or Edge.

| Command | What it does |
|---|---|
| `browser install` | Register ContextFlow with Chrome and Edge and unpack the extension. |
| `browser status` | Show whether it is set up and which tabs are visible now. |
| `browser uninstall` | Remove the registration (remove the extension in the browser itself). |

### Opening a specific project in a one-window app (Obsidian vaults, and similar)

ContextFlow restores a running app by moving its windows; it doesn't tell it
*what* to open. For apps that switch project through an address or an
argument, use a workspace's **start command** (Focus rules tab, "When I enter
this workspace"). It runs after the windows are back, exactly as written:

```
"C:\Users\you\AppData\Local\Programs\Obsidian\Obsidian.exe" "obsidian://open?vault=Work"
```

Give each workspace its own vault in its own start command (`vault=Personal` in
the other). Use `%20` for a space in a name. The program must be a full path to
an `.exe`; nothing runs through a shell, and nothing runs elevated. The same
works for any app that takes a project, profile or address on its command line.
Every switch runs the command again, which is harmless for an address that
just opens or focuses something, but not for one that opens a new window each
time.

---

## Advanced

### Looking at history

| Command | What it does |
|---|---|
| `snapshots <WORKSPACE>` | List a workspace's versions. |
| `show <WORKSPACE>` | Show a version (latest by default). `--version <n>` picks one, `--json` prints it as data. |
| `diff <WORKSPACE>` | Compare two versions: `--from <n>` (required) and `--to <n>` (default latest). `--live` compares a version with what is open now. `--json` prints data. |

### Keeping the database tidy

| Command | What it does |
|---|---|
| `delete-snapshot <WORKSPACE> --version <n> --yes` | Permanently delete one version. |
| `prune [WORKSPACE]` | Delete old versions by the retention rules. Shows what would go unless `--yes` is given. The latest version is always kept. `--all` for every workspace; `--keep-last <n>` and `--max-age-days <n>` override the rules. |
| `verify [WORKSPACE]` | Check version hashes and database health (all workspaces by default). `--version <n>` checks one. |
| `backup` | Write a consistent copy of the whole database. `--out <file>` chooses where; the default is a backups folder next to the database. |

### Moving workspaces between computers

| Command | What it does |
|---|---|
| `export <WORKSPACES…> --out <file>` | Write workspaces, with their full history, to a file. `--all` for every workspace, `--force` to overwrite a file, `--encrypt` to protect it with a passphrase read from `CONTEXTFLOW_PASSPHRASE`. |
| `import <FILE>` | Read an export file. Hashes are verified first. `--as <name>` imports under a different name (single-workspace files only). Imported workspaces must be approved with `workspace trust` before they can be restored. |

### Settings and diagnosis

| Command | What it does |
|---|---|
| `config path` | Print where the config file is. |
| `config show` | Print the settings in effect (file merged with defaults). |
| `config init` | Write a commented starter config file. |
| `info` | Show where the database, config and logs are. |
| `doctor` | Check what commonly stops ContextFlow from working and how to fix it. Includes a "Last switch" line saying where the time of the latest switch went. |
| `windows` | Debug: list the windows ContextFlow can see right now (`--json` for data). Its names are the ones `--only` expects. |

---

## Trying things safely

Use a separate database so nothing touches your real workspaces:

```bash
contextflow --db C:\temp\try.db workspace create demo
contextflow --db C:\temp\try.db capture demo --dry-run
```

`restore --dry-run` and `capture --dry-run` never start, move or save anything.
