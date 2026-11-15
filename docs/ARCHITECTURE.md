# ContextFlow: architecture, honest review and the plan

This is the one document to read first. It says what ContextFlow is, how it is
built today, where it has grown wider than it needs to be, what to cut, and
what to build next so the product impresses people.

Numbers here were counted from the code on 15 November 2026. Savings are
estimates, and are marked as such.

---

## 1. The idea in one sentence

**Press one key and your whole desk changes: the apps, windows, browser tabs
and project folders of the work you are switching to come back exactly where
they were, and the work you left is saved for later.**

Everything else in the project either serves that sentence or is a
distraction from it. That is the test used throughout this document.

---

## 2. How it is built today

Four layers. Each layer only talks to the one below it.

```
 FRONT ENDS      desktop app (tray + windows)   contextflow (CLI)   desktop icon (contextflow-launch)
 what you touch        app/                        cf-cli               cf-cli
                         \                           |                    /
                          \                          |    (optional) background agent
                           \                         |    cf-agent + cf-ipc
                            v                        v                  v
 SERVICES        cf-app: capture, switch/restore, autosave, park, search, shortcuts, health
 what it does
                            |
                            v
 ENGINE          cf-engine: start apps, wait for windows, place them, report per app
 how it does it  cf-cleanup: close the programs a workspace doesn't want
                 detectors: cf-browser, cf-vscode, cf-vstudio, cf-explorer, cf-jetbrains, cf-git
                            |
                            v
 FOUNDATION      cf-core (data model)   cf-storage (SQLite)   cf-windows (Win32)   cf-config   cf-diff
```

### What happens when you switch workspace

1. **Save where you are.** Capture the open windows, tabs and folders of the
   workspace you are leaving as a new version (skipped if nothing of it is open).
2. **Leave.** Run its stop commands, then close the programs its rules say to close.
3. **Enter.** Close what the new workspace doesn't want, run its start commands.
4. **Restore.** Start each saved app that isn't running, wait for its window,
   put every window back, reopen tabs and folders.
5. **Report.** Say what came back, what didn't and why. Nothing opens twice.

Only one switch runs at a time on the machine, whichever front end asked.

### Size

| Part | Lines | What it is |
|---|---|---|
| Rust, 18 crates | about 26,500 | 342 tests passing |
| Desktop app backend (`app/src-tauri`) | 4,774 of the above | The largest single crate |
| Desktop app UI (`app/src`) | about 6,000 | React; two screens are 1,117 and 793 lines |
| Docs | about 1,900 | Five files, with overlap |

---

## 3. Honest review

### What is strong

* **The core is sound.** Snapshots are immutable and versioned, restores report
  per app, imported workspaces must be approved, and a second double-click
  never starts everything twice. This is the part that is hard to get right.
* **The layering is real.** The engine doesn't know about the UI, and the three
  front ends share one set of services.
* **It is tested.** 342 tests, and CI runs format, lint and tests on every push.

### Where it has branched out

**1. Two background programs doing one job.** The tray app already stays
running, already owns the global shortcuts, the autosave timer and the
restart recovery, and already closes its windows when hidden. The new agent
(`cf-agent` + `cf-ipc`, about 2,750 lines with tests) is a *second* always-on
program that so far only takes restores from the icon and the CLI. As built,
it adds a moving part without yet giving the user anything they can see. This
is the biggest architectural question in the project; section 4 settles it.

**2. Four detector crates that are the same program four times.**
`cf-vscode`, `cf-vstudio`, `cf-explorer` and `cf-jetbrains` (1,472 lines) each
do the same thing: take a window title, find the matching folder or project
in that app's own recent list, and check the path is safe. The path-safety
function is copied nearly character for character in three of them, the saved
state has the same shape in all four, and `cf-app/src/capture.rs` has four
near-identical `attach_…` functions to call them.

**3. The same restore settings built in three places.** The agent, the CLI
dry run and the desktop app each fill in the same twelve-field `RestoreArgs`
by hand. Three copies of one decision is how front ends drift apart.

**4. Features ahead of the core promise.** The feature list is long: diff,
verify, prune, export with encryption, park, timeline, search, start/stop
commands, health check, four IDE integrations. Each is reasonable alone.
Together they hide the one thing that sells the product, and each one is code
to keep working. The CLI alone has more than 20 commands.

**5. Docs that describe the past.** `BLUEPRINT.md` (607 lines) is a task list
that has been fully delivered. `PLAN.md` has three "update" sections appended
to it and a timeline that no longer matches. A new contributor has to read
1,900 lines to learn what this page says.

**6. Speed is unmeasured.** The app shows "ready · 6.2 s" after a switch, but
nothing records where those seconds go. The agent was justified by speed, yet
the cost it removes (starting a small program and opening the database) is
probably tens of milliseconds, while waiting for apps to open their windows
is seconds. Until that is measured, speed work is guesswork.

---

## 4. Decisions

### Decision 1: one background program, not two

**Make the tray app the agent.** Put the pipe server (`cf-ipc`) inside the
desktop app's process. Delete the separate `contextflow-agent` program.

Why this is the efficient shape:

* The tray app is already running, already starts with Windows, and already
  holds the shortcuts and timers. Nothing has to be moved out of it.
