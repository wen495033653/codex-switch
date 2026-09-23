use super::{
    global_state::to_desktop_workspace_path,
    support::{global_state_path, provider_log_value, state_db_path},
};
use crate::{
    paths::{codex_dir, codex_home_from_state_db_path},
    session_sync_diagnostics::log_session_sync_event,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{BufRead, BufReader, ErrorKind},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Default)]
pub(super) struct StateThreadSyncMetadata {
    pub(super) user_event_thread_ids: HashSet<String>,
    pub(super) cwd_by_thread_id: BTreeMap<String, String>,
    /// Rollouts another process holds open without sharing (Windows sharing / lock violation or
    /// access denied). They are skipped for this sync, as before, but now counted in the summary.
    pub(super) locked_rollouts: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default)]
struct StateThreadUpdateCounts {
    provider_rows: usize,
    user_event_rows: usize,
    cwd_rows: usize,
}

impl StateThreadUpdateCounts {
    fn total(self) -> usize {
        self.provider_rows + self.user_event_rows + self.cwd_rows
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct RolloutThreadMetadata {
    pub(super) cwd: Option<String>,
    pub(super) has_user_event: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RolloutMetadataRead {
    Missing,
    Locked,
    Read(RolloutThreadMetadata),
}

const USER_EVENT_MARKERS: [&[u8]; 2] = [b"\"user_message\"", b"\"user_input\""];

const SESSION_META_MARKER: &[u8] = b"\"session_meta\"";

pub(super) fn pinned_thread_rollout_paths_if_exists() -> Result<Vec<PathBuf>, String> {
    let codex_home = codex_dir()?;
    pinned_thread_rollout_paths(&global_state_path()?, &state_db_path()?, &codex_home)
}

pub(super) fn pinned_thread_rollout_paths(
    global_state: &Path,
    state_db: &Path,
    codex_home: &Path,
) -> Result<Vec<PathBuf>, String> {
    if !global_state.exists() || !state_db.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(global_state).map_err(|err| {
        format!(
            "读取 Codex global state 失败 {}: {err}",
            global_state.display()
        )
    })?;
    let state: Value = serde_json::from_str(&content).map_err(|err| {
        format!(
            "解析 Codex global state 失败 {}: {err}",
            global_state.display()
        )
    })?;
    let pinned_thread_ids = state
        .get("pinned-thread-ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if pinned_thread_ids.is_empty() {
        return Ok(Vec::new());
    }

    let connection = Connection::open_with_flags(
        state_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| format!("打开 Codex state 数据库失败 {}: {err}", state_db.display()))?;
    let mut statement = connection
        .prepare("SELECT rollout_path FROM threads WHERE id = ?1")
        .map_err(|err| {
            format!(
                "读取 Codex pinned thread rollout 查询失败 {}: {err}",
                state_db.display()
            )
        })?;

    let mut paths = Vec::new();
    for thread_id in pinned_thread_ids {
        let thread_id = thread_id.as_str().unwrap_or("").trim();
        if thread_id.is_empty() {
            continue;
        }
        match statement.query_row([thread_id], |row| row.get::<_, String>(0)) {
            Ok(path) if !path.trim().is_empty() => {
                paths.push(state_thread_rollout_path(codex_home, &path))
            }
            Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(err) => {
                return Err(format!(
                    "读取 Codex pinned thread rollout 失败 {thread_id}: {err}"
                ))
            }
        }
    }

    Ok(paths)
}

fn state_threads_columns(connection: &Connection) -> Result<Option<HashSet<String>>, String> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'threads')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("检查 Codex state threads 表失败: {err}"))?;
    if exists == 0 {
        return Ok(None);
    }

