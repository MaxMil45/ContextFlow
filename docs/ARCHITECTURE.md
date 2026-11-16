# ContextFlow: architecture, status and what is next

Read this first. It says what ContextFlow is, how it is built now, what has
been done from the original review, and what is still to do. The full
technical description (algorithms, data model, trade-offs) is
[SOFTWARE_ARCHITECTURE.md](SOFTWARE_ARCHITECTURE.md).

Numbers were counted from the code on 16 November 2026.

---

## 1. The idea in one sentence

**Press one key and your whole desk changes: the apps, windows, browser tabs,
project folders and terminal tabs of the work you are switching to come back
exactly where they were, and the work you left is saved for later.**

Everything in the project either serves that sentence or waits. That is the
test for every change.

---

## 2. How it is built

Four layers. Each layer only talks to the one below it.

```
 FRONT ENDS      desktop app (tray + windows)   contextflow (CLI)   desktop icon (contextflow-launch)
 what you touch        app/                        cf-cli               cf-cli
                         |                           |                    |
                         | serves the pipe           +---- asks the app over the pipe (cf-ipc),
                         |                                 or does the work itself if it isn't running
                         v                           v                    v
 SERVICES        cf-app: capture, switch/restore, autosave, park, search, shortcuts, health
 what it does
                            |
                            v
 ENGINE          cf-engine: start apps, wait for windows, place them, report per app
 how it does it  cf-cleanup: close the programs a workspace doesn't want
                 detectors:  cf-detect (VS Code, Visual Studio, JetBrains, Explorer, Windows Terminal)
                             cf-browser (tabs, from the extension), cf-git
                            |
                            v
 FOUNDATION      cf-core (data model)   cf-storage (SQLite)   cf-windows (Win32)   cf-config   cf-diff
```

There is **one background program**: the tray app. It owns the shortcuts, the
autosave timer and the pipe the icon and the CLI talk to. There is no separate
agent.

### What happens when you switch workspace

1. **Save where you are.** Capture the open windows, tabs and folders of the
   workspace you are leaving as a new version (skipped if nothing is open).
2. **Stop.** Run its stop commands. Work out what the cleanup rules would close.
3. **Restore, with the cleanup alongside.** The engine brings the desk up in
   stages (apps already running are placed at once, the others start without
   waiting, tabs and folders are added, the rest is polled until it settles).
   At the same time, on another thread, the programs the rules say to close are
   closed.
4. **Finish.** Rebuild the saved stacking order and give the focus to the top
   window. Say what came back, what didn't and why. Nothing opens twice.

Only one switch runs at a time on the machine, whichever front end asked.

### Size

| Part | Lines | What it is |
|---|---|---|
| Rust, 13 crates and the app backend | about 29,100 | 387 tests passing, 4 more run by hand |
| Desktop app UI (`app/src`) | about 5,500 | React |
| Docs | about 2,600 current, 1,000 in `docs/history/` | See section 6 |

---

## 3. The original plan: what was done

The first version of this document found six problems and proposed six steps.
All are done.

| # | Step | Result |
|---|---|---|
| 1 | Measure a switch | Per-phase timings in the log and in `contextflow doctor` ("Last switch"). Waiting for apps, not scanning, is where the seconds go. |
| 2 | Simplify | Four detector crates became one `cf-detect` (now also Windows Terminal); one `RestoreArgs` builder; old docs moved to `docs/history/`. 18 crates became 13. |
| 3 | One background program | The pipe server lives in the tray app; the separate agent is gone. The icon and the CLI still work with the app closed. |
| 4 | Staged restore | Running apps are placed at once; first windows appear in about 0.35 s on a cold switch and in milliseconds on a warm one. |
| 5 | First run and setup | "Save what's open now" on first run. (The two setup chores are still open: see section 4.) |
| 6 | Show it | The README leads with the one-key switch. (The recording is still missing.) |

Done since, beyond the plan: force close never ends a program that is
showing a dialog (it is probably asking to save); ten unused dependencies were
removed; the CLI help leads with the everyday commands and
hides the advanced ones; the "leaving" and "entering" cleanups run together;
the first poll starts at 25 ms; the History tab can delete all older versions; the snapshot fingerprint no longer changes between
identical desks; fixed pauses replaced by place-at-once-then-recheck; cleanup
closes programs together and alongside the restore; Explorer folders are read
from the Shell by window handle; the saved stacking order and focus are
restored; windows saved on a larger display keep their share of a smaller one;
Windows Terminal folders and tabs are saved and reopened.

---

## 4. What is still to do

The full list, in order, with sizes and the test for "done", is
[ROADMAP.md](ROADMAP.md). In short:

* **For the owner:** try the current build on a real desk (several features
  have only been tested with a fake desktop), record the switch for the
  README, tag the first release (`v0.1.0`).
* **Next improvements:** pinned tabs and tab groups (a channel from the app to
  the browser extension), browser profiles, PowerShell's current folder,
  telling windows apart by more than their title.
* **Decided for now:** distributed through GitHub, with an unsigned installer
  and the extension installed with Developer mode. Store publishing and code
  signing come later.
* **Only with evidence:** event-driven window detection, one database
  connection, progress streamed to the command line, database encryption.

---

## 5. Rules to keep it from branching out again

1. **Does it serve the sentence in section 1?** If not, it waits.
2. **One of everything.** One background program, one place that builds
   restore settings, one detector routine, one document to read first.
3. **Front ends stay thin.** If the app and the CLI need the same behaviour,
   it lives in `cf-app`, never in both.
4. **Measure, then optimise.** No speed work without a number before and after.
5. **Finish before starting.** A slice is done when it is used by the product
   and visible to the user, not when it compiles.

---

## 6. Where everything is documented

| Document | What it covers |
|---|---|
| [README.md](../README.md) | What it is, install, the main commands |
| [COMMANDS.md](COMMANDS.md) | Every `contextflow` command and option |
| [ARCHITECTURE.md](ARCHITECTURE.md) | This page |
| [ROADMAP.md](ROADMAP.md) | What is left to do, improvements in order, ideas, and what was turned down |
| [SOFTWARE_ARCHITECTURE.md](SOFTWARE_ARCHITECTURE.md) | The full technical description |
| [AGENT.md](AGENT.md) | The pipe the icon and CLI use: access checks, protocol, fallback |
| [TESTING.md](TESTING.md) | How to test, including on a real desktop |
| `history/` | The original plan and blueprint, unchanged |
