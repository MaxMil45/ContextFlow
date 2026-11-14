//! The agent's side of the pipe: accept clients, check who they are, and
//! answer their requests in order, one thread per connection.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use crate::protocol::{decode_request, read_frame, write_message, ErrorCode, Response};
use crate::protocol::{RequestBody, ResponseBody};
use crate::Identity;

/// Answers requests. Runs on the connection's thread; may block (a restore
/// takes seconds).
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, request: RequestBody) -> ResponseBody;
}

impl<F> Handler for F
where
    F: Fn(RequestBody) -> ResponseBody + Send + Sync + 'static,
{
    fn handle(&self, request: RequestBody) -> ResponseBody {
        self(request)
    }
}

/// Stops a running [`Server`] from another thread.
#[derive(Clone)]
pub struct StopHandle {
    name: String,
    stop: Arc<AtomicBool>,
    /// Requests being answered right now.
    active: Arc<Mutex<usize>>,
}

impl StopHandle {
    /// Stop taking new work. The server exits once the requests it is
    /// answering have finished; until then it still answers `hello`.
    pub fn stop(&self) {
        if !self.stop.swap(true, Ordering::SeqCst) {
            self.wake();
        }
    }

    /// Wake the accept loop with a connection that sends nothing.
    fn wake(&self) {
        #[cfg(windows)]
        let _ = crate::pipe::open(&self.name, std::time::Duration::from_secs(1));
    }

    fn active(&self) -> usize {
        *self.active.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Count a request as in flight until the guard is dropped (also when the
    /// handler panics).
    #[cfg(windows)]
    fn begin(&self) -> InFlight<'_> {
        *self.active.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        InFlight(self)
    }

    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

#[cfg(windows)]
struct InFlight<'a>(&'a StopHandle);

#[cfg(windows)]
impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let left = {
            let mut n = self.0.active.lock().unwrap_or_else(|p| p.into_inner());
            *n -= 1;
            *n
        };
        // The last request of a stopping server: let the accept loop see it.
        if left == 0 && self.0.is_stopped() {
            self.0.wake();
        }
    }
}

pub struct Server {
    #[cfg(windows)]
    listener: crate::pipe::Listener,
    own: Identity,
    own_elevated: bool,
    control: StopHandle,
}

impl Server {
    /// Take the pipe `name`. Fails with `AddrInUse` if it already exists, so
    /// only one agent per user and session can run and nobody can squat it.
    pub fn bind(name: &str) -> io::Result<Self> {
        let own = Identity::current()?;
        #[cfg(windows)]
        {
            let listener = crate::pipe::Listener::bind(name, &own)?;
            Ok(Self {
                listener,
                own_elevated: crate::is_elevated()?,
                own,
                control: StopHandle {
                    name: name.to_owned(),
                    stop: Arc::default(),
                    active: Arc::default(),
                },
            })
        }
        #[cfg(not(windows))]
        {
            let _ = (name, own);
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    pub fn stop_handle(&self) -> StopHandle {
        self.control.clone()
    }

    /// Serve until stopped (by [`StopHandle::stop`] or a handler answering
    /// `shutting_down`) and the requests in flight have finished. While
    /// finishing them it keeps the pipe, answers `hello` and turns other
    /// requests away, so a client can tell "stopping" from "gone".
    pub fn run(self, handler: impl Handler) -> io::Result<()> {
        #[cfg(windows)]
        {
            let Server {
                mut listener,
                own,
                own_elevated,
                control,
            } = self;
            let handler: Arc<dyn Handler> = Arc::new(handler);
            let own = Arc::new(own);
            loop {
                if control.is_stopped() && control.active() == 0 {
                    break;
                }
                let pipe = match listener.accept() {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("agent pipe: accept failed: {e}");
                        // Never spin on a persistent failure.
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        continue;
                    }
                };
                if control.is_stopped() && control.active() == 0 {
                    break;
                }
                // Connections are not joined: an idle client holding one open
                // must never keep a stopping agent alive.
                let (handler, own, control) = (handler.clone(), own.clone(), control.clone());
                std::thread::spawn(move || {
                    if let Err(e) = serve_connection(pipe, &own, own_elevated, &*handler, &control)
                    {
                        tracing::debug!("agent pipe: connection ended: {e}");
                    }
                });
            }
            // Closing the listener removes the pipe: clients now find no agent.
            drop(listener);
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = handler;
            Err(io::ErrorKind::Unsupported.into())
        }
    }
}

#[cfg(windows)]
fn serve_connection(
    mut pipe: std::fs::File,
    own: &Identity,
    own_elevated: bool,
    handler: &dyn Handler,
    control: &StopHandle,
) -> io::Result<()> {
    let mut checked = false;
    while let Some(frame) = read_frame(&mut pipe)? {
        // Identify the client once it has written something (impersonation
        // needs that), before acting on anything it sent.
        if !checked {
            let verdict = crate::pipe::client_identity(&pipe)
                .map_err(|e| e.to_string())
                .and_then(|(peer, peer_elevated)| {
                    crate::check_client(own, own_elevated, &peer, peer_elevated)
                        .map_err(|e| e.to_string())
                });
            if let Err(why) = verdict {
                tracing::warn!("agent pipe: denied a client: {why}");
                let id = decode_request(&frame).map(|r| r.id).unwrap_or(0);
                write_message(
                    &mut pipe,
                    &Response::new(id, ResponseBody::error(ErrorCode::Denied, why)),
                )?;
                return Ok(());
            }
            checked = true;
        }
        let response = match decode_request(&frame) {
            // A stopping server finishes what it has but starts nothing new.
            Ok(req)
                if control.is_stopped()
                    && !matches!(req.body, RequestBody::Hello { .. } | RequestBody::Shutdown) =>
            {
                Response::new(
                    req.id,
                    ResponseBody::error(ErrorCode::ShuttingDown, "the agent is shutting down"),
                )
            }
            Ok(req) => {
                let _in_flight = control.begin();
                Response::new(req.id, handler.handle(req.body))
            }
            Err(reply) => *reply,
        };
        let stopping = matches!(response.body, ResponseBody::ShuttingDown);
        write_message(&mut pipe, &response)?;
        if stopping {
            control.stop();
            return Ok(());
        }
    }
    Ok(())
}
