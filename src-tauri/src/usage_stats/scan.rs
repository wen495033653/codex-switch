use super::{
    db::{
        db_error, find_owner_attribution, upsert_session_usage, SCAN_OUTCOME_BEFORE_START,
        SCAN_OUTCOME_DUPLICATE, SCAN_OUTCOME_IGNORED, SCAN_OUTCOME_INDEXED,
        SCAN_OUTCOME_MISSING_ATTRIBUTION,
    },
    model::{ScanWarnings, UsageScanSource, UsageWindowStarts},
    parse::parse_session_file,
    pricing::estimate_cost,
};
use rusqlite::{params, Connection};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

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

pub(super) fn scan_codex_sessions(
    connection: &Connection,
    source: &UsageScanSource,
    window_starts: &UsageWindowStarts,
    stats_started_at_seconds: i64,
    now: &str,
    warnings: &mut ScanWarnings,
    scan_states: &mut HashMap<String, SessionScanState>,
) -> Result<(), String> {
    let files = collect_session_files(&source.codex_home)?;
    let scan_scope = session_scan_scope(source, stats_started_at_seconds);
    let mut seen_session_ids = HashSet::new();
    for path in files {
        let source_path = path.to_string_lossy().into_owned();
        let stamp = match session_file_stamp(&path) {
            Ok(stamp) => stamp,
            Err(err) => {
                eprintln!("{err}");
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

        let parsed = match parse_session_file(&path, window_starts) {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("{err}");
                continue;
            }
        };
        if parsed.session_id.is_empty() || parsed.usage.is_none() || parsed.started_at.is_none() {
            upsert_session_scan_state(
                connection,
                scan_states,
                &source_path,
                stamp,
                &scan_scope,
                "",
                SCAN_OUTCOME_IGNORED,
                now,
            )?;
            continue;
        }
        if !seen_session_ids.insert(parsed.session_id.clone()) {
            upsert_session_scan_state(
                connection,
                scan_states,
                &source_path,
                stamp,
                &scan_scope,
                &parsed.session_id,
                SCAN_OUTCOME_DUPLICATE,
                now,
            )?;
            continue;
        }
        let started_at = parsed.started_at.as_ref().expect("checked above");
        let updated_at = parsed.updated_at.as_ref().unwrap_or(started_at);
        if updated_at.seconds < stats_started_at_seconds {
            warnings.skipped_before_start += 1;
            upsert_session_scan_state(
                connection,
                scan_states,
                &source_path,
                stamp,
                &scan_scope,
                &parsed.session_id,
                SCAN_OUTCOME_BEFORE_START,
                now,
            )?;
            continue;
        }
        let attribution = if let Some(attribution) = source.attribution_override.as_ref() {
            attribution.clone()
        } else {
            let Some(attribution) =
                find_owner_attribution(connection, &parsed.provider, started_at.seconds)?
            else {
                warnings.missing_attribution += 1;
                upsert_session_scan_state(
                    connection,
                    scan_states,
                    &source_path,
                    stamp,
                    &scan_scope,
                    &parsed.session_id,
                    SCAN_OUTCOME_MISSING_ATTRIBUTION,
                    now,
                )?;
                continue;
            };
            attribution
        };
        let usage = parsed.usage.as_ref().expect("checked above");
        let estimated = estimate_cost(&parsed.model, usage, parsed.model_context_window);
        upsert_session_usage(
            connection,
            &path,
            &parsed,
            usage,
            &attribution,
            &estimated,
            now,
        )?;
        upsert_session_scan_state(
            connection,
            scan_states,
            &source_path,
            stamp,
            &scan_scope,
            &parsed.session_id,
            SCAN_OUTCOME_INDEXED,
            now,
        )?;
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

#[allow(clippy::too_many_arguments)]
fn upsert_session_scan_state(
    connection: &Connection,
    scan_states: &mut HashMap<String, SessionScanState>,
    source_path: &str,
    stamp: SessionFileStamp,
    scan_scope: &str,
    session_id: &str,
    outcome: &str,
    now: &str,
) -> Result<(), String> {
    connection
        .execute(
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
            params![
                source_path,
                stamp.modified_nanos,
                i64::try_from(stamp.size).unwrap_or(i64::MAX),
                scan_scope,
                session_id,
                outcome,
                now
            ],
        )
        .map_err(|err| db_error("写入 token 统计扫描缓存失败", err))?;
    scan_states.insert(
        source_path.to_string(),
        SessionScanState {
            stamp,
            scan_scope: scan_scope.to_string(),
            session_id: session_id.to_string(),
            outcome: outcome.to_string(),
        },
    );
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