* The icon and the CLI then talk to the app when it is running, and do the
  work themselves when it isn't. That fallback already exists and is tested.
* It removes, rather than adds: the agent program, its start/stop/status
  commands, the installer steps that stop it, and three planned slices of work
  ("start at sign-in", "move autosave", "move shortcuts").

What is kept: all of `cf-ipc` (protocol, pipe security, tests) and the shared
request handler. What goes: about 300 lines (the agent's `main`, the
lifecycle module, `contextflow agent …`, installer hooks). The real saving is
the work that no longer needs doing.

The cost: the tray app uses more memory than a bare agent would. That is a
fair price for one program instead of two. If memory ever becomes the
complaint, a headless agent can be split out again, because the protocol
won't have changed.

### Decision 2: one detector, four small tables

Merge the four detector crates into one `cf-detect` crate with a single
"window title to project path" routine and one small description per app
(how to read the title, where its recent list lives). Adding a fifth editor
becomes a table entry and a parser, not a crate.

Estimated saving: 300 to 400 lines, three crates, and three of the four
`attach_…` functions.

### Decision 3: restore settings come from one function

One function turns "config + what was asked" into `RestoreArgs`. The app, the
CLI and the pipe handler all call it. Estimated saving: about 40 lines, and a
whole class of "it behaves differently from the app" bugs.

### Decision 4: measure before optimising

Record how long each phase of a switch takes (save, close, start, wait for
windows, place) and show it in the log and in `doctor`. Every speed decision
after that is made from those numbers.

### Decision 5: a smaller front door

Keep every feature that exists, but stop presenting them all at once. The
README, the CLI help and the app's first screen lead with create, capture,
switch and the desktop icon. Diff, verify, prune, export and backup move
under an "advanced" heading. No code is deleted for this; it is what people
see first that changes.

### Decision 6: one living document

This file replaces `PLAN.md` and `BLUEPRINT.md` as the thing to read. Move
those two to `docs/history/` unchanged. Keep `TESTING.md`. Shrink `AGENT.md`
to the protocol and security notes once Decision 1 is done.

---

## 5. What makes people say "wow"

People are not impressed by a feature list. They are impressed by one moment
that works every time. For ContextFlow that moment is:

> Ctrl+Alt+W, type "cli", Enter. Three seconds later the editor, the
> terminal, the browser with the right twelve tabs and the docs are on the
> right screens, and the game launcher and chat have closed themselves.

Four things make that moment land, in order of importance:

1. **It is fast, and visibly so.** Show the windows arriving, most important
   first, rather than a spinner followed by everything at once. Apps already
   running should be in place in under a second.
2. **It never embarrasses the user.** Nothing opens twice, nothing they were
   working on is lost, and anything that didn't come back is named plainly.
   This already works; it needs protecting, not building.
3. **The first five minutes need no manual.** On first run, look at the
   desktop and offer "Save what's open now as your first workspace". One
   click, and they have something to switch back to.
4. **No setup chores.** Today browser tabs need Developer mode and "Load
   unpacked", and the installer is unsigned so Windows warns about it. Publish
   the extension to the Chrome and Edge stores and sign the installer. These
   two chores lose more people than any missing feature.

And one thing to make the wow reach people who haven't installed it: a
20-second screen recording of a switch at the top of the README.

---

## 6. The plan

Each step is small enough to finish and check before starting the next.

| # | Step | Why now | Done when |
|---|---|---|---|
| 1 | **Measure a switch.** Per-phase timings in the log and `doctor`. | Every later choice depends on it. | A switch prints where its seconds went. |
| 2 | **Simplify.** Merge the detectors, one `RestoreArgs` builder, move old docs to `docs/history/`. | Less code to carry into everything after. | Three fewer crates, same tests passing. |
| 3 | **One background program.** Pipe server inside the tray app; remove the separate agent. | Settles the architecture before more is built on it. | Icon and CLI are served by the running app, and still work with it closed. |
| 4 | **Staged restore.** Place already-running apps at once; bring the rest in as they open, most important first, with visible progress. | This is the "wow" moment. | Warm switch under one second to first windows; progress visible throughout. |
| 5 | **First-run and setup.** "Save what's open now", store-published extension, signed installer. | Removes the reasons people give up. | A new user switches workspaces within five minutes with no terminal. |
| 6 | **Show it.** README leads with the recording and three commands. | Makes the work visible. | Someone who has never seen it understands it in 20 seconds. |

### Deliberately not doing yet

* **A live, event-driven model of the desktop.** Only worth it if step 1
  shows that scanning the desktop is a real share of switch time.
* **More integrations** (more IDEs, terminals, Office documents). After step 2
  they are cheap to add; before step 4 they don't change the first impression.
* **Triggers** (switch when docking, at a time of day). A good later feature,
  but it builds on a switch that is already fast and trusted.
* **Anything on macOS or Linux.**

---

## 7. Rules to keep it from branching out again

1. **Does it serve the sentence in section 1?** If not, it waits.
2. **One of everything.** One background program, one place that builds
   restore settings, one detector routine, one document to read first.
3. **Front ends stay thin.** If the app and the CLI need the same behaviour,
   it lives in `cf-app`, never in both.
4. **Measure, then optimise.** No speed work without a number before and after.
5. **Finish before starting.** A slice is done when it is used by the product
   and visible to the user, not when it compiles.
