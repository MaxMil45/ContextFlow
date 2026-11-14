# The ContextFlow agent

**Status:** design note for Phase 2's headless agent. The first slice is built:
`cf-ipc` (protocol and pipe transport), `cf-agent` (the `contextflow-agent`
program and its request handler) and `contextflow-launch` sending its restore
to a running agent. Everything else here is the plan for the next slices.

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
   were ever loosened.
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
→ {"v":1,"id":7,"op":"restore","workspace":"Client A"}
← {"v":1,"id":7,"status":"restored","workspace":"Client A","problems":[],"layout_note":null}
```

* `v` is the protocol version of the sender. The agent answers any version in
  `MIN_SUPPORTED..=PROTOCOL_VERSION`; anything else gets an
  `unsupported_version` error carrying both bounds, before anything happens.
* `id` is echoed so a client can match replies.
* `op` names the request: `hello`, `restore`, `shutdown` in v1. An op the agent
  does not know gets `unknown_operation`, again before anything happens.
* `status` names the reply: `hello` (agent version and protocol range),
  `restored` (workspace name, problems, layout note), `busy` (another restore
  holds the lock), `shutting_down`, `error` (`code` + `message`).

New fields are added with serde defaults so older peers ignore them; a change
that older peers cannot ignore bumps `PROTOCOL_VERSION`.

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
* **`contextflow`** (next): `restore`, `switch`, `leave`, `park`, `checkpoint`
  go through the same helper. Read-only commands (`list`, `show`, `history`)
  can keep reading SQLite directly until the agent owns the only connection.
* **Tauri app** (after that): the tray process becomes a client. Commands that
  write call the agent; the app subscribes to agent events (restore progress,
  workspace changed) over a second, long-lived connection instead of polling.
  The app starts the agent if it is not running.

## Lifecycle

* **Start.** `contextflow-agent.exe` is a windowless program. Slice 1 starts it
  by hand. Next, the installer registers it to run at sign-in (the Run key, the
  same place the tray app uses), and any client that finds no agent may start
  it in the background *after* finishing its own request in-process, so the
  next one is fast. Only one agent per user and session can bind the pipe.
* **Upgrade.** The installer sends `shutdown` (only the user's own processes
  can), waits for the process to exit, replaces the binaries and starts the
  new agent. Clients newer than a still-running old agent get
  `unsupported_version` / `unknown_operation` and fall back, so a half-finished
  upgrade degrades to today's behaviour, never to failure. `hello` reports the
  agent's build version so the tray app can restart a stale agent.
* **Failure.** If the agent crashes, the OS frees the restore mutex and the
  pipe; clients find no pipe and work in-process. A request in flight when it
  dies returns an error to that client only. The agent logs to the same daily
  log files as the other programs. A watchdog/restart policy is a later slice.
* **Shutdown.** `shutdown` replies `shutting_down`, stops accepting
  connections and exits once in-flight requests finish.

## Next slices

1. `contextflow restore`/`switch`/`leave` through the agent with the same fallback.
2. Start the agent at sign-in, and from a client that found none; `shutdown`
   from the installer; `hello`-based stale-agent restart from the tray app.
3. Move checkpoints/autosave and global shortcuts from the tray app into the agent.
4. Tauri app as a client, with an event subscription for restore progress.
5. The agent owns the only database connection; front ends stop opening SQLite.
6. Then the live, event-driven desktop model and staged restore build on it.
