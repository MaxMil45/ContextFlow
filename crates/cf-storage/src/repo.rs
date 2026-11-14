use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, SubsecRound, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

use cf_core::{
    hash, validate, CaptureStatus, CleanupConfig, Context, ContextId, CoreError, Snapshot,
    SnapshotId, SnapshotPayload, SnapshotSummary,
};

use crate::{migrations, Result, StorageError};

/// Handle to the ContextFlow database.
pub struct Database {
    pub(crate) conn: Connection,
}

/// Result of re-hashing a stored snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityReport {
    pub version: u32,
    pub stored_hash: String,
    pub computed_hash: String,
    pub payload_parses: bool,
}

/// Which snapshots to keep. See [`Database::retention_candidates`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Keep the newest `keep_last` snapshots.
    pub keep_last: Option<u32>,
    /// Keep snapshots younger than `max_age`.
    pub max_age: Option<chrono::TimeDelta>,
}

impl IntegrityReport {
    pub fn ok(&self) -> bool {
        self.payload_parses && self.stored_hash == self.computed_hash
    }
}

const CONTEXT_COLS: &str = "id, name, description, created_at, updated_at, is_archived";
const SNAPSHOT_COLS: &str =
    "id, context_id, version_number, created_at, snapshot_hash, capture_status, payload_json";

pub(crate) fn now() -> DateTime<Utc> {
    // Stored with millisecond precision; truncate so in-memory values round-trip.
    Utc::now().trunc_subsecs(3)
}

pub(crate) fn ts(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn conversion_err<E>(idx: usize, e: E) -> rusqlite::Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
}

fn get_ts(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<DateTime<Utc>> {
    let s: String = row.get(idx)?;
    DateTime::parse_from_rfc3339(&s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| conversion_err(idx, e))
}

fn get_uuid(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Uuid> {
    let s: String = row.get(idx)?;
    Uuid::parse_str(&s).map_err(|e| conversion_err(idx, e))
}

fn row_to_context(row: &rusqlite::Row<'_>) -> rusqlite::Result<Context> {
    Ok(Context {
        id: ContextId(get_uuid(row, 0)?),
        name: row.get(1)?,
        description: row.get(2)?,
        created_at: get_ts(row, 3)?,
        updated_at: get_ts(row, 4)?,
        is_archived: row.get::<_, i64>(5)? != 0,
    })
}

/// Snapshot row before the payload has been decoded.
struct RawSnapshot {
    id: SnapshotId,
    context_id: ContextId,
    version: u32,
    created_at: DateTime<Utc>,
    hash: String,
    capture_status: String,
    payload_json: String,
}

impl RawSnapshot {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: SnapshotId(get_uuid(row, 0)?),
            context_id: ContextId(get_uuid(row, 1)?),
            version: row.get(2)?,
            created_at: get_ts(row, 3)?,
            hash: row.get(4)?,
            capture_status: row.get(5)?,
            payload_json: row.get(6)?,
        })
    }

    fn status(&self) -> Result<CaptureStatus> {
        self.capture_status
            .parse()
            .map_err(|e: String| StorageError::Integrity(e))
    }

    fn into_snapshot(self) -> Result<Snapshot> {
        let capture_status = self.status()?;
        let payload: SnapshotPayload = serde_json::from_str(&self.payload_json).map_err(|e| {
            StorageError::Integrity(format!(
                "snapshot v{} payload is unreadable: {e}",
                self.version
            ))
        })?;
        Ok(Snapshot {
            id: self.id,
            context_id: self.context_id,
            version: self.version,
            created_at: self.created_at,
            hash: self.hash,
            capture_status,
            payload,
        })
    }
}

pub(crate) fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

