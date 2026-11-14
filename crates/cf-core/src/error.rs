use thiserror::Error;

pub type Result<T, E = CoreError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid name: {0}")]
    InvalidName(String),

    #[error("invalid snapshot payload: {0}")]
    InvalidPayload(String),

    #[error("invalid cleanup rule: {0}")]
    InvalidCleanup(String),

    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}
