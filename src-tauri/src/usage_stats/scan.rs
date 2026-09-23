use super::{
    db::{
        db_error, find_owner_attribution, upsert_session_usage, SCAN_OUTCOME_BEFORE_START,
        SCAN_OUTCOME_DUPLICATE, SCAN_OUTCOME_IGNORED, SCAN_OUTCOME_INDEXED,
        SCAN_OUTCOME_MISSING_ATTRIBUTION,
    },
    model::{EstimatedCost, OwnerAttribution, ParsedSession, ScanWarnings, UsageScanSource},
    parse::parse_session_file,
    pricing::estimate_cost,
};
use crate::app_log::log_event_once;
use rusqlite::{params, Connection, Transaction};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// An unreadable session file is left out of the statistics. The page refreshes every 30 s,
/// so each distinct failure is recorded once per run.
fn log_scan_file_error(stage: &str, error: String) {
    log_event_once(
        "usage_stats_scan_file_error",
        json!({ "stage": stage, "error": error, "handling": "skipped" }),
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SessionFileStamp {
    modified_nanos: i64,
    size: u64,
}

#[derive(Clone, Debug)]
pub(super) struct SessionScanState {
    stamp: SessionFileStamp,
    scan_scope: String,
    session_id: String,
    outcome: String,
}

/// What the scan decided for one changed session file. Nothing is written while the scan reads
/// and parses files; `write_session_scan_results` writes these later in one transaction.
pub(super) struct SessionScanWrite {
    source_path: String,
    stamp: SessionFileStamp,
    scan_scope: String,
    session_id: String,
    outcome: &'static str,
    indexed: Option<IndexedSession>,
}

struct IndexedSession {
    path: PathBuf,
    parsed: ParsedSession,
    attribution: OwnerAttribution,
    estimated: EstimatedCost,
}

/// Walks one source and queues a write for every file whose stamp or scope changed. Returns
/// whether anything was queued (or a changed file failed to parse), which is what tells
/// `AggregateCache` that the previous summary can no longer be reused.
pub(super) fn scan_codex_sessions(
    connection: &Connection,
    source: &UsageScanSource,
    stats_started_at_seconds: i64,
    warnings: &mut ScanWarnings,
    scan_states: &HashMap<String, SessionScanState>,
    writes: &mut Vec<SessionScanWrite>,
) -> Result<bool, String> {
    let files = collect_session_files(&source.codex_home)?;
    let mut changed = false;
    let scan_scope = session_scan_scope(source, stats_started_at_seconds);
    let mut seen_session_ids = HashSet::new();
    for path in files {
        let source_path = path.to_string_lossy().into_owned();
        let stamp = match session_file_stamp(&path) {
            Ok(stamp) => stamp,
            Err(err) => {
                log_scan_file_error("stamp", err);
                continue;
            }
        };
        if let Some(state) = scan_states.get(&source_path) {
            let duplicate_needs_promotion = state.outcome == SCAN_OUTCOME_DUPLICATE
                && !state.session_id.is_empty()
                && !seen_session_ids.contains(&state.session_id);
            if state.stamp == stamp && state.scan_scope == scan_scope && !duplicate_needs_promotion
            {
                // 稳态只读取目录项和 metadata，不再重复顺序读取整份 JSONL。
                apply_cached_scan_state(state, warnings, &mut seen_session_ids);
                continue;
            }
        }
        // everything below this point queues a database write
        changed = true;

        let parsed = match parse_session_file(&path) {
            Ok(parsed) => parsed,
            Err(err) => {
                log_scan_file_error("parse", err);
                continue;
            }
        };
        let mut queue = |session_id: &str, outcome: &'static str, indexed| {
            writes.push(SessionScanWrite {
                source_path: source_path.clone(),
                stamp,
                scan_scope: scan_scope.clone(),
                session_id: session_id.to_string(),
                outcome,
                indexed,
            });
        };
        if parsed.session_id.is_empty() || parsed.usage.is_none() || parsed.started_at.is_none() {
            queue("", SCAN_OUTCOME_IGNORED, None);
            continue;
        }
        if !seen_session_ids.insert(parsed.session_id.clone()) {
            queue(&parsed.session_id, SCAN_OUTCOME_DUPLICATE, None);
            continue;
        }
        let started_at = parsed.started_at.as_ref().expect("checked above");
        let updated_at = parsed.updated_at.as_ref().unwrap_or(started_at);
        if updated_at.seconds < stats_started_at_seconds {
            warnings.skipped_before_start += 1;
            queue(&parsed.session_id, SCAN_OUTCOME_BEFORE_START, None);
            continue;
        }
        let attribution = if let Some(attribution) = source.attribution_override.as_ref() {
            attribution.clone()
        } else {
            let Some(attribution) =
                find_owner_attribution(connection, &parsed.provider, started_at.seconds)?
            else {
                warnings.missing_attribution += 1;
                queue(&parsed.session_id, SCAN_OUTCOME_MISSING_ATTRIBUTION, None);
                continue;
            };
            attribution
        };
        let usage = parsed.usage.as_ref().expect("checked above");
        let estimated = estimate_cost(&parsed.model, usage, parsed.model_context_window);
        let session_id = parsed.session_id.clone();
        queue(
            &session_id,
            SCAN_OUTCOME_INDEXED,
            Some(IndexedSession {
                path,
                parsed,
                attribution,
                estimated,
            }),
        );
    }
    Ok(changed)
}

/// Writes the queued results of one refresh, in scan order, inside the caller's transaction. A
/// failure leaves the transaction to roll back, so the scan states still show these files as
/// changed and the next refresh repeats the work.
pub(super) fn write_session_scan_results(
    transaction: &Transaction<'_>,
    writes: &[SessionScanWrite],
    now: &str,
) -> Result<(), String> {
    for write in writes {
        if let Some(indexed) = write.indexed.as_ref() {
            let usage = indexed
                .parsed
                .usage
                .as_ref()
                .expect("indexed sessions have usage");
            upsert_session_usage(
                transaction,
                &indexed.path,
                &indexed.parsed,
                usage,
                &indexed.attribution,
                &indexed.estimated,
                now,
            )?;
        }
        upsert_session_scan_state(transaction, write, now)?;
    }
    Ok(())
}

fn session_scan_scope(source: &UsageScanSource, stats_started_at_seconds: i64) -> String {
    match source.attribution_override.as_ref() {
        Some(attribution) => format!(
            "{stats_started_at_seconds}:{}:{}",
            attribution.owner_type, attribution.owner_id
        ),
        None => format!("{stats_started_at_seconds}:automatic"),
    }
}

fn session_file_stamp(path: &Path) -> Result<SessionFileStamp, String> {
    let metadata = fs::metadata(path).map_err(|err| {
        format!(
            "读取 Codex session 文件元数据失败 {}: {err}",
            path.display()
        )
    })?;
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    Ok(SessionFileStamp {
        modified_nanos,
        size: metadata.len(),
    })
}

pub(super) fn load_session_scan_states(
    connection: &Connection,
) -> Result<HashMap<String, SessionScanState>, String> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT source_path, modified_nanos, file_size, scan_scope, session_id, outcome
            FROM session_scan_state
            "#,
        )
        .map_err(|err| db_error("读取 token 统计扫描缓存失败", err))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                SessionScanState {
                    stamp: SessionFileStamp {
                        modified_nanos: row.get(1)?,
                        size: row
                            .get::<_, i64>(2)
                            .map(|value| u64::try_from(value).unwrap_or(0))?,
                    },
                    scan_scope: row.get(3)?,
                    session_id: row.get(4)?,
                    outcome: row.get(5)?,
                },
            ))
        })
        .map_err(|err| db_error("读取 token 统计扫描缓存失败", err))?;
    rows.collect::<Result<HashMap<_, _>, _>>()
        .map_err(|err| db_error("读取 token 统计扫描缓存失败", err))
}

