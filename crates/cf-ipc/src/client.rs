//! A front end's side of the pipe.

use std::time::Duration;

use thiserror::Error;

#[cfg(windows)]
use crate::protocol::{read_frame, write_message};
use crate::protocol::{Request, RequestBody, Response, ResponseBody};

/// Why a call did not produce an answer.
#[derive(Debug, Error)]
pub enum CallError {
    /// The agent did not take the request: none is running, it is busy past
    /// the wait, it failed the identity check, or it refused the request
    /// before acting. Doing the work in-process instead is safe.
    #[error("no agent available: {0}")]
    Unavailable(String),
    /// The request may have reached the agent and the reply was lost. Do not
    /// retry in-process; report it.
    #[error("the agent did not answer: {0}")]
    Failed(String),
}

/// An open, verified connection to the agent.
pub struct Client {
    #[cfg(windows)]
    pipe: std::fs::File,
    next_id: u64,
}

impl Client {
    /// Connect to the agent on `name`, waiting up to `wait` if it is busy, and
    /// make sure it runs as this user in this session.
    pub fn connect(name: &str, wait: Duration) -> Result<Self, CallError> {
        #[cfg(windows)]
        {
            use crate::pipe::{open, server_identity, OpenError};
            let pipe = open(name, wait).map_err(|e| {
                CallError::Unavailable(match e {
                    OpenError::NotFound => "not running".into(),
                    OpenError::Busy => "busy".into(),
                    OpenError::Io(e) => e.to_string(),
                })
            })?;
            let own =
                crate::Identity::current().map_err(|e| CallError::Unavailable(e.to_string()))?;
            let server =
                server_identity(&pipe).map_err(|e| CallError::Unavailable(e.to_string()))?;
            crate::check_peer(&own, &server)
                .map_err(|e| CallError::Unavailable(format!("refusing the pipe's server: {e}")))?;
            Ok(Self { pipe, next_id: 1 })
        }
        #[cfg(not(windows))]
        {
            let _ = (name, wait);
            Err(CallError::Unavailable(
                "the agent channel needs Windows".into(),
            ))
        }
    }

    /// Send one request and wait for its reply. Error replies that prove the
    /// agent did nothing come back as [`CallError::Unavailable`].
    pub fn call(&mut self, body: RequestBody) -> Result<ResponseBody, CallError> {
        self.call_as(crate::PROTOCOL_VERSION, body)
    }

    /// [`Client::call`], declaring an older protocol `version`. For requests
    /// that have not changed since then (`hello`, `shutdown`), so an agent of
    /// any age answers them.
    pub fn call_as(&mut self, version: u32, body: RequestBody) -> Result<ResponseBody, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        let side_effects = body.has_side_effects();
        // Once a side-effecting request may have been sent, a lost reply is a
        // failure, never a reason to do the work again in-process.
        let lost = |what: String| {
            if side_effects {
                CallError::Failed(what)
            } else {
                CallError::Unavailable(what)
            }
        };
        let response = self
            .exchange(&Request {
                v: version,
                id,
                body,
            })
            .map_err(lost)?;
        if response.id != id {
            return Err(CallError::Failed(format!(
                "reply {} does not match request {id}",
                response.id
            )));
        }
        match response.body {
            ResponseBody::Error(e) if e.code.safe_to_fall_back() => Err(CallError::Unavailable(
                format!("agent refused the request: {}", e.message),
            )),
            body => Ok(body),
        }
    }

    #[cfg(windows)]
    fn exchange(&mut self, request: &Request) -> Result<Response, String> {
        write_message(&mut self.pipe, request).map_err(|e| e.to_string())?;
        let frame = read_frame(&mut self.pipe)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "the agent closed the connection".to_string())?;
        serde_json::from_slice(&frame).map_err(|e| format!("unreadable reply: {e}"))
    }

    #[cfg(not(windows))]
    fn exchange(&mut self, _request: &Request) -> Result<Response, String> {
        Err("the agent channel needs Windows".into())
    }
}

/// Connect, send one request and return the reply.
pub fn request(name: &str, body: RequestBody, wait: Duration) -> Result<ResponseBody, CallError> {
    Client::connect(name, wait)?.call(body)
}
