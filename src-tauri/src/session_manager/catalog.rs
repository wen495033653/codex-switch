use super::{
    codex_home::{
        conversation_path_key, ensure_session_relative_path, extract_uuid_like,
        normalize_relative_path, path_to_slash, relative_path_under_root, resolve_codex_root,
        session_index_title, validate_codex_root, SessionIndex,
    },
    model::{ConversationItem, SessionSummary},
    rollout::parse_session_file_for_list,
    state_db::{
        state_database_has_current_migrations, state_threads_schema, CURRENT_STATE_REQUIRED_COLUMNS,
    },
    util::{
        non_empty, system_time_to_rfc3339, timestamp_millis_to_rfc3339,
        timestamp_seconds_to_rfc3339, truncate_text,
    },
};
use crate::{
    codex_app_server::{list_interactive_threads, CodexDesktopThread},
    paths::codex_state_db_path_for_root,
    time_util::parse_rfc3339_seconds,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug)]
pub(super) struct CurrentStateCatalog {
    pub(super) conversations: Vec<ConversationItem>,
    pub(super) warnings: Vec<String>,
}

pub(super) fn scan_conversations_impl(root: Option<String>) -> Result<Value, String> {
    let root = resolve_codex_root(root.as_deref())?;
    validate_codex_root(&root)?;
    let desktop_threads = list_interactive_threads(&root)
        .map_err(|err| format!("通过 Codex Desktop 查询会话失败: {err}"))?;
    let (mut conversations, warnings, errors) =
        conversations_from_desktop_threads(&root, desktop_threads);

    conversations.sort_by(|a, b| {
        conversation_sort_key(b)
            .cmp(&conversation_sort_key(a))
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });

    Ok(json!({
        "ok": true,
        "root": root.to_string_lossy().to_string(),
        "conversations": conversations,
        "warnings": warnings,
        "errors": errors
    }))
}

pub(super) fn conversations_from_desktop_threads(
    root: &Path,
    desktop_threads: Vec<CodexDesktopThread>,
) -> (Vec<ConversationItem>, Vec<String>, Vec<String>) {
    let mut conversations = Vec::with_capacity(desktop_threads.len());
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_paths = HashSet::new();

    for thread in desktop_threads {
        let id = thread.id.clone();
        let path_key = conversation_path_key(&thread.path);
        if seen_ids.contains(&id) || seen_paths.contains(&path_key) {
            warnings.push(format!("已忽略 Codex Desktop 返回的重复会话: {id}"));
            continue;
        }

        let relative = match relative_path_under_root(root, &thread.path) {
            Some(relative) => relative,
            None => {
                errors.push(format!(
                    "Codex Desktop 返回了当前数据目录外的会话路径 {}: {}",
                    id,
                    thread.path.display()
                ));
                continue;
            }
        };
        if let Err(err) = ensure_session_relative_path(&relative) {
            errors.push(format!("Codex Desktop 返回了无效会话路径 {id}: {err}"));
            continue;
        }
        let metadata = match thread.path.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => {
                errors.push(format!(
                    "Codex Desktop 会话路径不是文件 {}: {}",
                    id,
                    thread.path.display()
                ));
                continue;
            }
            Err(err) => {
                errors.push(format!(
                    "读取 Codex Desktop 会话文件失败 {}: {}: {err}",
                    id,
                    thread.path.display()
                ));
                continue;
            }
        };

        seen_ids.insert(id.clone());
        seen_paths.insert(path_key);

        let preview = non_empty(thread.preview);
        let title = thread
            .name
            .and_then(non_empty)
            .or_else(|| preview.clone().map(|value| truncate_text(&value, 48)))
            .unwrap_or_else(|| id.clone());
        let updated_at = thread
            .recency_at
            .or(Some(thread.updated_at))
            .and_then(timestamp_seconds_to_rfc3339)
            .or_else(|| system_time_to_rfc3339(metadata.modified().ok()));

        conversations.push(ConversationItem {
            id,
            title,
            updated_at,
            status: if thread.archived {
                "archived".to_string()
            } else {
                "active".to_string()
            },
            source_path: thread.path.to_string_lossy().to_string(),
            relative_path: path_to_slash(&relative),
            size_bytes: metadata.len(),
            cwd: Some(thread.cwd.to_string_lossy().to_string()),
            preview,
            sha256: None,
            parse_error: None,
        });
    }

    (conversations, warnings, errors)
}

