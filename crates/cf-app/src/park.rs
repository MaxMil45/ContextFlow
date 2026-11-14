//! Park and resume: put the workspace you're in aside to deal with something
//! else, then come back to it in one step.
//!
//! Parking saves the workspace (with your note) like any switch and pushes it
//! onto a small stack, so interruptions can nest: park *Thesis* for *Hotfix*,
//! park *Hotfix* for a call, and Resume takes you back through them in order.

use cf_core::{Context, ContextId};
use cf_storage::{Database, EventKind};

const KEY: &str = "return_stack";
/// Most workspaces kept to come back to.
const MAX_PARKED: usize = 10;

fn read(db: &Database) -> Vec<ContextId> {
    db.state_value(KEY)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| s.parse().ok())
        .collect()
}

fn write(db: &Database, stack: &[ContextId]) -> anyhow::Result<()> {
    let ids: Vec<String> = stack.iter().map(|i| i.to_string()).collect();
    let value = (!ids.is_empty())
        .then(|| serde_json::to_string(&ids))
        .transpose()?;
    db.set_state_value(KEY, value.as_deref())?;
    Ok(())
}

/// The parked workspaces that still exist, the one Resume returns to first.
pub fn parked(db: &Database) -> Vec<Context> {
    read(db)
        .into_iter()
        .rev()
        .filter_map(|id| db.find_context(&id.to_string()).ok())
        .filter(|c| !c.is_archived)
        .collect()
}

/// Park `id` (normally the workspace you're in) to come back to.
pub fn park(db: &Database, id: ContextId, note: Option<&str>) -> anyhow::Result<()> {
    let mut stack = read(db);
    stack.retain(|s| *s != id);
    stack.push(id);
    if stack.len() > MAX_PARKED {
        stack.remove(0);
    }
    write(db, &stack)?;
    db.record_event(EventKind::Park, Some(id), None, None, note)?;
    Ok(())
}

/// Take the workspace to resume off the stack (skipping any since deleted or
/// archived). The caller then restores it.
pub fn take_resume(db: &Database) -> anyhow::Result<Option<Context>> {
    let mut stack = read(db);
    while let Some(id) = stack.pop() {
        if let Ok(ctx) = db.find_context(&id.to_string()) {
            if !ctx.is_archived {
                write(db, &stack)?;
                db.record_event(EventKind::Resume, Some(ctx.id), None, None, None)?;
                return Ok(Some(ctx));
            }
        }
    }
    write(db, &stack)?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interruptions_nest_and_resume_in_order() {
        let db = Database::open_in_memory().unwrap();
        let thesis = db.create_context("Thesis", None).unwrap();
        let hotfix = db.create_context("Hotfix", None).unwrap();
        park(&db, thesis.id, Some("chapter 3")).unwrap();
        park(&db, hotfix.id, None).unwrap();
        assert_eq!(
            parked(&db)
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Hotfix", "Thesis"]
        );
        assert_eq!(take_resume(&db).unwrap().unwrap().name, "Hotfix");
        assert_eq!(take_resume(&db).unwrap().unwrap().name, "Thesis");
        assert!(take_resume(&db).unwrap().is_none());
        // The timeline remembers it all, newest first.
        let kinds: Vec<_> = db
            .recent_events(10)
            .unwrap()
            .iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::Resume,
                EventKind::Resume,
                EventKind::Park,
                EventKind::Park
            ]
        );
        assert_eq!(
            db.recent_events(10).unwrap()[3].note.as_deref(),
            Some("chapter 3")
        );
    }

    #[test]
    fn a_deleted_workspace_is_skipped() {
        let db = Database::open_in_memory().unwrap();
        let a = db.create_context("A", None).unwrap();
        let b = db.create_context("B", None).unwrap();
        park(&db, a.id, None).unwrap();
        park(&db, b.id, None).unwrap();
        db.delete_context(b.id).unwrap();
        assert_eq!(take_resume(&db).unwrap().unwrap().name, "A");
    }
}
