//! The local channel between the ContextFlow agent and its front ends.
//!
//! * [`protocol`]: versioned request/response types and framing (portable).
//! * [`access`]: who may talk to whom.
//! * [`server`] / [`client`]: the per-user, per-session named pipe (Windows).
//!
//! See `docs/AGENT.md` for the design.

pub mod access;
pub mod client;
#[cfg(windows)]
mod pipe;
pub mod protocol;
pub mod server;

pub use access::{check_client, check_peer, is_elevated, AccessDenied, Identity};
pub use client::{request, CallError, Client};
pub use protocol::*;
pub use server::{Handler, Server, StopHandle};

/// The pipe the agent of this user and login session listens on.
pub fn default_pipe_name() -> std::io::Result<String> {
    Ok(pipe_name_for(&Identity::current()?))
}

/// `\\.\pipe\ContextFlow.Agent.<SID>.<session>`: pipes share one machine-wide
/// namespace, so the name separates users and their login sessions.
pub fn pipe_name_for(id: &Identity) -> String {
    format!(r"\\.\pipe\ContextFlow.Agent.{}.{}", id.sid, id.session)
}