impl Database {
    /// Open (creating if necessary) the database at `path` and apply migrations.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        Self::init(Connection::open(path)?)
    }

    /// Private in-memory database, for tests and dry runs.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        // journal_mode returns a row, so it must be queried rather than executed.
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        migrations::run(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    pub fn latest_schema_version() -> i64 {
        migrations::latest_version()
    }

    /// Low-level access for diagnostics and tests. Do not use for writes.
    #[doc(hidden)]
    pub fn raw_connection(&self) -> &Connection {
        &self.conn
    }

    /// Run SQLite's own consistency check. Returns an empty list when healthy.
    pub fn integrity_check(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("PRAGMA integrity_check")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().filter(|r| r != "ok").collect())
    }

    // ----------------------------------------------------------------- contexts

    pub fn create_context(&self, name: &str, description: Option<&str>) -> Result<Context> {
        let name = validate::context_name(name)?;
        let now = now();
        let ctx = Context {
            id: ContextId::new(),
            name,
            description: description
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
            created_at: now,
            updated_at: now,
            is_archived: false,
        };
        let res = self.conn.execute(
            &format!("INSERT INTO contexts ({CONTEXT_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, 0)"),
            params![
                ctx.id.to_string(),
                ctx.name,
                ctx.description,
                ts(&now),
                ts(&now)
            ],
        );
        match res {
            Ok(_) => Ok(ctx),
            Err(e) if is_unique_violation(&e) => Err(StorageError::Conflict(format!(
                "a workspace named '{}' already exists",
                ctx.name
            ))),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_contexts(&self, include_archived: bool) -> Result<Vec<Context>> {
        let sql = format!(
            "SELECT {CONTEXT_COLS} FROM contexts {} ORDER BY updated_at DESC, name",
            if include_archived {
                ""
            } else {
                "WHERE is_archived = 0"
            }
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_context)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Look a workspace up by id or (case-insensitive) name.
    pub fn find_context(&self, key: &str) -> Result<Context> {
        let key = key.trim();
        let found = if let Ok(id) = Uuid::parse_str(key) {
            self.conn
                .query_row(
                    &format!("SELECT {CONTEXT_COLS} FROM contexts WHERE id = ?1"),
                    [id.to_string()],
                    row_to_context,
                )
                .optional()?
        } else {
            self.conn
                .query_row(
                    &format!("SELECT {CONTEXT_COLS} FROM contexts WHERE name = ?1"),
                    [key],
                    row_to_context,
                )
                .optional()?
        };
        found.ok_or_else(|| StorageError::NotFound(format!("no workspace named '{key}'")))
    }

    /// Rename a workspace. Names are unique ignoring case; changing only the
    /// capitalisation of a workspace's own name is allowed. Snapshots, the
    /// pinned version and the approval stay as they are (they follow the id).
    pub fn rename_context(&self, id: ContextId, new_name: &str) -> Result<Context> {
        let name = validate::context_name(new_name)?;
        let res = self.conn.execute(
            "UPDATE contexts SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![name, ts(&now()), id.to_string()],
        );
        match res {
            Ok(0) => Err(StorageError::NotFound(format!("no workspace with id {id}"))),
            Ok(_) => self.find_context(&id.to_string()),
            Err(e) if is_unique_violation(&e) => Err(StorageError::Conflict(format!(
                "a workspace named '{name}' already exists"
            ))),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_archived(&self, id: ContextId, archived: bool) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE contexts SET is_archived = ?1, updated_at = ?2 WHERE id = ?3",
            params![archived as i64, ts(&now()), id.to_string()],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("no workspace with id {id}")));
        }
        Ok(())
    }

    /// Whether the user has approved this workspace for restoring. Workspaces
    /// created here are trusted; imported ones are not until approved.
    pub fn is_trusted(&self, id: ContextId) -> Result<bool> {
        let trusted: Option<i64> = self
            .conn
            .query_row(
                "SELECT is_trusted FROM contexts WHERE id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        trusted
            .map(|t| t != 0)
            .ok_or_else(|| StorageError::NotFound(format!("no workspace with id {id}")))
    }

    pub fn set_trusted(&self, id: ContextId, trusted: bool) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE contexts SET is_trusted = ?1 WHERE id = ?2",
            params![trusted as i64, id.to_string()],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("no workspace with id {id}")));
        }
        Ok(())
    }

    /// The workspace's cleanup rules (empty when none were set).
    pub fn cleanup_config(&self, id: ContextId) -> Result<CleanupConfig> {
        let row: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT cleanup_json FROM contexts WHERE id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        match row {
            None => Err(StorageError::NotFound(format!("no workspace with id {id}"))),
            Some(None) => Ok(CleanupConfig::default()),
            Some(Some(json)) => serde_json::from_str(&json)
                .map_err(|e| StorageError::Integrity(format!("cleanup rules of {id}: {e}"))),
        }
    }

    /// Save the workspace's cleanup rules (validated and normalised) and
    /// return what was saved.
    pub fn set_cleanup_config(
        &self,
        id: ContextId,
        config: CleanupConfig,
    ) -> Result<CleanupConfig> {
        let config = config.normalized()?;
        let json = if config.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&config).map_err(CoreError::from)?)
        };
        let n = self.conn.execute(
            "UPDATE contexts SET cleanup_json = ?1 WHERE id = ?2",
            params![json, id.to_string()],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("no workspace with id {id}")));
        }
        Ok(config)
    }

    /// The workspace the user is currently in (the one last restored), if it
    /// still exists.
    pub fn active_context(&self) -> Result<Option<ContextId>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT c.id FROM app_state s JOIN contexts c ON c.id = s.value
                 WHERE s.key = 'active_context'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id.and_then(|s| s.parse::<ContextId>().ok()))
    }

    /// Record which workspace is active, or none.
    pub fn set_active_context(&self, id: Option<ContextId>) -> Result<()> {
        match id {
            Some(id) => self.conn.execute(
                "INSERT INTO app_state (key, value) VALUES ('active_context', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [id.to_string()],
            )?,
            None => self
                .conn
                .execute("DELETE FROM app_state WHERE key = 'active_context'", [])?,
        };
        Ok(())
    }

    /// The snapshot version restore uses for this workspace; `None` means the
    /// latest one.
    pub fn pinned_version(&self, id: ContextId) -> Result<Option<u32>> {
        let row: Option<Option<u32>> = self
            .conn
            .query_row(
                "SELECT pinned_version FROM contexts WHERE id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        row.ok_or_else(|| StorageError::NotFound(format!("no workspace with id {id}")))
    }

    /// Pin restore to `version`, or go back to "latest" with `None`.
    pub fn set_pinned_version(&self, id: ContextId, version: Option<u32>) -> Result<()> {
        if let Some(v) = version {
            let exists: Option<i64> = self
                .conn
                .query_row(
                    "SELECT 1 FROM context_snapshots WHERE context_id = ?1 AND version_number = ?2",
                    params![id.to_string(), v],
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Err(StorageError::NotFound(format!(
                    "snapshot version {v} does not exist"
                )));
            }
        }
        let n = self.conn.execute(
            "UPDATE contexts SET pinned_version = ?1 WHERE id = ?2",
            params![version, id.to_string()],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("no workspace with id {id}")));
        }
        Ok(())
    }

    // ---------------------------------------------------------------- snapshots

    /// Validate, hash and store a new snapshot as the next version of `context_id`.
    ///
    /// Versions are allocated inside an IMMEDIATE transaction, so concurrent
    /// writers cannot allocate the same version number.
    pub fn create_snapshot(
        &mut self,
        context_id: ContextId,
        payload: SnapshotPayload,
        capture_status: CaptureStatus,
    ) -> Result<Snapshot> {
        validate::payload(&payload)?;
        let payload_json = hash::canonical_json(&payload)?;
        let snapshot_hash = hash::sha256_hex(payload_json.as_bytes());
        let created_at = now();
        let id = SnapshotId::new();

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let row: Option<(i64, u32)> = tx
            .query_row(
                "SELECT is_archived, last_version FROM contexts WHERE id = ?1",
                [context_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let last_version = match row {
            None => {
                return Err(StorageError::NotFound(format!(
                    "no workspace with id {context_id}"
                )))
            }
            Some((1, _)) => {
                return Err(StorageError::Conflict(
                    "workspace is archived; unarchive it before capturing".into(),
                ))
            }
            Some((_, v)) => v,
        };
        let version = last_version + 1;

        insert_snapshot_row(
            &tx,
            id,
            context_id,
            version,
            &created_at,
            &snapshot_hash,
            capture_status,
            &payload_json,
        )?;
        insert_projections(&tx, id, &payload, &created_at)?;
        tx.execute(
            "UPDATE contexts SET updated_at = ?1, last_version = ?2 WHERE id = ?3",
            params![ts(&created_at), version, context_id.to_string()],
        )?;
        tx.commit()?;

        tracing::debug!(%context_id, version, hash = %snapshot_hash, "snapshot created");
        Ok(Snapshot {
            id,
            context_id,
            version,
            created_at,
            hash: snapshot_hash,
            capture_status,
            payload,
        })
    }

    pub fn list_snapshots(&self, context_id: ContextId) -> Result<Vec<SnapshotSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, version_number, created_at, snapshot_hash, capture_status
             FROM context_snapshots WHERE context_id = ?1 ORDER BY version_number DESC",
        )?;
        let rows = stmt.query_map([context_id.to_string()], |row| {
            Ok((
                SnapshotId(get_uuid(row, 0)?),
                row.get::<_, u32>(1)?,
                get_ts(row, 2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, version, created_at, hash, status) = row?;
            out.push(SnapshotSummary {
                id,
                version,
                created_at,
                hash,
                capture_status: status.parse().map_err(StorageError::Integrity)?,
            });
        }
        Ok(out)
    }

    fn raw_snapshot(&self, context_id: ContextId, version: Option<u32>) -> Result<RawSnapshot> {
        let raw = match version {
            Some(v) => self
                .conn
                .query_row(
                    &format!(
                        "SELECT {SNAPSHOT_COLS} FROM context_snapshots
                         WHERE context_id = ?1 AND version_number = ?2"
                    ),
                    params![context_id.to_string(), v],
                    RawSnapshot::from_row,
                )
                .optional()?,
            None => self
                .conn
                .query_row(
                    &format!(
                        "SELECT {SNAPSHOT_COLS} FROM context_snapshots
                         WHERE context_id = ?1 ORDER BY version_number DESC LIMIT 1"
                    ),
                    [context_id.to_string()],
                    RawSnapshot::from_row,
                )
                .optional()?,
        };
        raw.ok_or_else(|| {
            StorageError::NotFound(match version {
                Some(v) => format!("snapshot version {v} does not exist"),
                None => "this workspace has no snapshots yet".into(),
            })
        })
    }

    /// Fetch a snapshot by version, or the latest when `version` is `None`.
    pub fn get_snapshot(&self, context_id: ContextId, version: Option<u32>) -> Result<Snapshot> {
        self.raw_snapshot(context_id, version)?.into_snapshot()
    }

    // ------------------------------------------------------ deletion / retention

    /// Permanently delete one snapshot and its projections. Version numbers
    /// are never reused afterwards.
    pub fn delete_snapshot(&self, context_id: ContextId, version: u32) -> Result<()> {
        if self.pinned_version(context_id)? == Some(version) {
            return Err(StorageError::Conflict(format!(
                "v{version} is pinned; run `contextflow unpin` first"
            )));
        }
        let n = self.conn.execute(
            "DELETE FROM context_snapshots WHERE context_id = ?1 AND version_number = ?2",
            params![context_id.to_string(), version],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!(
                "snapshot version {version} does not exist"
            )));
        }
        tracing::info!(%context_id, version, "snapshot deleted");
        Ok(())
    }

    /// Permanently delete a workspace and all of its snapshots.
    pub fn delete_context(&self, id: ContextId) -> Result<()> {
        let n = self
            .conn
            .execute("DELETE FROM contexts WHERE id = ?1", [id.to_string()])?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("no workspace with id {id}")));
        }
        tracing::info!(context_id = %id, "workspace deleted");
        Ok(())
    }

    /// Snapshots that `policy` would prune, newest first. Never includes the
    /// latest snapshot. A snapshot qualifies only if it breaks every rule set.
    pub fn retention_candidates(
        &self,
        context_id: ContextId,
        policy: &RetentionPolicy,
        now: DateTime<Utc>,
    ) -> Result<Vec<SnapshotSummary>> {
        if policy.keep_last.is_none() && policy.max_age.is_none() {
            return Ok(Vec::new());
        }
        let pinned = self.pinned_version(context_id)?;
        let all = self.list_snapshots(context_id)?;
        Ok(all
            .into_iter()
            .enumerate()
            .skip(1) // the latest snapshot is always kept
            .filter(|(_, s)| Some(s.version) != pinned) // and so is the pinned one
            .filter(|(rank, s)| {
                let outside_count = policy.keep_last.map(|n| *rank >= n as usize);
                let too_old = policy.max_age.map(|age| s.created_at < now - age);
                [outside_count, too_old].into_iter().flatten().all(|b| b)
            })
            .map(|(_, s)| s)
            .collect())
    }

    /// Delete the snapshots selected by [`Self::retention_candidates`] in one
    /// transaction. Returns the deleted versions.
    pub fn prune(
        &mut self,
        context_id: ContextId,
        policy: &RetentionPolicy,
        now: DateTime<Utc>,
    ) -> Result<Vec<u32>> {
        let versions: Vec<u32> = self
            .retention_candidates(context_id, policy, now)?
            .into_iter()
            .map(|s| s.version)
            .collect();
        let tx = self.conn.transaction()?;
        for v in &versions {
            tx.execute(
                "DELETE FROM context_snapshots WHERE context_id = ?1 AND version_number = ?2",
                params![context_id.to_string(), v],
            )?;
        }
        tx.commit()?;
        if !versions.is_empty() {
            tracing::info!(%context_id, ?versions, "pruned snapshots");
        }
        Ok(versions)
    }

    // ------------------------------------------------------------------ backup

    /// Write a consistent, compacted copy of the whole database to `dest`.
    /// Refuses to overwrite an existing file.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        if dest.exists() {
            return Err(StorageError::Conflict(format!(
                "{} already exists; choose another file",
                dest.display()
            )));
        }
        if let Some(parent) = dest.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        self.conn
            .execute("VACUUM INTO ?1", [dest.to_string_lossy().as_ref()])?;
        tracing::info!(
            dest = %cf_core::redact::home(&dest.display().to_string()),
            "database backup written"
        );
        Ok(())
    }

    /// Re-hash the stored canonical payload and compare with the stored hash.
    pub fn verify_snapshot(
        &self,
        context_id: ContextId,
        version: Option<u32>,
    ) -> Result<IntegrityReport> {
        let raw = self.raw_snapshot(context_id, version)?;
        let computed_hash = hash::sha256_hex(raw.payload_json.as_bytes());
        let payload_parses = serde_json::from_str::<SnapshotPayload>(&raw.payload_json).is_ok();
        Ok(IntegrityReport {
            version: raw.version,
            stored_hash: raw.hash,
            computed_hash,
            payload_parses,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_snapshot_row(
    tx: &Transaction<'_>,
    id: SnapshotId,
    context_id: ContextId,
    version: u32,
    created_at: &DateTime<Utc>,
    hash: &str,
    status: CaptureStatus,
    payload_json: &str,
) -> Result<()> {
    tx.execute(
        &format!(
            "INSERT INTO context_snapshots ({SNAPSHOT_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
        ),
        params![
            id.to_string(),
            context_id.to_string(),
            version,
            ts(created_at),
            hash,
            status.as_str(),
            payload_json
        ],
    )?;
    Ok(())
}

pub(crate) fn insert_projections(
    tx: &Transaction<'_>,
    snapshot_id: SnapshotId,
    p: &SnapshotPayload,
    created_at: &DateTime<Utc>,
) -> Result<()> {
    let sid = snapshot_id.to_string();

    let mut apps = tx.prepare(
        "INSERT INTO snapshot_applications
         (snapshot_id, application_id, executable_path, launch_arguments_json,
          working_directory, adapter_id, adapter_state_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for a in &p.applications {
        apps.execute(params![
            sid,
            a.application_id,
            a.executable_path,
            serde_json::to_string(&a.launch_arguments).map_err(cf_core::CoreError::from)?,
            a.working_directory,
            a.adapter_id,
            hash::canonical_json(&a.adapter_state)?,
        ])?;
    }

    let mut wins = tx.prepare(
        "INSERT INTO snapshot_windows
         (snapshot_id, application_id, title, window_class, monitor_identifier,
          x, y, width, height, display_state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    for w in &p.windows {
        wins.execute(params![
            sid,
            w.application_id,
            w.title,
            w.window_class,
            w.monitor_id,
            w.rect.x,
            w.rect.y,
            w.rect.width,
            w.rect.height,
            w.display_state.as_str(),
        ])?;
    }

    let mut git = tx.prepare(
        "INSERT INTO snapshot_git_states
         (snapshot_id, repository_path, branch_name, head_commit,
          staged_changes, modified_changes, untracked_changes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for g in &p.git {
        git.execute(params![
            sid,
            g.repository_path,
            g.branch,
            g.head_commit,
            g.staged,
            g.modified,
            g.untracked
        ])?;
    }

    let mut notes = tx.prepare(
        "INSERT INTO snapshot_notes (snapshot_id, note_content, created_at) VALUES (?1, ?2, ?3)",
    )?;
    for n in &p.notes {
        notes.execute(params![sid, n, ts(created_at)])?;
    }
    Ok(())
}