    let mut statement = connection
        .prepare("PRAGMA table_info(threads)")
        .map_err(|err| format!("读取 Codex state threads 表结构失败: {err}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| format!("读取 Codex state threads 表结构失败: {err}"))?;
    let mut columns = HashSet::new();
    for row in rows {
        columns.insert(row.map_err(|err| format!("读取 Codex state threads 列失败: {err}"))?);
    }
    Ok(Some(columns))
}

pub(super) fn collect_state_thread_sync_metadata(
    connection: &Connection,
    state_db: &Path,
    columns: &HashSet<String>,
) -> Result<StateThreadSyncMetadata, String> {
    let wants_user_event = columns.contains("has_user_event");
    let wants_cwd = columns.contains("cwd");
    if !wants_user_event && !wants_cwd {
        return Ok(StateThreadSyncMetadata::default());
    }
    if !columns.contains("id") || !columns.contains("rollout_path") {
        return Ok(StateThreadSyncMetadata::default());
    }

    // A row that already has has_user_event = 1 cannot be changed by the user-event update, so its
    // rollout does not need to be scanned for user messages.
    let sql = format!(
        "SELECT id, rollout_path, {}
         FROM threads
         WHERE COALESCE(rollout_path, '') <> ''",
        if wants_user_event {
            "CASE WHEN COALESCE(has_user_event, 0) = 1 THEN 1 ELSE 0 END"
        } else {
            "0"
        }
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|err| format!("查询 Codex state 会话 rollout 路径失败: {err}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)? == 1,
            ))
        })
        .map_err(|err| format!("读取 Codex state 会话 rollout 路径失败: {err}"))?;

    let mut metadata = StateThreadSyncMetadata::default();
    let root = codex_home_from_state_db_path(state_db);
    for row in rows {
        let (thread_id, rollout_path, user_event_recorded) =
            row.map_err(|err| format!("读取 Codex state 会话 rollout 路径失败: {err}"))?;
        let thread_id = thread_id.trim();
        let Some(rollout_path) = rollout_path else {
            continue;
        };
        if thread_id.is_empty() || rollout_path.trim().is_empty() {
            continue;
        }
        let scan_user_event = wants_user_event && !user_event_recorded;
        if !scan_user_event && !wants_cwd {
            continue;
        }
        let rollout_path = state_thread_rollout_path(&root, &rollout_path);
        let rollout_metadata =
            match read_rollout_thread_metadata(&rollout_path, scan_user_event, wants_cwd)? {
                RolloutMetadataRead::Read(rollout_metadata) => rollout_metadata,
                RolloutMetadataRead::Missing => continue,
                RolloutMetadataRead::Locked => {
                    metadata
                        .locked_rollouts
                        .push(rollout_path.to_string_lossy().to_string());
                    continue;
                }
            };
        if scan_user_event && rollout_metadata.has_user_event {
            metadata.user_event_thread_ids.insert(thread_id.to_string());
        }
        if wants_cwd {
            if let Some(cwd) = rollout_metadata.cwd {
                metadata.cwd_by_thread_id.insert(thread_id.to_string(), cwd);
            }
        }
    }
    Ok(metadata)
}

fn state_thread_rollout_path(root: &Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path.trim());
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

/// Streams a rollout line by line and stops as soon as the requested facts are known: the cwd of
/// the first `session_meta` line (normally line 1) and whether any line mentions a user event.
/// The whole file is only read when a wanted fact is absent.
pub(super) fn read_rollout_thread_metadata(
    path: &Path,
    scan_user_event: bool,
    read_cwd: bool,
) -> Result<RolloutMetadataRead, String> {
    let read_error = |err: std::io::Error| {
        format!(
            "读取 Codex state 会话 rollout 元数据失败 {}: {err}",
            path.display()
        )
    };
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(RolloutMetadataRead::Missing),
        Err(err) if is_locked_io_error(&err) => return Ok(RolloutMetadataRead::Locked),
        Err(err) => return Err(read_error(err)),
    };
    let mut reader = BufReader::new(file);
    let mut metadata = RolloutThreadMetadata::default();
    let mut need_user_event = scan_user_event;
    let mut need_cwd = read_cwd;
    let mut line = Vec::new();
    while need_user_event || need_cwd {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(err) if is_locked_io_error(&err) => return Ok(RolloutMetadataRead::Locked),
            Err(err) => return Err(read_error(err)),
        }
        if need_user_event
            && USER_EVENT_MARKERS
                .iter()
                .any(|marker| contains_bytes(&line, marker))
        {
            metadata.has_user_event = true;
            need_user_event = false;
        }
        if need_cwd && contains_bytes(&line, SESSION_META_MARKER) {
            if let Some(cwd) = session_meta_line_cwd(&line) {
                metadata.cwd = cwd;
                need_cwd = false;
            }
        }
    }
    Ok(RolloutMetadataRead::Read(metadata))
}

