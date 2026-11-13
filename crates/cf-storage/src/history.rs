//! Restore history: one `restore_runs` row per restore, one `restore_steps`
//! row per application. Written after the fact; never read by the engine.

use chrono::{DateTime, Utc};
use rusqlite::params;

use cf_core::SnapshotId;

use crate::repo::ts;
use crate::{Database, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreStepRecord {
    pub component_type: String,
    pub component_identifier: String,
    pub status: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreRunSummary {
    pub started_at: DateTime<Utc>,
    pub overall_status: String,
}

impl Database {
    /// Store a finished restore run and its steps atomically.
    pub fn record_restore_run(
        &mut self,
        snapshot_id: SnapshotId,
        started_at: DateTime<Utc>,
        completed_at: DateTime<Utc>,
        overall_status: &str,
        steps: &[RestoreStepRecord],
    ) -> Result<()> {
        let run_id = uuid::Uuid::new_v4().to_string();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO restore_runs (id, snapshot_id, started_at, completed_at, overall_status)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                run_id,
                snapshot_id.to_string(),
                ts(&started_at),
                ts(&completed_at),
                overall_status
            ],
        )?;
        for s in steps {
            tx.execute(
                "INSERT INTO restore_steps
                 (restore_run_id, component_type, component_identifier, status,
                  error_code, error_message, duration_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    run_id,
                    s.component_type,
                    s.component_identifier,
                    s.status,
                    s.error_code,
                    s.error_message,
                    s.duration_ms as i64
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Most recent restore runs of a snapshot, newest first.
    pub fn restore_runs(&self, snapshot_id: SnapshotId) -> Result<Vec<RestoreRunSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT started_at, overall_status FROM restore_runs
             WHERE snapshot_id = ?1 ORDER BY started_at DESC LIMIT 20",
        )?;
        let rows = stmt.query_map([snapshot_id.to_string()], |r| {
            let s: String = r.get(0)?;
            Ok(RestoreRunSummary {
                started_at: DateTime::parse_from_rfc3339(&s)
                    .map(|d| d.with_timezone(&Utc))
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                overall_status: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
