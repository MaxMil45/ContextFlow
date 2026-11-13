//! The integration contract every application adapter implements.
//!
//! The Context Engine only ever talks to applications through this trait, so
//! new integrations (JetBrains, Docker Desktop, ...) never require engine
//! changes. Adapters declare what they can do through [`AdapterCapabilities`];
//! the engine must not call an operation an adapter does not advertise.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type Result<T, E = AdapterError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("operation '{0}' is not supported by this adapter")]
    Unsupported(&'static str),
    #[error("application is not available: {0}")]
    Unavailable(String),
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("adapter state has schema version {found}, expected {expected}")]
    SchemaMismatch { found: u32, expected: u32 },
    #[error("{0}")]
    Other(String),
}

/// What an adapter can do. Used for capability negotiation and for the UI to
/// explain honestly what will and will not be restored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterCapabilities {
    pub discover: bool,
    pub capture: bool,
    pub validate: bool,
    pub restore: bool,
    pub verify: bool,
    /// Can the adapter tell whether the application has unsaved work?
    pub detect_unsaved: bool,
    /// Can the adapter ask the application to close gracefully?
    pub graceful_close: bool,
}

/// A running instance of an application the adapter recognised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplicationInstance {
    pub adapter_id: String,
    pub process_id: u32,
    /// Process start time (FILETIME ticks) — with the PID this forms a
    /// reuse-safe process identity.
    pub process_start_time: Option<u64>,
    pub executable_path: String,
    /// Adapter-specific discovery details.
    #[serde(default)]
    pub details: serde_json::Value,
}

/// Versioned, adapter-owned state stored inside a snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterSnapshot {
    pub adapter_id: String,
    pub schema_version: u32,
    pub state: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Blocking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn is_restorable(&self) -> bool {
        !self.issues.iter().any(|i| i.severity == Severity::Blocking)
    }
}

/// Outcome of one restoration task (spec §4.4, stage 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreStatus {
    Success,
    PartialSuccess,
    Failed,
    Skipped,
    RequiresUserIntervention,
}

impl RestoreStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PartialSuccess => "partial_success",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::RequiresUserIntervention => "requires_user_intervention",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreReport {
    pub status: RestoreStatus,
    pub message: Option<String>,
    /// Process ids launched by this restore, for window identification.
    #[serde(default)]
    pub launched_process_ids: Vec<u32>,
}

#[async_trait]
pub trait ContextAdapter: Send + Sync {
    /// Stable identifier, e.g. `vscode`, `git`, `chromium`, `windows-terminal`.
    fn id(&self) -> &'static str;

    fn capabilities(&self) -> AdapterCapabilities;

    /// Upper bound for any single operation; the engine enforces it.
    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }

    async fn discover(&self) -> Result<Vec<ApplicationInstance>> {
        Err(AdapterError::Unsupported("discover"))
    }

    async fn capture(&self, _instance: &ApplicationInstance) -> Result<AdapterSnapshot> {
        Err(AdapterError::Unsupported("capture"))
    }

    async fn validate(&self, _snapshot: &AdapterSnapshot) -> Result<ValidationReport> {
        Ok(ValidationReport::default())
    }

    async fn restore(&self, _snapshot: &AdapterSnapshot) -> Result<RestoreReport> {
        Err(AdapterError::Unsupported("restore"))
    }

    async fn verify(&self, _snapshot: &AdapterSnapshot) -> Result<RestoreStatus> {
        Err(AdapterError::Unsupported("verify"))
    }
}

/// Registry of available adapters, keyed by id.
#[derive(Default, Clone)]
pub struct AdapterRegistry {
    adapters: BTreeMap<&'static str, Arc<dyn ContextAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an adapter. Returns `false` (and keeps the existing one) if an
    /// adapter with the same id is already registered.
    pub fn register(&mut self, adapter: Arc<dyn ContextAdapter>) -> bool {
        use std::collections::btree_map::Entry;
        match self.adapters.entry(adapter.id()) {
            Entry::Vacant(v) => {
                v.insert(adapter);
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn ContextAdapter>> {
        self.adapters.get(id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.adapters.keys().copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn ContextAdapter>> {
        self.adapters.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NotesAdapter;

    #[async_trait]
    impl ContextAdapter for NotesAdapter {
        fn id(&self) -> &'static str {
            "notes"
        }
        fn capabilities(&self) -> AdapterCapabilities {
            AdapterCapabilities {
                restore: true,
                ..Default::default()
            }
        }
        async fn restore(&self, _s: &AdapterSnapshot) -> Result<RestoreReport> {
            Ok(RestoreReport {
                status: RestoreStatus::Success,
                message: None,
                launched_process_ids: vec![],
            })
        }
    }

    #[tokio::test]
    async fn registry_and_default_methods() {
        let mut reg = AdapterRegistry::new();
        assert!(reg.register(Arc::new(NotesAdapter)));
        assert!(
            !reg.register(Arc::new(NotesAdapter)),
            "duplicate ids rejected"
        );
        assert_eq!(reg.ids().collect::<Vec<_>>(), vec!["notes"]);

        let a = reg.get("notes").unwrap();
        let snap = AdapterSnapshot {
            adapter_id: "notes".into(),
            schema_version: 1,
            state: serde_json::Value::Null,
        };
        assert_eq!(
            a.restore(&snap).await.unwrap().status,
            RestoreStatus::Success
        );
        assert!(matches!(
            a.discover().await,
            Err(AdapterError::Unsupported("discover"))
        ));
        assert!(a.validate(&snap).await.unwrap().is_restorable());
    }
}
