//! The agent's side of the pipe: accept clients, check who they are, and
//! answer their requests in order, one thread per connection.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
}

impl StopHandle {
    pub fn stop(&self) {
        if !self.stop.swap(true, Ordering::SeqCst) {
            // Wake the accept loop with a connection that sends nothing.
            #[cfg(windows)]
            let _ = crate::pipe::open(&self.name, std::time::Duration::from_secs(1));
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

pub struct Server {
    #[cfg(windows)]
    listener: crate::pipe::Listener,
    own: Identity,
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
                own,
                control: StopHandle {
                    name: name.to_owned(),
                    stop: Arc::default(),
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
    /// `shutting_down`), then wait for in-flight requests to finish.
    pub fn run(self, handler: impl Handler) -> io::Result<()> {
        #[cfg(windows)]
        {
            let Server {
                mut listener,
                own,
                control,
            } = self;
            let handler: Arc<dyn Handler> = Arc::new(handler);
            let own = Arc::new(own);
            let mut workers: Vec<std::thread::JoinHandle<()>> = Vec::new();
            while !control.is_stopped() {
                let pipe = match listener.accept() {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("agent pipe: accept failed: {e}");
                        continue;
                    }
                };
                if control.is_stopped() {
                    break;
                }
                workers.retain(|w| !w.is_finished());
                let (handler, own, control) = (handler.clone(), own.clone(), control.clone());
                workers.push(std::thread::spawn(move || {
                    if let Err(e) = serve_connection(pipe, &own, &*handler, &control) {
                        tracing::debug!("agent pipe: connection ended: {e}");
                    }
                }));
            }
            for w in workers {
                let _ = w.join();
            }
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
                .and_then(|peer| crate::check_peer(own, &peer).map_err(|e| e.to_string()));
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
            Ok(req) => Response::new(req.id, handler.handle(req.body)),
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
