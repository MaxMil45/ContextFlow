//! The one error type every command returns. It crosses the IPC boundary as
//! `{ code, message }` so the UI can branch on `code` and show `message`.

use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/bindings/")]
pub enum ErrorCode {
    NotFound,
    Conflict,
    RestoreBusy,
    /// A file the command would create is already there (e.g. a desktop shortcut).
    AlreadyExists,
    Internal,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

fn code_for(e: &cf_storage::StorageError) -> ErrorCode {
    use cf_storage::StorageError as E;
    match e {
        E::NotFound(_) => ErrorCode::NotFound,
        E::Conflict(_) => ErrorCode::Conflict,
        _ => ErrorCode::Internal,
    }
}

impl From<cf_storage::StorageError> for CommandError {
    fn from(e: cf_storage::StorageError) -> Self {
        Self::new(code_for(&e), e.to_string())
    }
}

impl From<anyhow::Error> for CommandError {
    fn from(e: anyhow::Error) -> Self {
        let code = if e.downcast_ref::<cf_cli::session::RestoreBusy>().is_some() {
            ErrorCode::RestoreBusy
        } else if e
            .downcast_ref::<cf_cli::shortcut::ShortcutExists>()
            .is_some()
        {
            ErrorCode::AlreadyExists
        } else if let Some(s) = e.downcast_ref::<cf_storage::StorageError>() {
            code_for(s)
        } else {
            ErrorCode::Internal
        };
        if code == ErrorCode::Internal {
            tracing::error!("{e:#}");
        }
        Self::new(code, format!("{e:#}"))
    }
}

pub type CmdResult<T> = Result<T, CommandError>;
