use super::*;
use crate::codex_sessions::sync_codex_session_index_then_queue_rollouts;

mod detect;
mod pending;
mod restart;

pub(crate) use pending::{attach_ide_reopen, build_ide_reopen_payload};

use detect::{build_ide_summary, normalize_ide_entries};
use pending::apply_pending_ide_auth;
use restart::restart_from_ide_snapshot;

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

    apply_pending_ide_auth(&pending)?;
    let target_provider = if pending.api_mode { "api" } else { "openai" };
    let mut session_sync_warning = None;
    let result = restart_from_ide_snapshot(&pending.snapshot, || {
        if let Err(err) = sync_codex_session_index_then_queue_rollouts(target_provider) {
            session_sync_warning = Some(err);
        }
    })?;
    let message = if bool_field(&result, "restarted") {
        "Codex app 重启成功"
    } else {
        "未能重启 Codex app"
    };
    let message = if let Some(err) = session_sync_warning {
        format!("{message}；会话同步失败：{err}")
    } else {
        message.to_string()
    };
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
            apply_pending_ide_auth(&pending)?;
        }
    }
    store_payload(Some("已忽略重启提示"))
}
