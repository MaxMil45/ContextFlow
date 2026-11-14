use rusqlite::Connection;

use crate::{Result, StorageError};

/// Ordered list of migrations. Never edit a migration that has shipped;
/// append a new one instead.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_initial.sql"),
    include_str!("../migrations/0002_version_counter.sql"),
    include_str!("../migrations/0003_trust.sql"),
    include_str!("../migrations/0004_pinned_version.sql"),
    include_str!("../migrations/0005_cleanup.sql"),
    include_str!("../migrations/0006_drop_unused_projections.sql"),
];

pub fn run(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let supported = MIGRATIONS.len() as i64;
    if current > supported {
        return Err(StorageError::UnsupportedSchema {
            found: current,
            supported,
        });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let target = index as i64 + 1;
        tracing::info!(target, "applying database migration");
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", target)?;
        tx.commit()?;
    }
    Ok(())
}

pub fn latest_version() -> i64 {
    MIGRATIONS.len() as i64
}
