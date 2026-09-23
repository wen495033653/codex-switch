use super::capture_open_ide_snapshot;
use super::{IdePending, IdeRuntime};
use crate::{accounts::random_urlsafe, app_log::log_event};
use serde_json::{json, Value};
use time::OffsetDateTime;

const MAX_PENDING_SNAPSHOTS: usize = 20;

fn create_ide_snapshot_id() -> String {
    format!(
        "{}_{}",
        OffsetDateTime::now_utc().unix_timestamp(),
        random_urlsafe(8)
    )
}

fn snapshot_created_at(snapshot_id: &str) -> i64 {
    snapshot_id
        .split_once('_')
        .and_then(|(seconds, _)| seconds.parse().ok())
        .unwrap_or(i64::MIN)
}

/// Ids to drop so that at most `MAX_PENDING_SNAPSHOTS` remain, oldest first. HashMap order is
/// arbitrary, so without sorting the snapshot just handed to the window could be the one dropped.
fn oldest_snapshot_ids<'a>(ids: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut ids = ids.cloned().collect::<Vec<_>>();
    let overflow = ids.len().saturating_sub(MAX_PENDING_SNAPSHOTS);
    ids.sort_by_key(|id| snapshot_created_at(id));
    ids.truncate(overflow);
    ids
}

pub(crate) fn build_ide_reopen_payload(
    runtime: &IdeRuntime,
    account_id: String,
    api_mode: bool,
    session_sync_provider: Option<String>,
) -> Option<Value> {
    let snapshot = capture_open_ide_snapshot().ok()?;
    let entries = snapshot
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        return None;
    }

    let snapshot_id = create_ide_snapshot_id();
    let Ok(mut snapshots) = runtime.snapshots.lock() else {
        // Without a stored snapshot the confirm button could only fail, so no prompt is offered.
        log_event(
            "ide_reopen_snapshot_store_error",
            json!({ "error": "编辑器快照状态锁异常", "entries": entries.len() }),
        );
        return None;
    };
    snapshots.insert(
        snapshot_id.clone(),
        IdePending {
            snapshot: snapshot.clone(),
            account_id,
            api_mode,
            session_sync_provider: session_sync_provider.clone(),
        },
    );
    for id in oldest_snapshot_ids(snapshots.keys()) {
        snapshots.remove(&id);
    }
    drop(snapshots);

    Some(json!({
        "snapshot_id": snapshot_id,
        "summary": snapshot.get("summary").cloned().unwrap_or_else(|| json!([])),
        "session_sync": session_sync_provider.is_some()
    }))
}

pub(crate) fn attach_ide_reopen(mut payload: Value, ide_reopen: Option<Value>) -> Value {
    if let Some(value) = ide_reopen {
        payload["ide_reopen"] = value;
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_drops_the_oldest_snapshots() {
        let ids = (0..23)
            .rev()
            .map(|index| format!("{}_id{index}", 1_700_000_000 + index))
            .collect::<Vec<_>>();

        let dropped = oldest_snapshot_ids(ids.iter());

        assert_eq!(
            dropped,
            vec![
                "1700000000_id0".to_string(),
                "1700000001_id1".to_string(),
                "1700000002_id2".to_string()
            ]
        );
    }

    #[test]
    fn no_snapshot_is_dropped_within_the_limit() {
        let ids = (0..20)
            .map(|index| format!("{}_id{index}", 1_700_000_000 + index))
            .collect::<Vec<_>>();

        assert!(oldest_snapshot_ids(ids.iter()).is_empty());
    }
}
