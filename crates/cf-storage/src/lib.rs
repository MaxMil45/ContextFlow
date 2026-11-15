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
mod export;
mod history;
mod migrations;
mod repo;
mod timeline;

pub use error::{Result, StorageError};
pub use export::{
    ExportFile, ExportedSnapshot, ExportedWorkspace, EXPORT_FORMAT, EXPORT_FORMAT_VERSION,
};
pub use history::{RestoreRunRecord, RestoreRunSummary, RestoreStepRecord, RestoreTiming};
pub use repo::{
    ContextOverview, Database, IntegrityReport, RetentionPolicy, SaveMode, SnapshotOrigin,
    AUTO_SAVES_KEPT,
};
pub use timeline::{EventKind, EventRecord};
