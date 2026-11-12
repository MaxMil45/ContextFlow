use cf_core::CoreError;
use thiserror::Error;

pub type Result<T, E = StorageError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Core(#[from] CoreError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    Conflict(String),

    #[error("integrity check failed: {0}")]
    Integrity(String),

    #[error("database schema version {found} is newer than this build supports ({supported}); upgrade ContextFlow")]
    UnsupportedSchema { found: i64, supported: i64 },
}
