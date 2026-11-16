use super::{
    codex_home::{
        remove_from_global_state, resolve_codex_root, session_id_variants, validate_codex_root,
    },
    util::dedupe_strings,
};
use crate::{
    app_log::log_event,
    codex_app_server::{delete_threads, ThreadDeleteBatch},
    codex_sessions::lock_codex_session_io,
};
use serde_json::{json, Value};
use std::path::Path;

pub(super) fn delete_conversations_impl(
    root: String,
    mut thread_ids: Vec<String>,
) -> Result<Value, String> {
    if thread_ids.is_empty() {
        return Err("请先选择要删除的会话".to_string());
    }
    if thread_ids.iter().any(|id| id.trim().is_empty()) {
        return Err("会话 ID 为空".to_string());
    }
    dedupe_strings(&mut thread_ids);

    let root = resolve_codex_root(Some(&root))?;
    let _io_guard = lock_codex_session_io("永久删除会话")?;
    validate_codex_root(&root)?;
    Ok(finish_delete(&root, delete_threads(&root, thread_ids)?))
}

pub(super) fn finish_delete(root: &Path, batch: ThreadDeleteBatch) -> Value {
    let ThreadDeleteBatch {
        deleted,
        removed_ids,
        errors,
    } = batch;
    let global_state_error = if removed_ids.is_empty() {
        None
    } else {
        let mut ids = removed_ids
            .iter()
            .flat_map(|id| session_id_variants(id))
            .collect();
        dedupe_strings(&mut ids);
        remove_from_global_state(root, &ids, "delete").err()
    };

    let mut message = format!("已永久删除 {deleted} 个会话");
    if !errors.is_empty() {
        message.push_str(&format!("，{} 个失败：{}", errors.len(), errors.join("；")));
    }
    let mut warnings = Vec::new();
    if let Some(err) = &global_state_error {
        log_event(
            "session_manager_delete_cleanup_error",
            json!({ "root": root, "deleted": deleted, "globalStateError": err }),
        );
        let warning = format!("Codex global state 清理失败: {err}");
        message.push('；');
        message.push_str(&warning);
        warnings.push(warning);
    }
    json!({
        "ok": errors.is_empty() && warnings.is_empty(),
        "message": message,
        "report": {
            "deleted": deleted,
            "global_state_error": global_state_error,
            "failed": errors.len(),
            "errors": errors,
            "warnings": warnings
        }
    })
}
