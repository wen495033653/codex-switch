use super::{
    db::{
        delete_cursor, find_owner_attribution, insert_record, load_cursors, save_cursor, FileStamp,
    },
    model::{OwnerAttribution, UsageRecord, UsageScanSource, OWNER_TYPE_UNATTRIBUTED},
    pricing::{estimate_request_cost, ServiceTier},
    records::{read_new_records, RolloutCursor},
};
use crate::app_log::{log_event, log_event_once};
use rusqlite::{Connection, Transaction};
use serde_json::json;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// What one refresh read from the rollout files. Nothing is written while the files are read;
/// `write_scan` stores all of it later in one transaction.
#[derive(Default)]
pub(super) struct ScanResult {
    records: Vec<UsageRecord>,
    cursors: Vec<(String, FileStamp, RolloutCursor)>,
    stale_cursors: Vec<String>,
}

/// An unreadable file is left for the next refresh. The page refreshes every 30 s, so each
/// distinct failure is recorded once per run.
fn log_scan_file_error(stage: &str, path: &str, error: String) {
    log_event_once(
        "usage_stats_scan_file_error",
        json!({ "stage": stage, "path": path, "error": error, "handling": "retry_next_refresh" }),
    );
}

/// Reads what every rollout file gained since the last refresh. A file never read before is
/// only opened once it was modified after `counted_after_seconds`: nothing older is counted.
pub(super) fn scan_sources(
    connection: &Connection,
    sources: &[UsageScanSource],
    counted_after_seconds: i64,
) -> Result<ScanResult, String> {
    let known = load_cursors(connection)?;
    let counted_after_nanos = counted_after_seconds
        .checked_mul(1_000_000_000)
        .ok_or_else(|| format!("token 统计记录起点超出范围: {counted_after_seconds}"))?;
    let mut result = ScanResult::default();
    let mut present = HashSet::new();
    for source in sources {
        for path in collect_rollout_files(&source.codex_home)? {
            let source_path = path.to_string_lossy().into_owned();
            present.insert(source_path.clone());
            let stamp = match file_stamp(&path) {
                Ok(stamp) => stamp,
                Err(err) => {
                    log_scan_file_error("stamp", &source_path, err);
                    continue;
                }
            };
            let previous = known.get(&source_path);
            match previous {
                Some((known_stamp, _)) if *known_stamp == stamp => continue,
                None if stamp.modified_nanos <= counted_after_nanos => continue,
                _ => {}
            }
            let previous_cursor = previous.map(|(_, cursor)| cursor);
            let read = match read_new_records(&path, previous_cursor) {
                Ok(read) => read,
                Err(err) => {
                    log_scan_file_error("read", &source_path, err.to_string());
                    continue;
                }
            };
            if read.restarted {
                log_event(
                    "usage_stats_rollout_reread",
                    json!({
                        "path": source_path,
                        "previousOffset": previous_cursor.map(|cursor| cursor.offset),
                        "reason": "resume_check_mismatch"
                    }),
                );
            }
            for skipped in &read.skipped {
                log_event_once(
                    "usage_stats_record_line_error",
                    json!({
                        "path": source_path,
                        "offset": skipped.offset,
                        "error": skipped.reason,
                        "handling": "skipped"
                    }),
                );
            }
            for record in read.records {
                if record.timestamp_seconds <= counted_after_seconds {
                    continue;
                }
                let owner = match source.attribution_override.as_ref() {
                    Some(owner) => owner.clone(),
                    None => find_owner_attribution(
                        connection,
                        &record.provider,
                        record.timestamp_seconds,
                    )?
                    .unwrap_or(OwnerAttribution {
                        owner_type: OWNER_TYPE_UNATTRIBUTED.to_string(),
                        owner_id: String::new(),
                    }),
                };
                let cost = estimate_request_cost(
                    &record.model,
                    &record.usage,
                    ServiceTier::from_setting(&record.service_tier),
                );
                result.records.push(UsageRecord {
                    response_id: record.response_id,
                    thread_id: record.thread_id,
                    timestamp_seconds: record.timestamp_seconds,
                    owner,
                    model: record.model,
                    usage: record.usage,
                    cost,
                });
            }
            result.cursors.push((source_path, stamp, read.cursor));
        }
    }
    result.stale_cursors = known
        .into_keys()
        .filter(|source_path| !present.contains(source_path))
        .collect();
    Ok(result)
}

/// Stores one refresh inside the caller's transaction. A failure leaves the transaction to roll
/// back, so the cursors still point before these records and the next refresh reads them again.
pub(super) fn write_scan(transaction: &Transaction<'_>, scan: &ScanResult) -> Result<(), String> {
    for record in &scan.records {
        insert_record(transaction, record, true)?;
    }
    for (source_path, stamp, cursor) in &scan.cursors {
        save_cursor(transaction, source_path, *stamp, cursor)?;
    }
    for source_path in &scan.stale_cursors {
        delete_cursor(transaction, source_path)?;
    }
    Ok(())
}

fn file_stamp(path: &Path) -> Result<FileStamp, String> {
    let metadata = fs::metadata(path).map_err(|err| {
        format!(
            "读取 Codex session 文件元数据失败 {}: {err}",
            path.display()
        )
    })?;
    let modified = metadata.modified().map_err(|err| {
        format!(
            "读取 Codex session 文件修改时间失败 {}: {err}",
            path.display()
        )
    })?;
    let modified_nanos = modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .map_err(|err| {
            format!(
                "Codex session 文件修改时间早于 1970 年 {}: {err}",
                path.display()
            )
        })?;
    Ok(FileStamp {
        modified_nanos,
        size: metadata.len(),
    })
}

fn collect_rollout_files(codex_home: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    collect_rollout_files_recursive(&codex_home.join("sessions"), &mut files)?;
    collect_rollout_files_recursive(&codex_home.join("archived_sessions"), &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_rollout_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
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
            collect_rollout_files_recursive(&path, files)?;
        } else if is_rollout_jsonl(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_rollout_jsonl(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.starts_with("rollout-"))
        && path.extension().and_then(|value| value.to_str()) == Some("jsonl")
}
