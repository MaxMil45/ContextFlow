//! Portable JSON export and verified import of workspaces.
//!
//! Each snapshot is exported with its canonical payload *bytes* and hash, so an
//! import can prove that nothing was altered in transit before writing anything.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use cf_core::{hash, validate, CaptureStatus, Context, ContextId, SnapshotId, SnapshotPayload};

use crate::repo::{insert_snapshot_row, is_unique_violation, now, ts};
use crate::{Database, Result, StorageError};

pub const EXPORT_FORMAT: &str = "contextflow-export";
pub const EXPORT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportFile {
    pub format: String,
    pub format_version: u32,
    pub exported_at: DateTime<Utc>,
    pub contextflow_version: String,
    pub workspaces: Vec<ExportedWorkspace>,
}

impl ExportFile {
    pub fn new(workspaces: Vec<ExportedWorkspace>, contextflow_version: &str) -> Self {
        Self {
            format: EXPORT_FORMAT.into(),
            format_version: EXPORT_FORMAT_VERSION,
            exported_at: now(),
            contextflow_version: contextflow_version.into(),
            workspaces,
        }
    }

    pub fn check_header(&self) -> Result<()> {
        if self.format != EXPORT_FORMAT {
            return Err(StorageError::Integrity(format!(
                "not a ContextFlow export (format '{}')",
                self.format
            )));
        }
        if self.format_version != EXPORT_FORMAT_VERSION {
            return Err(StorageError::Integrity(format!(
                "export format version {} is not supported (expected {EXPORT_FORMAT_VERSION})",
                self.format_version
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedWorkspace {
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub is_archived: bool,
    pub last_version: u32,
    pub snapshots: Vec<ExportedSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedSnapshot {
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub capture_status: CaptureStatus,
    pub hash: String,
    /// Canonical payload JSON exactly as stored; `hash` is its SHA-256.
    pub payload_json: String,
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| StorageError::Integrity(format!("bad timestamp '{s}': {e}")))
}

impl Database {
    pub fn export_context(&self, id: ContextId) -> Result<ExportedWorkspace> {
        type Row = (String, Option<String>, String, i64, u32);
        let row: Option<Row> = self
            .conn
            .query_row(
                "SELECT name, description, created_at, is_archived, last_version
                 FROM contexts WHERE id = ?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (name, description, created_at, archived, last_version) =
            row.ok_or_else(|| StorageError::NotFound(format!("no workspace with id {id}")))?;

        let mut stmt = self.conn.prepare(
            "SELECT version_number, created_at, capture_status, snapshot_hash, payload_json
             FROM context_snapshots WHERE context_id = ?1 ORDER BY version_number",
        )?;
        let rows = stmt.query_map([id.to_string()], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut snapshots = Vec::new();
        for row in rows {
            let (version, created, status, hash, payload_json) = row?;
            snapshots.push(ExportedSnapshot {
                version,
                created_at: parse_ts(&created)?,
                capture_status: status.parse().map_err(StorageError::Integrity)?,
                hash,
                payload_json,
            });
        }
        Ok(ExportedWorkspace {
            name,
            description,
            created_at: parse_ts(&created_at)?,
            is_archived: archived != 0,
            last_version,
            snapshots,
        })
    }

    /// Import a workspace with its full history. Every snapshot's hash and
    /// payload are verified *before* anything is written; any failure means
    /// nothing is imported.
    pub fn import_workspace(
        &mut self,
        ws: &ExportedWorkspace,
        name_override: Option<&str>,
    ) -> Result<Context> {
        let name = validate::context_name(name_override.unwrap_or(&ws.name))?;

        let mut seen = BTreeSet::new();
        let mut payloads: Vec<SnapshotPayload> = Vec::with_capacity(ws.snapshots.len());
        for s in &ws.snapshots {
            if s.version == 0 || !seen.insert(s.version) {
                return Err(StorageError::Integrity(format!(
                    "workspace '{}' has a zero or duplicate version number ({})",
                    ws.name, s.version
                )));
            }
            if hash::sha256_hex(s.payload_json.as_bytes()) != s.hash {
                return Err(StorageError::Integrity(format!(
                    "snapshot v{} of '{}' fails its hash check; the export file is corrupt or was edited",
                    s.version, ws.name
                )));
            }
            let payload: SnapshotPayload = serde_json::from_str(&s.payload_json).map_err(|e| {
                StorageError::Integrity(format!(
                    "snapshot v{} payload is unreadable: {e}",
                    s.version
                ))
            })?;
            validate::payload(&payload)?;
            payloads.push(payload);
        }
        let last_version = ws.last_version.max(seen.last().copied().unwrap_or(0));

        let id = ContextId::new();
        let updated_at = now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let res = tx.execute(
            "INSERT INTO contexts
             (id, name, description, created_at, updated_at, is_archived, last_version, is_trusted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
            params![
                id.to_string(),
                name,
                ws.description,
                ts(&ws.created_at),
                ts(&updated_at),
                ws.is_archived as i64,
                last_version
            ],
        );
        match res {
            Ok(_) => {}
            Err(e) if is_unique_violation(&e) => {
                let msg = format!("a workspace named '{name}' already exists");
                return Err(StorageError::Conflict(format!(
                    "{msg}; import it under another name with --as"
                )));
            }
            Err(e) => return Err(e.into()),
        }
        for s in ws.snapshots.iter() {
            let sid = SnapshotId::new();
            insert_snapshot_row(
                &tx,
                sid,
                id,
                s.version,
                &s.created_at,
                &s.hash,
                s.capture_status,
                &s.payload_json,
                "manual",
            )?;
        }
        tx.commit()?;
        tracing::info!(workspace = %name, snapshots = ws.snapshots.len(), "workspace imported");

        Ok(Context {
            id,
            name,
            description: ws.description.clone(),
            created_at: ws.created_at,
            updated_at,
            is_archived: ws.is_archived,
        })
    }
}