/// `Some(cwd)` when the line is a `session_meta` event with an object payload (the first such
/// line decides, even without a cwd); `None` to keep looking.
fn session_meta_line_cwd(line: &[u8]) -> Option<Option<String>> {
    let event = serde_json::from_slice::<Value>(line).ok()?;
    if event.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let payload = event.get("payload").and_then(Value::as_object)?;
    Some(
        payload
            .get("cwd")
            .and_then(Value::as_str)
            .and_then(to_desktop_workspace_path),
    )
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn is_locked_io_error(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::PermissionDenied)
        || matches!(error.raw_os_error(), Some(32 | 33))
}

#[cfg(test)]
pub(super) fn sync_codex_state_threads_to_provider(
    state_db: &Path,
    target_provider: &str,
) -> Result<usize, String> {
    sync_codex_state_threads_to_provider_with_diagnostics(state_db, target_provider, None)
}

pub(super) fn sync_codex_state_threads_to_provider_with_diagnostics(
    state_db: &Path,
    target_provider: &str,
    trigger: Option<&str>,
) -> Result<usize, String> {
    if !state_db.exists() {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_state_db_missing",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string()
                }),
            );
        }
        return Ok(0);
    }
    let mut connection = Connection::open_with_flags(
        state_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| format!("打开 Codex state 数据库失败 {}: {err}", state_db.display()))?;
    connection
        .busy_timeout(Duration::from_millis(3000))
        .map_err(|err| format!("配置 Codex state 数据库等待超时失败: {err}"))?;
    let Some(columns) = state_threads_columns(&connection)? else {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_state_db_threads_missing",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string()
                }),
            );
        }
        return Ok(0);
    };
    if !columns.contains("model_provider") {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_state_db_threads_unsupported",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string(),
                    "missingColumn": "model_provider"
                }),
            );
        }
        return Ok(0);
    }
    let thread_metadata = collect_state_thread_sync_metadata(&connection, state_db, &columns)?;
    let before_summary = trigger
        .map(|_| query_state_db_summary(&connection, target_provider))
        .transpose();
    let transaction = connection
        .transaction()
        .map_err(|err| format!("开始 Codex state 会话同步事务失败: {err}"))?;
    let mut counts = StateThreadUpdateCounts {
        provider_rows: transaction
            .execute(
                "UPDATE threads
                 SET model_provider = ?1
                 WHERE model_provider IS NULL
                    OR model_provider <> ?1",
                [target_provider],
            )
            .map_err(|err| {
                format!(
                    "更新 Codex state 会话 provider 失败 {}: {err}",
                    state_db.display()
                )
            })?,
        ..StateThreadUpdateCounts::default()
    };
    if columns.contains("has_user_event") {
        for thread_id in &thread_metadata.user_event_thread_ids {
            counts.user_event_rows += transaction
                .execute(
                    "UPDATE threads
                     SET has_user_event = 1
                     WHERE id = ?1
                        AND COALESCE(has_user_event, 0) <> 1",
                    [thread_id],
                )
                .map_err(|err| {
                    format!(
                        "更新 Codex state 会话 user event 状态失败 {}: {err}",
                        state_db.display()
                    )
                })?;
        }
    }
    if columns.contains("cwd") {
        for (thread_id, cwd) in &thread_metadata.cwd_by_thread_id {
            counts.cwd_rows += transaction
                .execute(
                    "UPDATE threads
                     SET cwd = ?1
                     WHERE id = ?2
                        AND COALESCE(cwd, '') <> ?1",
                    (cwd, thread_id),
                )
                .map_err(|err| {
                    format!(
                        "更新 Codex state 会话 cwd 失败 {}: {err}",
                        state_db.display()
                    )
                })?;
        }
    }
    transaction
        .commit()
        .map_err(|err| format!("保存 Codex state 会话同步结果失败: {err}"))?;
    let updated = counts.total();
    log_state_db_update_summary(
        state_db,
        target_provider,
        trigger,
        counts,
        &thread_metadata.locked_rollouts,
        before_summary,
    );
    Ok(updated)
}

