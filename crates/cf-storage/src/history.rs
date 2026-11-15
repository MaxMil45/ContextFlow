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

/// Restore runs kept per snapshot; older ones are deleted as new ones arrive.
const MAX_RUNS_PER_SNAPSHOT: i64 = 50;

/// One past restore of a workspace, with what went wrong in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreRunRecord {
    pub started_at: DateTime<Utc>,
    pub version: u32,
    pub overall_status: String,
    pub steps: usize,
    /// (program, status, message) of every step that didn't succeed.
    pub problems: Vec<(String, String, Option<String>)>,
}

/// How long each phase of one restore took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreTiming {
    pub workspace: String,
    pub started_at: DateTime<Utc>,
    /// (phase, milliseconds), in the order they ran.
    pub phases: Vec<(String, u64)>,
    /// Milliseconds from the start until the first window was in place.
    pub first_window_ms: Option<u64>,
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
        // Keep only the newest runs of this snapshot. The steps of the deleted
        // runs go with them (ON DELETE CASCADE). Ties on the timestamp fall
        // back to insertion order, so the run just added is never the one lost.
        tx.execute(
            "DELETE FROM restore_runs
             WHERE snapshot_id = ?1
               AND id NOT IN (
                   SELECT id FROM restore_runs WHERE snapshot_id = ?1
                   ORDER BY started_at DESC, rowid DESC LIMIT ?2)",
            params![snapshot_id.to_string(), MAX_RUNS_PER_SNAPSHOT],
        )?;
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

    /// The most recent restores of a workspace (any version), newest first.
    pub fn recent_restore_runs(
        &self,
        context_id: cf_core::ContextId,
        limit: u32,
    ) -> Result<Vec<RestoreRunRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.started_at, s.version_number, r.overall_status,
                    (SELECT COUNT(*) FROM restore_steps t
                     WHERE t.restore_run_id = r.id AND t.component_type = 'application')
             FROM restore_runs r JOIN context_snapshots s ON s.id = r.snapshot_id
             WHERE s.context_id = ?1 ORDER BY r.started_at DESC LIMIT ?2",
        )?;
        let runs = stmt
            .query_map(params![context_id.to_string(), limit], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u32>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut problems = self.conn.prepare(
            "SELECT component_identifier, status, error_message FROM restore_steps
             WHERE restore_run_id = ?1 AND component_type = 'application'
               AND status NOT IN ('success', 'skipped') ORDER BY id",
        )?;
        let mut out = Vec::new();
        for (id, started, version, overall, steps) in runs {
            let started_at = DateTime::parse_from_rfc3339(&started)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|e| crate::StorageError::Integrity(format!("restore run time: {e}")))?;
            let list = problems
                .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            out.push(RestoreRunRecord {
                started_at,
                version,
                overall_status: overall,
                steps: steps as usize,
                problems: list,
            });
        }
        Ok(out)
    }

    /// Where the time went in the most recent restore that recorded it. The
    /// phases are the run's steps with `component_type` "phase"; the first
    /// window is the "milestone" step `first_window`.
    pub fn last_restore_timing(&self) -> Result<Option<RestoreTiming>> {
        use rusqlite::OptionalExtension as _;
        let run = self
            .conn
            .query_row(
                "SELECT r.id, r.started_at, c.name
                 FROM restore_runs r
                 JOIN context_snapshots s ON s.id = r.snapshot_id
                 JOIN contexts c ON c.id = s.context_id
                 WHERE EXISTS (SELECT 1 FROM restore_steps t
                               WHERE t.restore_run_id = r.id AND t.component_type = 'phase')
                 ORDER BY r.started_at DESC, r.rowid DESC LIMIT 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, started, workspace)) = run else {
            return Ok(None);
        };
        let started_at = DateTime::parse_from_rfc3339(&started)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|e| crate::StorageError::Integrity(format!("restore run time: {e}")))?;
        let phases = self
            .conn
            .prepare(
                "SELECT component_identifier, COALESCE(duration_ms, 0) FROM restore_steps
                 WHERE restore_run_id = ?1 AND component_type = 'phase' ORDER BY id",
            )?
            .query_map([&id], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let first_window_ms = self
            .conn
            .query_row(
                "SELECT duration_ms FROM restore_steps WHERE restore_run_id = ?1
                 AND component_type = 'milestone' AND component_identifier = 'first_window'",
                [&id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .map(|ms| ms as u64);
        Ok(Some(RestoreTiming {
            workspace,
            started_at,
            phases,
            first_window_ms,
        }))
    }
}
