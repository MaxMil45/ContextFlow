# The ContextFlow agent

**Status:** design note for Phase 2's headless agent. Built so far:

* Slice 1: `cf-ipc` (protocol and pipe transport), `cf-agent` (the
  `contextflow-agent` program and its request handler) and
  `contextflow-launch` sending its restore to a running agent.
* Slice 2: `contextflow restore` and `contextflow leave` go through the agent
  (protocol v2), and `contextflow agent start|status|stop` manages it.

Everything else here is the plan for the next slices.

## Why

Today every front end (the tray app, `contextflow`, `contextflow-launch`) opens
SQLite itself and the only thing they share is the restore mutex
`Local\ContextFlow.Restore`. Each desktop-icon double-click starts a new
process that loads the config, opens and migrates the database and samples the
desktop from scratch. A long-lived agent can keep that state warm, which is
the base for faster switches, staged restore and triggers. Front ends become
thin clients that can come and go; the tray app already closes its web views
when hidden.

## What the agent owns

| Responsibility | Today | With the agent |
| --- | --- | --- |
| **Database** | Each front end opens it | The agent holds the only long-lived connection and does every write. Front ends ask over the pipe. (Slice 1: the agent opens it at start; front ends still open it too, which SQLite handles, until each operation moves.) |
| **Restore lock** | Named mutex, taken by whichever process restores | The agent serialises restores itself. It keeps taking `Local\ContextFlow.Restore` while any front end can still restore in-process (the fallback), so the two never run at once. |
| **Checkpoints / autosave** | Run from the tray app's timer and on switch | The agent runs the timer, so saving works with no window open. |
| **Global shortcuts** | Registered by the tray app | Registered by the agent; a shortcut sends a request to itself. |
| **Live desktop model** | (none) | Later slice: an event-driven model fed by WinEvent hooks, replacing per-switch sampling. Not part of slice 1. |

The agent has no UI. It never shows a message box: problems go back to the
client that asked, and the client decides how to show them.

## The channel

A **named pipe per user and per login session**:

```
\\.\pipe\ContextFlow.Agent.<user SID>.<session id>
```

Pipes live in one machine-wide namespace, so the name carries the user's SID
(two users never collide) and the Windows session id (the same user signed in
on the console and over RDP gets two agents, each driving its own desktop).

### Access check

The name is not a secret, so it is not the security boundary. These are:

1. **DACL.** The agent creates every pipe instance with the security
   descriptor `O:<SID>D:P(A;;GA;;;<SID>)`: owned by the user, a protected DACL
   with one entry granting the user full access, nothing inherited. Other
   users, including administrators without taking ownership, cannot open it
   or create instances of it. Objects with no explicit label get the default
   *medium, no-write-up* mandatory label, so low-integrity processes of the same
   user (browser sandboxes, AppContainers) cannot write to it either.
2. **No remote clients.** `PIPE_REJECT_REMOTE_CLIENTS` refuses SMB connections.
3. **No squatting.** The first instance is created with
   `FILE_FLAG_FIRST_PIPE_INSTANCE`; if the name already exists the agent exits
   instead of sharing it. Further instances need `FILE_CREATE_PIPE_INSTANCE`,
   which the DACL gives only to the user.
4. **Server checks the client.** After the first request arrives the agent
   impersonates the client (`ImpersonateNamedPipeClient`), reads the token's
   user SID, reverts, and also reads the client's session id
   (`GetNamedPipeClientSessionId`). Both must match the agent's own, or the
   connection is answered `denied` and closed. This holds even if the DACL
   were ever loosened. An agent running as administrator also refuses a
   client that is not: it starts every program it restores with its own
   rights, so answering would hand those rights to an ordinary process. The
   refused client does the work itself, with its own rights.