fn upsert_session_scan_state(
    transaction: &Transaction<'_>,
    write: &SessionScanWrite,
    now: &str,
) -> Result<(), String> {
    transaction
        .prepare_cached(
            r#"
            INSERT INTO session_scan_state(
                source_path, modified_nanos, file_size, scan_scope,
                session_id, outcome, last_scanned_at
            )
            VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(source_path) DO UPDATE SET
                modified_nanos = excluded.modified_nanos,
                file_size = excluded.file_size,
                scan_scope = excluded.scan_scope,
                session_id = excluded.session_id,
                outcome = excluded.outcome,
                last_scanned_at = excluded.last_scanned_at
            "#,
        )
        .and_then(|mut statement| {
            statement.execute(params![
                write.source_path,
                write.stamp.modified_nanos,
                i64::try_from(write.stamp.size).unwrap_or(i64::MAX),
                write.scan_scope,
                write.session_id,
                write.outcome,
                now
            ])
        })
        .map_err(|err| db_error("写入 token 统计扫描缓存失败", err))?;
    Ok(())
}

fn apply_cached_scan_state(
    state: &SessionScanState,
    warnings: &mut ScanWarnings,
    seen_session_ids: &mut HashSet<String>,
) {
    match state.outcome.as_str() {
        SCAN_OUTCOME_MISSING_ATTRIBUTION => warnings.missing_attribution += 1,
        SCAN_OUTCOME_BEFORE_START => warnings.skipped_before_start += 1,
        _ => {}
    }
    if !state.session_id.is_empty() && state.outcome != SCAN_OUTCOME_IGNORED {
        seen_session_ids.insert(state.session_id.clone());
    }
}

fn collect_session_files(codex_home: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    collect_jsonl_files_recursive(&codex_home.join("sessions"), &mut files)?;
    collect_jsonl_files_recursive(&codex_home.join("archived_sessions"), &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_jsonl_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(dir)
        .map_err(|err| format!("读取 Codex session 目录失败 {}: {err}", dir.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|err| format!("读取 Codex session 目录失败 {}: {err}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files_recursive(&path, files)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}
