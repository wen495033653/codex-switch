use super::*;
use crate::{
    codex_sessions::sync_codex_sessions_to_provider_now_from,
    session_sync_diagnostics::log_session_sync_event,
};

mod detect;
mod pending;
mod restart;

pub(crate) use pending::{attach_ide_reopen, build_ide_reopen_payload};

pub(crate) use detect::detect_ide_app;
use detect::{build_ide_summary, normalize_ide_entries};
use restart::restart_from_ide_snapshot;

fn ide_summary_text(value: &Value) -> String {
    let names = value
        .get("summary")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            let display_name = string_field(&item, "displayName");
            let count = value_u64_field(&item, "count").unwrap_or(0);
            if count > 1 {
                format!("{display_name} x{count}")
            } else {
                display_name
            }
        })
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();

    if names.is_empty() {
        "Codex app 或 VS Code".to_string()
    } else {
        names.join("、")
    }
}

pub(crate) fn capture_open_ide_snapshot() -> Result<Value, String> {
    if !cfg!(windows) {
        return Ok(json!({
            "capturedAt": now_string(),
            "entries": [],
            "summary": []
        }));
    }

    let output = run_pwsh(CAPTURE_OPEN_IDE_SNAPSHOT)?;
    let rows = json_as_array(parse_json_output(&output, json!([]))?);
    let entries = normalize_ide_entries(rows);
    let summary = build_ide_summary(&entries);

    Ok(json!({
        "capturedAt": now_string(),
        "entries": entries,
        "summary": summary
    }))
}

#[tauri::command]
pub(crate) fn restart_open_ides(
    snapshot_id: String,
    runtime: State<'_, Arc<IdeRuntime>>,
) -> Result<Value, String> {
    let id = snapshot_id.trim();
    if id.is_empty() {
        return Err("编辑器快照 ID 不能为空".to_string());
    }
    let pending = runtime
        .snapshots
        .lock()
        .map_err(|_| "编辑器快照状态锁异常".to_string())?
        .remove(id)
        .ok_or_else(|| "编辑器快照不存在或已过期".to_string())?;

    log_session_sync_event(
        "ide_reopen_confirm_start",
        json!({
            "snapshotId": id,
            "apiMode": pending.api_mode,
            "accountIdPresent": !pending.account_id.trim().is_empty(),
            "sessionSyncProvider": pending.session_sync_provider.clone()
        }),
    );
    let session_sync_provider = pending.session_sync_provider.clone();
    let mut session_sync_warning = None;
    let result = restart_from_ide_snapshot(&pending.snapshot, || {
        if let Some(target_provider) = session_sync_provider.as_deref() {
            if let Err(err) =
                sync_codex_sessions_to_provider_now_from(target_provider, "ide_reopen_confirm")
            {
                session_sync_warning = Some(err);
            }
        } else {
            log_session_sync_event(
                "ide_reopen_session_sync_skip",
                json!({
                    "snapshotId": id,
                    "reason": "missing_session_sync_provider"
                }),
            );
        }
        Ok(())
    })?;
    let target_text = ide_summary_text(&result);
    let message = if bool_field(&result, "restarted") {
        format!("{target_text} 已重新打开")
    } else {
        format!("未能重新打开 {target_text}")
    };
    let message = if let Some(ref err) = session_sync_warning {
        format!("{message}；会话同步失败：{err}")
    } else {
        message
    };
    log_session_sync_event(
        "ide_reopen_confirm_finish",
        json!({
            "snapshotId": id,
            "sessionSyncWarning": session_sync_warning,
            "result": result
        }),
    );
    store_payload(Some(&message))
}

#[tauri::command]
pub(crate) fn discard_ide_snapshot(
    snapshot_id: String,
    runtime: State<'_, Arc<IdeRuntime>>,
) -> Result<Value, String> {
    let id = snapshot_id.trim();
    if !id.is_empty() {
        if let Some(pending) = runtime
            .snapshots
            .lock()
            .map_err(|_| "编辑器快照状态锁异常".to_string())?
            .remove(id)
        {
            log_session_sync_event(
                "ide_reopen_discard_without_config_apply",
                json!({
                    "snapshotId": id,
                    "apiMode": pending.api_mode,
                    "accountIdPresent": !pending.account_id.trim().is_empty(),
                    "sessionSyncProvider": pending.session_sync_provider.clone()
                }),
            );
        }
    }
    store_payload(Some("已忽略重启提示"))
}