pub(super) fn conversation_from_path(
    root: &Path,
    path: &Path,
    status: &str,
    session_index: &SessionIndex,
) -> Result<ConversationItem, String> {
    let metadata = fs::metadata(path)
        .map_err(|err| format!("读取会话文件信息失败 {}: {err}", path.display()))?;
    let summary = parse_session_file_for_list(path).unwrap_or_else(|err| SessionSummary {
        parse_error: Some(err),
        ..SessionSummary::default()
    });
    let relative_path = path
        .strip_prefix(root)
        .map(path_to_slash)
        .unwrap_or_else(|_| path.to_string_lossy().to_string());
    let id = summary
        .id
        .clone()
        .or_else(|| extract_uuid_like(&relative_path))
        .unwrap_or_else(|| relative_path.clone());
    let title = session_index_title(session_index, &id)
        .or_else(|| {
            summary.title.clone().or_else(|| {
                summary
                    .first_user_message
                    .clone()
                    .map(|text| truncate_text(&text, 48))
            })
        })
        .unwrap_or_else(|| "未命名会话".to_string());
    let updated_at = summary
        .updated_at
        .clone()
        .or_else(|| system_time_to_rfc3339(metadata.modified().ok()));
    Ok(ConversationItem {
        id,
        title,
        updated_at,
        status: status.to_string(),
        source_path: path.to_string_lossy().to_string(),
        relative_path,
        size_bytes: metadata.len(),
        cwd: summary.cwd,
        preview: summary.preview,
        sha256: None,
        parse_error: summary.parse_error,
    })
}

const CURRENT_STATE_THREADS_QUERY: &str = "SELECT id,
        rollout_path,
        COALESCE(title, ''),
        COALESCE(preview, ''),
        COALESCE(cwd, ''),
        COALESCE(archived, 0),
        CASE
          WHEN COALESCE(recency_at_ms, 0) > 0 THEN recency_at_ms
          WHEN COALESCE(updated_at_ms, 0) > 0 THEN updated_at_ms
          WHEN COALESCE(recency_at, 0) > 0 THEN recency_at * 1000
          ELSE COALESCE(updated_at, 0) * 1000
        END AS effective_updated_at_ms
 FROM threads
 WHERE rollout_path IS NOT NULL AND TRIM(rollout_path) <> ''
 ORDER BY recency_at_ms DESC, id DESC";

struct StateThreadRow {
    id: String,
    rollout_path: String,
    title: String,
    preview: String,
    cwd: String,
    archived: i64,
    updated_at_ms: i64,
}

fn open_current_state_db(root: &Path) -> Result<Connection, String> {
    let state_db = codex_state_db_path_for_root(root)?;
    if !state_db.exists() {
        return Err(format!(
            "未检测到新版 Codex 数据库 {}，请先启动新版 ChatGPT Desktop 完成初始化",
            state_db.display()
        ));
    }
    let connection = Connection::open_with_flags(
        &state_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| {
        format!(
            "打开新版 Codex state 数据库失败 {}: {err}",
            state_db.display()
        )
    })?;
    connection
        .busy_timeout(Duration::from_millis(3000))
        .map_err(|err| format!("配置新版 Codex state 数据库等待超时失败: {err}"))?;
    let Some(schema) = state_threads_schema(&connection)? else {
        return Err("新版 Codex 数据库缺少 threads 表，请更新 ChatGPT Desktop".to_string());
    };
    if CURRENT_STATE_REQUIRED_COLUMNS
        .iter()
        .any(|column| !schema.contains_key(*column))
        || !state_database_has_current_migrations(&connection)?
    {
        return Err("ChatGPT Desktop 会话数据库结构过旧，请更新到最新版本".to_string());
    }
    Ok(connection)
}

/// Calls `visit` for every thread row in catalog order until it returns `true`.
fn visit_state_thread_rows(
    connection: &Connection,
    mut visit: impl FnMut(StateThreadRow) -> bool,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(CURRENT_STATE_THREADS_QUERY)
        .map_err(|err| format!("读取新版 Codex threads 目录失败: {err}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(StateThreadRow {
                id: row.get(0)?,
                rollout_path: row.get(1)?,
                title: row.get(2)?,
                preview: row.get(3)?,
                cwd: row.get(4)?,
                archived: row.get(5)?,
                updated_at_ms: row.get(6)?,
            })
        })
        .map_err(|err| format!("查询新版 Codex threads 目录失败: {err}"))?;
    for row in rows {
        if visit(row.map_err(|err| format!("解析新版 Codex thread 失败: {err}"))?) {
            break;
        }
    }
    Ok(())
}

