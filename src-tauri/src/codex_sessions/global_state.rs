use super::support::GLOBAL_STATE_FILE_NAME;
use crate::{app_log::log_event, atomic_file::write_file_atomically};
use serde_json::{json, Map, Value};
use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub(super) fn to_desktop_workspace_path(value: &str) -> Option<String> {
    let stripped = value.trim();
    if stripped.is_empty() {
        return None;
    }
    let lower = stripped.to_ascii_lowercase();
    if lower.starts_with(r"\\?\unc\") {
        return Some(format!(r"\\{}", stripped[8..].replace('/', "\\")));
    }
    if let Some(stripped) = stripped.strip_prefix(r"\\?\") {
        return Some(stripped.replace('\\', "/"));
    }
    Some(stripped.to_string())
}

#[cfg(test)]
pub(super) fn sync_global_state_workspace_roots(path: &Path) -> Result<usize, String> {
    sync_global_state_workspace_roots_with_diagnostics(path, None)
}

#[cfg(test)]
pub(super) fn preview_global_state_workspace_roots(path: &Path) -> Result<usize, String> {
    preview_global_state_workspace_roots_with_diagnostics(path, None)
}

pub(super) fn preview_global_state_workspace_roots_with_diagnostics(
    path: &Path,
    trigger: Option<&str>,
) -> Result<usize, String> {
    let state = load_global_state(path)?;
    let next = normalized_global_state_workspace_roots(&state);
    let updated = global_state_update_count(&state, &next);
    if let Some(trigger) = trigger {
        log_event(
            "session_sync_preflight_global_state_summary",
            json!({
                "trigger": trigger,
                "globalState": path.to_string_lossy().to_string(),
                "updated": updated
            }),
        );
    }
    Ok(updated)
}

/// The one read-modify-write of `.codex-global-state.json`, shared by the session sync and the
/// session manager (caller holds the session I/O lock). `update` edits the parsed JSON and
/// returns how many changes it made; with none, nothing is written. Otherwise the original text
/// is saved to `backup_path()` (the caller picks the location) and the file is rewritten in
/// place, only if it still exists: a file removed meanwhile is reported, never recreated.
pub(crate) fn rewrite_global_state_file(
    path: &Path,
    backup_path: impl FnOnce() -> Result<PathBuf, String>,
    update: impl FnOnce(&mut Value) -> usize,
) -> Result<usize, String> {
    let original_content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(0),
        Err(err) => {
            return Err(format!(
                "读取 Codex global state 失败 {}: {err}",
                path.display()
            ))
        }
    };
    let mut value: Value = serde_json::from_str(&original_content)
        .map_err(|err| format!("解析 Codex global state 失败 {}: {err}", path.display()))?;
    let updated = update(&mut value);
    if updated == 0 {
        return Ok(0);
    }
    let backup_path = backup_path()?;
    fs::write(&backup_path, &original_content).map_err(|err| {
        format!(
            "备份 Codex global state 失败 {}: {err}",
            backup_path.display()
        )
    })?;
    let mut output = serde_json::to_string_pretty(&value)
        .map_err(|err| format!("序列化 Codex global state 失败: {err}"))?;
    output.push('\n');
    // The whole document is rewritten, so it is replaced atomically: a crash mid-write used to
    // leave Codex a truncated global state. A file deleted since it was read is not recreated.
    if !path.exists() {
        return Err(format!(
            "Codex global state 在写入前被删除，未重新创建: {}",
            path.display()
        ));
    }
    write_file_atomically(path, output.as_bytes())
        .map_err(|err| format!("写入 Codex global state 失败 {}: {err}", path.display()))?;
    Ok(updated)
}

