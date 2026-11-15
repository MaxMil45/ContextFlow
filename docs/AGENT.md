# The pipe between the front ends and the app

While the ContextFlow desktop app is running (in the tray, with or without a
window), it serves a named pipe. The desktop icon (`contextflow-launch`) and
`contextflow restore` / `leave` hand their work to it there; when the app is
not running they do the same work themselves. Both paths run one handler,
`cf_app::service::handle`, so the result is the same either way.

This note is the protocol and its security. The code is in `cf-ipc` (protocol,
pipe, access checks) and `cf_app::service` (the handler and the client helper).
The pipe and protocol names still say "agent": an earlier version ran a
separate `contextflow-agent` program, and the names are kept so the protocol
did not change.

The app serves the pipe only when it uses the per-user database and config.
Started with `CONTEXTFLOW_DB` or `CONTEXTFLOW_CONFIG`, it does not serve it,
and clients given `--db` or `--config` never ask it.

## The channel

A **named pipe per user and per login session**:

```
\\.\pipe\ContextFlow.Agent.<user SID>.<session id>
```

Pipes live in one machine-wide namespace, so the name carries the user's SID
(two users never collide) and the Windows session id (the same user signed in
on the console and over RDP gets two pipes, each driving its own desktop).

### Access check

The name is not a secret, so it is not the security boundary. These are:

1. **DACL.** The app creates every pipe instance with the security
   descriptor `O:<SID>D:P(A;;GA;;;<SID>)`: owned by the user, a protected DACL
   with one entry granting the user full access, nothing inherited. Other
   users, including administrators without taking ownership, cannot open it
   or create instances of it. Objects with no explicit label get the default
   *medium, no-write-up* mandatory label, so low-integrity processes of the same
   user (browser sandboxes, AppContainers) cannot write to it either.
2. **No remote clients.** `PIPE_REJECT_REMOTE_CLIENTS` refuses SMB connections.
3. **No squatting.** The first instance is created with
   `FILE_FLAG_FIRST_PIPE_INSTANCE`; if the name already exists the app does not
   serve it (and logs why). Further instances need `FILE_CREATE_PIPE_INSTANCE`,
   which the DACL gives only to the user.
4. **Server checks the client.** After the first request arrives the app
   impersonates the client (`ImpersonateNamedPipeClient`), reads the token's
   user SID, reverts, and also reads the client's session id
   (`GetNamedPipeClientSessionId`). Both must match the app's own, or the
   connection is answered `denied` and closed. This holds even if the DACL
   were ever loosened. An app running as administrator also refuses a
   client that is not: it starts every program it restores with its own
   rights, so answering would hand those rights to an ordinary process. The
   refused client does the work itself, with its own rights.
5. **Client checks the server.** Clients open the pipe with
   `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, so the app can only
   *identify* them, never act as them. The client then reads the pipe's owner
   SID (`GetSecurityInfo`) and the server's session id, and talks to it only if
   both are its own. A pipe planted by someone else is treated as "app not running".

Within one user and session there is no further boundary: any process the
user runs at medium integrity can already do everything ContextFlow does. The
app still keeps the existing in-band rules: imported workspaces must be
approved before they restore, and a request names a workspace, never a path,
program or command line. The app never opens a database or config path that
a client sends; a client started with `--db` or `--config` restores in-process
instead of asking the app.

### Protocol

Framing: a 4-byte little-endian length, then that many bytes of UTF-8 JSON.
Frames over 1 MiB are refused. A connection carries any number of
request/response pairs in order; either side may close it.

```json
→ {"v":2,"id":7,"op":"restore","workspace":"Client A","only":["code"]}
← {"v":2,"id":7,"status":"restored","workspace":"Client A","version":12,"overall":"success","steps":[…],…}
```

* `v` is the protocol version of the sender. The app answers any version in
  `MIN_SUPPORTED..=PROTOCOL_VERSION`; anything else gets an
  `unsupported_version` error carrying both bounds, before anything happens.
* `id` is echoed so a client can match replies.
* `op` names the request: `hello`, `restore`, `leave`, `shutdown`. An op the
  app does not know gets `unknown_operation`, again before anything happens.
* `status` names the reply: `hello` (app version and protocol range),
  `restored` (workspace, version, per-application steps, what was saved and
  closed first, problems, progress lines), `left`, `busy` (another restore
  holds the lock), `shutting_down`, `error` (`code` + `message`).

New fields are added with serde defaults so older peers ignore them; a change
that older peers cannot ignore bumps `PROTOCOL_VERSION`.

| Version | Adds |
| --- | --- |
| v1 | `hello`, `restore` (workspace, version), `shutdown` |
| v2 | `restore` options `only`, `timeout_secs`, `no_cleanup`, `no_save` (an app that ignored them would do the wrong restore, hence the bump); `leave`; fuller summaries |

The app answers v1 and v2.

## Clients and fallback

Every client follows one rule: **if the app cannot take the request, do it
in-process.** "Cannot" means: no pipe, the pipe busy past a short wait, the
server failing the identity check, or an `unsupported_version` /
`unknown_operation` reply. In all of those the app provably did nothing.

Once a side-effecting request has been written, the client does **not** fall
back: if the connection breaks, it reports the error. The restore lock
(`Local\ContextFlow.Restore`) would stop a double restore anyway, but a silent
second attempt is still wrong. The app's own restores (the switcher, the
dashboard) take the same lock, so they and a pipe request never run at once.

* **`contextflow-launch`** sends `restore` for the icon's workspace, shows the
  returned problems in its message box, and does nothing on `busy`.
* **`contextflow`** sends `restore` and `leave` and prints the same output
  whichever side did the work. `restore --dry-run` only reads, so it always
  runs in-process. Other commands read SQLite directly.

Progress lines are not streamed to the client yet: the app returns them in
the summary and the client prints them afterwards. In-process they are shown
live. While the app serves a switch it shows them as they happen on its
on-screen card, the same one the switcher uses, ending with the result.

If the app quits or crashes, the OS frees the pipe and the restore lock, and
clients work in-process. A request in flight returns an error to that client
only. `shutdown` stops the app serving the pipe (not the app itself) until it
next starts.
