use super::{
    db::{
        delete_cursor, find_owner_attribution, insert_record, load_cursors, reprice_stored_record,
        save_cursor, FileStamp,
    },
    model::{OwnerAttribution, ServiceTier, UsageRecord, UsageScanSource, OWNER_TYPE_UNATTRIBUTED},
    pricing::estimate_request_cost,
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
    /// Files read again from the start for a price check.
    pub(super) reread_files: usize,
}

/// What `write_scan` stored.
pub(super) struct ScanWrite {
    /// Records stored without a price that this refresh priced.
    pub(super) repriced: usize,
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
/// Files modified at or after `reread_since` are read again from the start, so the records they
/// hold are seen again (with the tier they ran on) for a price check.
pub(super) fn scan_sources(
    connection: &Connection,
    sources: &[UsageScanSource],
    counted_after_seconds: i64,
    reread_since: Option<i64>,
) -> Result<ScanResult, String> {
    let known = load_cursors(connection)?;
    let to_nanos = |seconds: i64| {
        seconds
            .checked_mul(1_000_000_000)
            .ok_or_else(|| format!("token 统计时间超出范围: {seconds}"))
    };
    let counted_after_nanos = to_nanos(counted_after_seconds)?;
    let reread_since_nanos = reread_since.map(to_nanos).transpose()?;
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
            let reread = reread_since_nanos.is_some_and(|since| stamp.modified_nanos >= since);
            let previous = known.get(&source_path);
            match previous {
                Some((known_stamp, _)) if !reread && *known_stamp == stamp => continue,
                None if stamp.modified_nanos <= counted_after_nanos => continue,
                _ => {}
            }
            if reread {
                result.reread_files += 1;
            }
            let previous_cursor = previous.filter(|_| !reread).map(|(_, cursor)| cursor);
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
                let service_tier = ServiceTier::from_setting(&record.service_tier);
                let cost = estimate_request_cost(
                    &record.model,
                    &record.usage,
                    service_tier,
                    &owner.owner_type,
                );
                result.records.push(UsageRecord {
                    response_id: record.response_id,
                    thread_id: record.thread_id,
                    timestamp_seconds: record.timestamp_seconds,
                    owner,
                    model: record.model,
                    usage: record.usage,
                    service_tier,
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
pub(super) fn write_scan(
    transaction: &Transaction<'_>,
    scan: &ScanResult,
) -> Result<ScanWrite, String> {
    let mut repriced = 0;
    for record in &scan.records {
        // A response seen again (a fork's copy, a rewritten or re-read file) may be one stored
        // while its model had no price.
        if !insert_record(transaction, record, true)? && reprice_stored_record(transaction, record)?
        {
            repriced += 1;
        }
    }
    for (source_path, stamp, cursor) in &scan.cursors {
        save_cursor(transaction, source_path, *stamp, cursor)?;
    }
    for source_path in &scan.stale_cursors {
        delete_cursor(transaction, source_path)?;
    }
    Ok(ScanWrite { repriced })
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
        // The entry's type comes with the directory listing; Path::is_dir would open each file.
        let file_type = entry
            .file_type()
            .map_err(|err| format!("读取 Codex session 条目类型失败 {}: {err}", path.display()))?;
        if file_type.is_dir() {
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
