use super::*;

fn create_ide_snapshot_id() -> String {
    format!(
        "{}_{}",
        OffsetDateTime::now_utc().unix_timestamp(),
        random_urlsafe(8)
    )
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
    if let Ok(mut snapshots) = runtime.snapshots.lock() {
        snapshots.insert(
            snapshot_id.clone(),
            IdePending {
                snapshot: snapshot.clone(),
                account_id,
                api_mode,
                session_sync_provider: session_sync_provider.clone(),
            },
        );
        if snapshots.len() > 20 {
            let overflow = snapshots.len() - 20;
            let keys: Vec<String> = snapshots.keys().take(overflow).cloned().collect();
            for key in keys {
                snapshots.remove(&key);
            }
        }
    }

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

pub(super) fn apply_pending_ide_auth(pending: &IdePending) -> Result<(), String> {
    if pending.api_mode {
        let mut settings = read_settings_value()?;
        let api_profile_id = pending.account_id.trim();
        if !api_profile_id.is_empty() {
            settings = update_settings_value(&json!({
                "active_api_profile_id": api_profile_id
            }))?;
        }
        let profile = settings
            .get("api_mode")
            .cloned()
            .unwrap_or_else(default_api_mode);
        set_api_mode(&profile)?;
        return Ok(());
    }

    if pending.account_id.trim().is_empty() {
        return Ok(());
    }
    let account = find_store_account(&pending.account_id)?;
    write_account_auth(&account)?;
    set_subscription_mode()?;
    let _ = mark_store_account_used(&pending.account_id)?;
    Ok(())
}