pub(super) fn preview_codex_state_threads_to_provider_with_diagnostics(
    state_db: &Path,
    target_provider: &str,
    trigger: Option<&str>,
    count_metadata_updates: bool,
) -> Result<usize, String> {
    if !state_db.exists() {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_preflight_state_db_missing",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string()
                }),
            );
        }
        return Ok(0);
    }
    let connection = Connection::open_with_flags(
        state_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| format!("打开 Codex state 数据库失败 {}: {err}", state_db.display()))?;
    connection
        .busy_timeout(Duration::from_millis(3000))
        .map_err(|err| format!("配置 Codex state 数据库等待超时失败: {err}"))?;
    let Some(columns) = state_threads_columns(&connection)? else {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_preflight_state_db_threads_missing",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string()
                }),
            );
        }
        return Ok(0);
    };
    if !columns.contains("model_provider") {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_preflight_state_db_threads_unsupported",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string(),
                    "missingColumn": "model_provider"
                }),
            );
        }
        return Ok(0);
    }
    let thread_metadata = if count_metadata_updates {
        collect_state_thread_sync_metadata(&connection, state_db, &columns)?
    } else {
        StateThreadSyncMetadata::default()
    };
    let mut counts = StateThreadUpdateCounts {
        provider_rows: connection
            .query_row(
                "SELECT COUNT(*) FROM threads
                 WHERE model_provider IS NULL
                    OR model_provider <> ?1",
                [target_provider],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|err| {
                format!(
                    "统计 Codex state 会话 provider 待同步数量失败 {}: {err}",
                    state_db.display()
                )
            })? as usize,
        ..StateThreadUpdateCounts::default()
    };
    if columns.contains("has_user_event") {
        for thread_id in &thread_metadata.user_event_thread_ids {
            counts.user_event_rows += connection
                .query_row(
                    "SELECT COUNT(*) FROM threads
                     WHERE id = ?1
                        AND COALESCE(has_user_event, 0) <> 1",
                    [thread_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|err| {
                    format!(
                        "统计 Codex state 会话 user event 待同步数量失败 {}: {err}",
                        state_db.display()
                    )
                })? as usize;
        }
    }
    if columns.contains("cwd") {
        for (thread_id, cwd) in &thread_metadata.cwd_by_thread_id {
            counts.cwd_rows += connection
                .query_row(
                    "SELECT COUNT(*) FROM threads
                     WHERE id = ?1
                        AND COALESCE(cwd, '') <> ?2",
                    (thread_id, cwd),
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|err| {
                    format!(
                        "统计 Codex state 会话 cwd 待同步数量失败 {}: {err}",
                        state_db.display()
                    )
                })? as usize;
        }
    }
    let detected = counts.total();
    let updated = if count_metadata_updates {
        detected
    } else {
        counts.provider_rows
    };
    if let Some(trigger) = trigger {
        log_session_sync_event(
            "session_sync_preflight_state_db_summary",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "stateDb": state_db.to_string_lossy().to_string(),
                "providerRowsUpdated": counts.provider_rows,
                "userEventRowsUpdated": counts.user_event_rows,
                "cwdRowsUpdated": counts.cwd_rows,
                "detected": detected,
                "countMetadataUpdates": count_metadata_updates,
                "lockedRollouts": thread_metadata.locked_rollouts.len(),
                "lockedRolloutPaths": thread_metadata.locked_rollouts,
                "updated": updated
            }),
        );
    }
    Ok(updated)
}

