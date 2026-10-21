use super::{
    codex_logs::CodexLog,
    db::{
        delete_cursor, fill_record_timing, find_owner_attribution, insert_record, load_cursors,
        reprice_stored_record, save_cursor, FileStamp,
    },
    model::{
        OwnerAttribution, RequestTiming, ServiceTier, UsageRecord, UsageScanSource,
        OWNER_TYPE_UNATTRIBUTED,
    },
    pricing::estimate_request_cost,
    records::{read_new_records, RolloutCursor, RolloutRecord},
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
    /// Files read again from the start for a price check or the duration fill.
    pub(super) reread_files: usize,
    /// Responses of the last ten minutes with a known start, looked up in a Codex log.
    pub(super) recent_first_token_lookups: usize,
    /// How many of those the log had a first output item for.
    pub(super) recent_first_tokens_found: usize,
}

/// What `write_scan` stored.
pub(super) struct ScanWrite {
    /// Records stored without a price that this refresh priced.
    pub(super) repriced: usize,
    /// Records stored without a duration or first-token time that got one.
    pub(super) timings_filled: usize,
}

const RECENT_RESPONSE_MS: i64 = 10 * 60 * 1000;

/// The Codex log of one source, opened when the first response with a known start is read.
enum SourceLog {
    NotOpened,
    Open(CodexLog),
    Unavailable,
}

impl SourceLog {
    fn get(&mut self, codex_home: &Path) -> Option<&CodexLog> {
        if matches!(self, Self::NotOpened) {
            *self = match CodexLog::open(codex_home) {
                Ok(Some(log)) => Self::Open(log),
                Ok(None) => Self::Unavailable,
                Err(err) => {
                    log_codex_log_error(codex_home, err);
                    Self::Unavailable
                }
            };
        }
        match self {
            Self::Open(log) => Some(log),
            _ => None,
        }
    }
}

/// A Codex log that cannot be read leaves the first-token times empty; the statistics go on.
fn log_codex_log_error(codex_home: &Path, error: String) {
    log_event_once(
        "usage_stats_codex_log_error",
        json!({
            "codexHome": codex_home.display().to_string(),
            "error": error,
            "handling": "first_token_left_empty"
        }),
    );
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
    now_seconds: i64,
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
        let mut source_log = SourceLog::NotOpened;
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
                let timing = request_timing(
                    &record,
                    &mut source_log,
                    &source.codex_home,
                    now_seconds.saturating_mul(1000),
                    &mut result,
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
                    timing,
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

/// The duration from the rollout. The first-token time from the rollout when it says (as CC
/// Switch reads it), otherwise from the Codex log of the same home.
fn request_timing(
    record: &RolloutRecord,
    source_log: &mut SourceLog,
    codex_home: &Path,
    now_ms: i64,
    result: &mut ScanResult,
) -> RequestTiming {
    let Some(started_ms) = record.request_started_ms else {
        return RequestTiming::default();
    };
    let duration_ms = record.completed_ms - started_ms;
    // CC Switch's bound: a first token at the very start or after the end is a wrong start.
    let rollout_first_token = record
        .first_output_ms
        .map(|at| at - started_ms)
        .filter(|first_token| *first_token > 0 && *first_token <= duration_ms);
    if rollout_first_token.is_some() {
        return RequestTiming {
            duration_ms: Some(duration_ms),
            first_token_ms: rollout_first_token,
        };
    }
    let first_output = source_log.get(codex_home).and_then(|log| {
        log.first_output_ms(&record.thread_id, started_ms, record.completed_ms)
            .inspect_err(|err| log_codex_log_error(codex_home, err.clone()))
            .ok()
            .map(|found| (log, found))
    });
    if let Some((_, found)) = &first_output {
        if now_ms - record.completed_ms <= RECENT_RESPONSE_MS {
            result.recent_first_token_lookups += 1;
            result.recent_first_tokens_found += usize::from(found.is_some());
        }
    }
    RequestTiming {
        duration_ms: Some(duration_ms),
        first_token_ms: first_output
            .and_then(|(_, found)| found)
            .map(|at| at - started_ms),
    }
}

/// Stores one refresh inside the caller's transaction. A failure leaves the transaction to roll
/// back, so the cursors still point before these records and the next refresh reads them again.
pub(super) fn write_scan(
    transaction: &Transaction<'_>,
    scan: &ScanResult,
) -> Result<ScanWrite, String> {
    let mut repriced = 0;
    let mut timings_filled = 0;
    for record in &scan.records {
        if insert_record(transaction, record, true)? {
            continue;
        }
        // A response seen again (a fork's copy, a rewritten or re-read file) may be one stored
        // while its model had no price, or before durations were kept.
        if reprice_stored_record(transaction, record)? {
            repriced += 1;
        }
        if fill_record_timing(transaction, record)? {
            timings_filled += 1;
        }
    }
    for (source_path, stamp, cursor) in &scan.cursors {
        save_cursor(transaction, source_path, *stamp, cursor)?;
    }
    for source_path in &scan.stale_cursors {
        delete_cursor(transaction, source_path)?;
    }
    Ok(ScanWrite {
        repriced,
        timings_filled,
    })
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