5. **Client checks the server.** Clients open the pipe with
   `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, so the agent can only
   *identify* them, never act as them. The client then reads the pipe's owner
   SID (`GetSecurityInfo`) and the server's session id, and talks to it only if
   both are its own. A pipe planted by someone else is treated as "no agent".

Within one user and session there is no further boundary: any process the
user runs at medium integrity can already do everything ContextFlow does. The
agent still keeps the existing in-band rules: imported workspaces must be
approved before they restore, and a request names a workspace, never a path,
program or command line. The agent never opens a database or config path that
a client sends; a client started with `--db` or `--config` restores in-process
instead of asking the agent.

### Protocol

Framing: a 4-byte little-endian length, then that many bytes of UTF-8 JSON.
Frames over 1 MiB are refused. A connection carries any number of
request/response pairs in order; either side may close it.

```json
→ {"v":2,"id":7,"op":"restore","workspace":"Client A","only":["code"]}
← {"v":2,"id":7,"status":"restored","workspace":"Client A","version":12,"overall":"success","steps":[…],…}
```

* `v` is the protocol version of the sender. The agent answers any version in
  `MIN_SUPPORTED..=PROTOCOL_VERSION`; anything else gets an
  `unsupported_version` error carrying both bounds, before anything happens.
* `id` is echoed so a client can match replies.
* `op` names the request: `hello`, `restore`, `leave`, `shutdown`. An op the
  agent does not know gets `unknown_operation`, again before anything happens.
* `status` names the reply: `hello` (agent version and protocol range),
  `restored` (workspace, version, per-application steps, what was saved and
  closed first, problems, progress lines), `left`, `busy` (another restore
  holds the lock), `shutting_down`, `error` (`code` + `message`).

New fields are added with serde defaults so older peers ignore them; a change
that older peers cannot ignore bumps `PROTOCOL_VERSION`.

| Version | Adds |
| --- | --- |
| v1 | `hello`, `restore` (workspace, version), `shutdown` |
| v2 | `restore` options `only`, `timeout_secs`, `no_cleanup`, `no_save` (an agent that ignored them would do the wrong restore, hence the bump); `leave`; fuller summaries |

The agent answers v1 and v2. `hello` and `shutdown` are sent as v1 by
`contextflow agent status|stop`, so an agent of any age can be found and
stopped.

Progress lines are not streamed yet: the agent returns them in the summary and
the client prints them afterwards. In-process they are still shown live.

## Clients and fallback

Every client follows one rule: **if the agent cannot take the request, do it
in-process exactly as today.** "Cannot" means: no pipe, the pipe busy past a
short wait, the server failing the identity check, or an `unsupported_version`
/ `unknown_operation` reply. In all of those the agent provably did nothing.

Once a side-effecting request has been written, the client does **not** fall
back: if the connection breaks, it reports the error. The restore lock would
stop a double restore anyway, but a silent second attempt is still wrong.

The in-process path calls the same `cf_agent::Service::handle` the agent runs,
so both paths produce the same result and summary.

* **`contextflow-launch`** (slice 1, done): sends `restore` for the icon's
  workspace, shows the returned problems in its message box, does nothing on
  `busy`, and falls back in-process when there is no agent.
* **`contextflow`** (slice 2, done for `restore` and `leave`): both go through
  the same helper and print the same output whichever side did the work.
  `restore --dry-run` only reads, so it always runs in-process. `park` and
  `checkpoint` follow. Read-only commands (`list`, `show`, `history`) keep
  reading SQLite directly until the agent owns the only connection.
* **Tauri app** (after that): the tray process becomes a client. Commands that
  write call the agent; the app subscribes to agent events (restore progress,
  workspace changed) over a second, long-lived connection instead of polling.
  The app starts the agent if it is not running.

## Lifecycle

* **Start.** `contextflow-agent.exe` is a windowless program.
  `contextflow agent start` starts it detached (found next to `contextflow.exe`
  or on `PATH`) and waits until it answers. Next, the installer registers it
  to run at sign-in (the Run key, the same place the tray app uses), and any
  client that finds no agent may start it in the background *after* finishing
  its own request in-process, so the next one is fast. Only one agent per
  user and session can bind the pipe.
* **Upgrade.** `contextflow agent stop` (and later the installer) sends
  `shutdown` (only the user's own processes can), waits until the pipe is
  gone, then the binaries can be replaced and the new agent started.
  `contextflow agent status` says when a running agent is too old for this
  build's requests. Clients newer than a still-running old agent get
  `unsupported_version` / `unknown_operation` and fall back, so a half-finished
  upgrade degrades to today's behaviour, never to failure. `hello` reports the
  agent's build version so the tray app can restart a stale agent.
* **Failure.** If the agent crashes, the OS frees the restore mutex and the
  pipe; clients find no pipe and work in-process. A request in flight when it
  dies returns an error to that client only. The agent logs to the same daily
  log files as the other programs. A watchdog/restart policy is a later slice.
* **Shutdown.** `shutdown` replies `shutting_down`. The agent then finishes
  the requests it is answering, and until they are done it keeps the pipe,
  still answers `hello` and turns other requests away with `shutting_down`
  (which clients treat as "do it yourself"). Then it removes the pipe and
  exits. Idle connections never keep it alive. `hello` and `shutdown` never
  wait for the database, so `agent status` and `agent stop` answer at once
  even in the middle of a restore.
* **Installer.** The agent ships next to the other programs. Before
  installing or uninstalling, the installer runs `contextflow agent stop` and
  then ends any agent still running from the install folder, so files are
  never locked and an old agent never outlives an upgrade. It is not started
  automatically yet.
* **Working folder and rights.** The agent switches to the user's profile
  folder at start (programs it restores inherit it, and the folder it was
  started from is not kept locked). Run it as the ordinary user, not as
  administrator.

## Next slices

1. ~~`contextflow restore`/`leave` through the agent with the same fallback;
   `contextflow agent start|status|stop`.~~ Done.
2. Start the agent at sign-in, and from a client that found none;
   stale-agent restart from the tray app. (Bundling it and stopping it from
   the installer is done.)
3. `park`, `resume` and `checkpoint` through the agent; streamed progress
   events instead of lines returned at the end.
4. Move checkpoints/autosave and global shortcuts from the tray app into the agent.
5. Tauri app as a client, with an event subscription for restore progress.
6. The agent owns the only database connection; front ends stop opening SQLite.
7. Then the live, event-driven desktop model and staged restore build on it.
