//! SQLite persistence for ContextFlow.
//!
//! * WAL journal, foreign keys enforced, parameterised SQL only.
//! * Versioned migrations tracked in `PRAGMA user_version`.
//! * Snapshots are written in a single IMMEDIATE transaction and are immutable.
//!
//! `rusqlite` is synchronous. Async callers (the Tauri app, the restore engine)
//! must call into this crate from a dedicated database thread or
//! `tokio::task::spawn_blocking`, never directly on the async executor.

mod error;
mod migrations;
mod repo;

pub use error::{Result, StorageError};
pub use repo::{Database, IntegrityReport};
