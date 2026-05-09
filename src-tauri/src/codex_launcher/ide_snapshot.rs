use super::*;

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
    let result = restart_from_ide_snapshot(&pending.snapshot)?;
    let message = if bool_field(&result, "restarted") {
        "Codex app 重启成功".to_string()
    } else {
        "未能重启 Codex app".to_string()
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
