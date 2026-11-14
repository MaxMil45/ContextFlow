//! The timeline (`context_events`) and each workspace's start and stop commands.

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};

use cf_core::{ContextId, CoreError, WorkspaceCommands};

use crate::repo::{now, ts};
use crate::{Database, Result, StorageError};

/// Entries kept in the timeline.
const MAX_EVENTS: i64 = 1000;

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Left one workspace for another.
    Switch,
    /// Restored a workspace without leaving another.
    Restore,
    /// Left a workspace without entering another.
    Leave,
    /// Put a workspace aside to come back to.
    Park,
    /// Came back to a parked workspace.
    Resume,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Switch => "switch",
            Self::Restore => "restore",
            Self::Leave => "leave",
            Self::Park => "park",
            Self::Resume => "resume",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "switch" => Self::Switch,
            "leave" => Self::Leave,
            "park" => Self::Park,
            "resume" => Self::Resume,
            _ => Self::Restore,
        }
    }
}

/// One timeline entry, with the workspaces' names as they are now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecord {
    pub at: DateTime<Utc>,
    pub kind: EventKind,
    pub workspace_id: Option<ContextId>,
    pub workspace: Option<String>,
    pub from_id: Option<ContextId>,
    pub from: Option<String>,
    pub version: Option<u32>,
    pub note: Option<String>,
}

impl Database {
    /// Add a timeline entry (older ones beyond the limit are dropped).
    pub fn record_event(
        &self,
        kind: EventKind,
        workspace: Option<ContextId>,
        from: Option<ContextId>,
        version: Option<u32>,
        note: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO context_events (at, kind, context_id, from_context_id, version, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ts(&now()),
                kind.as_str(),
                workspace.map(|w| w.to_string()),
                from.map(|f| f.to_string()),
                version,
                note
            ],
        )?;
        self.conn.execute(
            "DELETE FROM context_events WHERE id NOT IN
               (SELECT id FROM context_events ORDER BY id DESC LIMIT ?1)",
            [MAX_EVENTS],
        )?;
        Ok(())
    }

    /// The most recent timeline entries, newest first.
    pub fn recent_events(&self, limit: u32) -> Result<Vec<EventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.at, e.kind, e.context_id, c.name, e.from_context_id, f.name, e.version, e.note
             FROM context_events e
             LEFT JOIN contexts c ON c.id = e.context_id
             LEFT JOIN contexts f ON f.id = e.from_context_id
             ORDER BY e.id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<u32>>(6)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })?;
        let id = |s: Option<String>| s.and_then(|s| s.parse::<ContextId>().ok());
        let mut out = Vec::new();
        for row in rows {
            let (at, kind, wid, wname, fid, fname, version, note) = row?;
            out.push(EventRecord {
                at: DateTime::parse_from_rfc3339(&at)
                    .map(|d| d.with_timezone(&Utc))
                    .map_err(|e| StorageError::Integrity(format!("timeline time: {e}")))?,
                kind: EventKind::parse(&kind),
                workspace_id: id(wid),
                workspace: wname,
                from_id: id(fid),
                from: fname,
                version,
                note,
            });
        }
        Ok(out)
    }

    /// The workspace's start and stop commands (empty when none were set).
    pub fn workspace_commands(&self, id: ContextId) -> Result<WorkspaceCommands> {
        let row: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT commands_json FROM contexts WHERE id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        match row {
            None => Err(StorageError::NotFound(format!("no workspace with id {id}"))),
            Some(None) => Ok(WorkspaceCommands::default()),
            Some(Some(json)) => serde_json::from_str(&json)
                .map_err(|e| StorageError::Integrity(format!("commands of {id}: {e}"))),
        }
    }

    /// Save the workspace's start and stop commands (checked first).
    pub fn set_workspace_commands(
        &self,
        id: ContextId,
        commands: WorkspaceCommands,
    ) -> Result<WorkspaceCommands> {
        let commands = commands.normalized()?;
        let json = if commands.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&commands).map_err(CoreError::from)?)
        };
        let n = self.conn.execute(
            "UPDATE contexts SET commands_json = ?1 WHERE id = ?2",
            params![json, id.to_string()],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("no workspace with id {id}")));
        }
        Ok(commands)
    }
}