pub(super) fn sync_global_state_workspace_roots_with_diagnostics(
    path: &Path,
    trigger: Option<&str>,
) -> Result<usize, String> {
    if !path.exists() {
        if let Some(trigger) = trigger {
            log_event(
                "session_sync_global_state_missing",
                json!({
                    "trigger": trigger,
                    "globalState": path.to_string_lossy().to_string()
                }),
            );
        }
        return Ok(0);
    }

    let backup_path = || Ok(path.with_file_name(format!("{GLOBAL_STATE_FILE_NAME}.bak")));
    let updated = rewrite_global_state_file(path, backup_path, |value| {
        // A non-object document has no workspace roots to normalize (it was read as empty).
        let Some(state) = value.as_object_mut() else {
            return 0;
        };
        let next = normalized_global_state_workspace_roots(state);
        let updated = global_state_update_count(state, &next);
        for (key, value) in next {
            state.insert(key, value);
        }
        updated
    })?;
    if let Some(trigger) = trigger {
        log_event(
            "session_sync_global_state_summary",
            json!({
                "trigger": trigger,
                "globalState": path.to_string_lossy().to_string(),
                "updated": updated
            }),
        );
    }
    Ok(updated)
}

fn load_global_state(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|err| format!("读取 Codex global state 失败 {}: {err}", path.display()))?;
    parse_global_state(&content, path)
}

fn parse_global_state(content: &str, path: &Path) -> Result<Map<String, Value>, String> {
    let value: Value = serde_json::from_str(content)
        .map_err(|err| format!("解析 Codex global state 失败 {}: {err}", path.display()))?;
    Ok(value.as_object().cloned().unwrap_or_default())
}

fn normalized_global_state_workspace_roots(state: &Map<String, Value>) -> Map<String, Value> {
    let mut next = Map::new();
    if let Some(value) = state.get("electron-saved-workspace-roots") {
        next.insert(
            "electron-saved-workspace-roots".to_string(),
            json!(dedupe_paths(path_array(value))),
        );
    }
    if let Some(value) = state.get("project-order") {
        next.insert(
            "project-order".to_string(),
            json!(dedupe_paths(path_array(value))),
        );
    }
    if let Some(value) = state.get("active-workspace-roots") {
        let normalized = dedupe_paths(path_array(value));
        let next_value = if value.is_array() {
            json!(normalized)
        } else if let Some(first) = normalized.first() {
            json!(first)
        } else {
            value.clone()
        };
        next.insert("active-workspace-roots".to_string(), next_value);
    }
    if let Some(value) = state
        .get("electron-workspace-root-labels")
        .and_then(Value::as_object)
    {
        let mut labels = Map::new();
        for (key, item) in value {
            labels.insert(
                to_desktop_workspace_path(key).unwrap_or_else(|| key.clone()),
                item.clone(),
            );
        }
        next.insert(
            "electron-workspace-root-labels".to_string(),
            Value::Object(labels),
        );
    }
    if let Some(open_targets) = state
        .get("open-in-target-preferences")
        .and_then(Value::as_object)
    {
        let mut next_open_targets = open_targets.clone();
        if let Some(per_path) =
            copy_resolved_object_keys(open_targets.get("perPath").and_then(Value::as_object))
        {
            next_open_targets.insert("perPath".to_string(), Value::Object(per_path));
        }
        next.insert(
            "open-in-target-preferences".to_string(),
            Value::Object(next_open_targets),
        );
    }
    next
}

fn copy_resolved_object_keys(value: Option<&Map<String, Value>>) -> Option<Map<String, Value>> {
    let value = value?;
    let mut next = Map::new();
    for (key, item) in value {
        next.insert(
            to_desktop_workspace_path(key).unwrap_or_else(|| key.clone()),
            item.clone(),
        );
    }
    Some(next)
}

fn global_state_update_count(state: &Map<String, Value>, next: &Map<String, Value>) -> usize {
    next.iter()
        .filter(|(key, value)| state.get(*key) != Some(*value))
        .count()
}

fn path_array(value: &Value) -> Vec<String> {
    if let Some(items) = value.as_array() {
        items
            .iter()
            .filter_map(Value::as_str)
            .filter(|item| !item.trim().is_empty())
            .map(ToString::to_string)
            .collect()
    } else if let Some(value) = value.as_str().filter(|item| !item.trim().is_empty()) {
        vec![value.to_string()]
    } else {
        Vec::new()
    }
}

fn dedupe_paths(paths: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for path in paths {
        let normalized = to_desktop_workspace_path(&path).unwrap_or(path);
        if seen.insert(normalized.to_ascii_lowercase()) {
            result.push(normalized);
        }
    }
    result
}