fn log_state_db_update_summary(
    state_db: &Path,
    target_provider: &str,
    trigger: Option<&str>,
    counts: StateThreadUpdateCounts,
    locked_rollouts: &[String],
    summary: Result<Option<Value>, String>,
) {
    let Some(trigger) = trigger else {
        return;
    };
    let updated = counts.total();
    let summary = match summary {
        Ok(Some(summary)) => summary,
        Ok(None) if updated == 0 && locked_rollouts.is_empty() => return,
        Ok(None) => json!({}),
        Err(err) => json!({ "summaryError": err }),
    };
    if updated == 0 && locked_rollouts.is_empty() && summary.get("summaryError").is_none() {
        return;
    }
    log_session_sync_event(
        "session_sync_state_db_summary",
        json!({
            "trigger": trigger,
            "targetProvider": target_provider,
            "stateDb": state_db.to_string_lossy().to_string(),
            "providerRowsUpdated": counts.provider_rows,
            "userEventRowsUpdated": counts.user_event_rows,
            "cwdRowsUpdated": counts.cwd_rows,
            "lockedRollouts": locked_rollouts.len(),
            "lockedRolloutPaths": locked_rollouts,
            "updated": updated,
            "summary": summary
        }),
    );
}

pub(super) fn query_state_db_summary(
    connection: &Connection,
    target_provider: &str,
) -> Result<Value, String> {
    let total_threads = connection
        .query_row("SELECT COUNT(*) FROM threads", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|err| format!("统计 Codex state threads 总数失败: {err}"))?;
    let target_threads = connection
        .query_row(
            "SELECT COUNT(*) FROM threads WHERE model_provider = ?1",
            [target_provider],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("统计 Codex state target provider 数量失败: {err}"))?;
    let would_update = connection
        .query_row(
            "SELECT COUNT(*) FROM threads
             WHERE model_provider IS NULL
                OR model_provider <> ?1",
            [target_provider],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("统计 Codex state 待同步数量失败: {err}"))?;
    let mut provider_counts_statement = connection
        .prepare(
            "SELECT COALESCE(model_provider, ''), COUNT(*)
             FROM threads
             GROUP BY COALESCE(model_provider, '')
             ORDER BY COUNT(*) DESC, COALESCE(model_provider, '')",
        )
        .map_err(|err| format!("统计 Codex state provider 分布失败: {err}"))?;
    let provider_counts = provider_counts_statement
        .query_map([], |row| {
            let provider = row.get::<_, String>(0)?;
            let threads = row.get::<_, i64>(1)?;
            Ok(json!({
                "provider": provider_log_value(&provider),
                "threads": threads
            }))
        })
        .map_err(|err| format!("读取 Codex state provider 分布失败: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("读取 Codex state provider 分布失败: {err}"))?;
    let mut provider_changes_statement = connection
        .prepare(
            "SELECT COALESCE(model_provider, ''), COUNT(*)
             FROM threads
             WHERE model_provider IS NULL
                OR model_provider <> ?1
             GROUP BY COALESCE(model_provider, '')
             ORDER BY COUNT(*) DESC, COALESCE(model_provider, '')",
        )
        .map_err(|err| format!("统计 Codex state provider 变更失败: {err}"))?;
    let provider_changes = provider_changes_statement
        .query_map([target_provider], |row| {
            let from_provider = row.get::<_, String>(0)?;
            let threads = row.get::<_, i64>(1)?;
            Ok(json!({
                "fromProvider": provider_log_value(&from_provider),
                "toProvider": target_provider,
                "threads": threads
            }))
        })
        .map_err(|err| format!("读取 Codex state provider 变更失败: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("读取 Codex state provider 变更失败: {err}"))?;
    Ok(json!({
        "totalThreads": total_threads,
        "targetProviderThreads": target_threads,
        "wouldUpdateThreads": would_update,
        "providerCounts": provider_counts,
        "providerChanges": provider_changes
    }))
}