fn conversation_item_from_state_row(
    row: StateThreadRow,
    path: &Path,
    relative: &Path,
    metadata: &fs::Metadata,
    session_index: &SessionIndex,
) -> ConversationItem {
    let preview = non_empty(row.preview);
    ConversationItem {
        title: session_index_title(session_index, &row.id)
            .or_else(|| non_empty(row.title))
            .or_else(|| preview.clone().map(|value| truncate_text(&value, 48)))
            .unwrap_or_else(|| row.id.clone()),
        id: row.id,
        updated_at: timestamp_millis_to_rfc3339(row.updated_at_ms)
            .or_else(|| system_time_to_rfc3339(metadata.modified().ok())),
        status: if row.archived == 0 {
            "active".to_string()
        } else {
            "archived".to_string()
        },
        source_path: path.to_string_lossy().to_string(),
        relative_path: path_to_slash(relative),
        size_bytes: metadata.len(),
        cwd: non_empty(row.cwd),
        preview,
        sha256: None,
        parse_error: None,
    }
}

pub(super) fn read_current_state_conversations(
    root: &Path,
    session_index: &SessionIndex,
) -> Result<CurrentStateCatalog, String> {
    let connection = open_current_state_db(root)?;
    let mut conversations = Vec::new();
    let mut indexed_paths = HashSet::new();
    let mut invalid_paths = 0usize;
    let mut duplicate_paths = 0usize;
    visit_state_thread_rows(&connection, |row| {
        let Some((path, relative)) = resolve_state_rollout_path(root, &row.rollout_path) else {
            invalid_paths += 1;
            return false;
        };
        let Ok(metadata) = path.metadata() else {
            return false;
        };
        if !metadata.is_file() {
            return false;
        }
        if !indexed_paths.insert(conversation_path_key(&path)) {
            duplicate_paths += 1;
            return false;
        }
        conversations.push(conversation_item_from_state_row(
            row,
            &path,
            &relative,
            &metadata,
            session_index,
        ));
        false
    })?;

    let mut warnings = Vec::new();
    if invalid_paths > 0 {
        warnings.push(format!(
            "已忽略 {invalid_paths} 条不属于当前 Codex 数据目录的新版索引"
        ));
    }
    if duplicate_paths > 0 {
        warnings.push(format!("已忽略 {duplicate_paths} 条重复的新版会话索引"));
    }
    Ok(CurrentStateCatalog {
        conversations,
        warnings,
    })
}

/// The catalog entry for one file, without building the catalog: rows are visited in catalog
/// order and only rows naming the same file get the filesystem calls (metadata, canonicalize)
/// the full catalog makes for every row. The first row resolving to the file wins, exactly as in
/// the catalog, where later rows for the same file are dropped as duplicates.
pub(super) fn current_state_conversation_for_path(
    root: &Path,
    path: &Path,
    session_index: &SessionIndex,
) -> Result<Option<ConversationItem>, String> {
    let target = conversation_path_key(path);
    let Some(target_name) = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
    else {
        return Ok(None);
    };
    let connection = open_current_state_db(root)?;
    let mut found = None;
    visit_state_thread_rows(&connection, |row| {
        let Some((row_path, relative)) = resolve_state_rollout_path(root, &row.rollout_path) else {
            return false;
        };
        if !same_file_name(&row_path, &target_name) {
            return false;
        }
        let Ok(metadata) = row_path.metadata() else {
            return false;
        };
        if !metadata.is_file() || conversation_path_key(&row_path) != target {
            return false;
        }
        found = Some(conversation_item_from_state_row(
            row,
            &row_path,
            &relative,
            &metadata,
            session_index,
        ));
        true
    })?;
    Ok(found)
}

/// Same comparison `conversation_path_key` applies to the final component (ASCII case-insensitive
/// on Windows).
fn same_file_name(path: &Path, target_name: &str) -> bool {
    let Some(name) = path.file_name().map(|name| name.to_string_lossy()) else {
        return false;
    };
    if cfg!(windows) {
        name.eq_ignore_ascii_case(target_name)
    } else {
        name == target_name
    }
}

fn resolve_state_rollout_path(root: &Path, rollout_path: &str) -> Option<(PathBuf, PathBuf)> {
    let raw = rollout_path.trim();
    if raw.is_empty() {
        return None;
    }
    let raw = raw.strip_prefix(r"\\?\").unwrap_or(raw);
    let candidate = PathBuf::from(raw);
    let (path, relative) = if candidate.is_absolute() {
        let relative = relative_path_under_root(root, &candidate)?;
        (candidate, relative)
    } else {
        let normalized = normalize_relative_path(&path_to_slash(&candidate)).ok()?;
        (root.join(&normalized), normalized)
    };
    ensure_session_relative_path(&relative).ok()?;
    Some((path, relative))
}

fn conversation_sort_key(item: &ConversationItem) -> i64 {
    item.updated_at
        .as_deref()
        .and_then(parse_rfc3339_seconds)
        .unwrap_or(0)
}
