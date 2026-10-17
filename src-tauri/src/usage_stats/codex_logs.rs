//! When a response started streaming, from Codex's own log database (`logs_<n>.sqlite` in its
//! home). Codex notes each output item of a response as it arrives (`Output item item_type=…`
//! from `codex_core::stream_events_utils`); the first one after the request was sent is the
//! closest record there is of the first token. Rollouts only hold finished items.
//!
//! Codex keeps the last 1,000 lines per thread, so in a busy thread a response's lines are gone
//! some 140 responses (about 20 minutes) later; the statistics are therefore refreshed every
//! minute in the background (`usage_stats::start_usage_stats_auto_refresher`).

use rusqlite::{params, Connection, OpenFlags};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const FIRST_OUTPUT_SQL: &str = "SELECT ts, ts_nanos FROM logs \
     WHERE thread_id = ?1 AND ts >= ?2 AND ts <= ?3 \
       AND target = 'codex_core::stream_events_utils' \
       AND feedback_log_body LIKE '%Output item item_type=%' \
       AND feedback_log_body NOT LIKE '%handle_output_item_done%' \
     ORDER BY ts, ts_nanos";

pub(super) struct CodexLog {
    connection: Connection,
    pub(super) path: PathBuf,
}

impl CodexLog {
    /// The newest log database in `codex_home`, read only; `Ok(None)` when there is none.
    pub(super) fn open(codex_home: &Path) -> Result<Option<Self>, String> {
        let Some(path) = newest_log_database(codex_home)? else {
            return Ok(None);
        };
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|err| format!("打开 Codex 日志库失败 {}: {err}", path.display()))?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(|err| format!("设置 Codex 日志库等待时间失败 {}: {err}", path.display()))?;
        Ok(Some(Self { connection, path }))
    }

    /// When the first output item of a response arrived (ms since the epoch): the first one Codex
    /// noted for the thread after the request was sent and no later than the response completed.
    pub(super) fn first_output_ms(
        &self,
        thread_id: &str,
        started_ms: i64,
        completed_ms: i64,
    ) -> Result<Option<i64>, String> {
        let mut statement = self
            .connection
            .prepare_cached(FIRST_OUTPUT_SQL)
            .map_err(|err| format!("读取 Codex 日志失败 {}: {err}", self.path.display()))?;
        let mut rows = statement
            .query(params![
                thread_id,
                started_ms.div_euclid(1000),
                completed_ms.div_euclid(1000)
            ])
            .map_err(|err| format!("读取 Codex 日志失败 {}: {err}", self.path.display()))?;
        while let Some(row) = rows
            .next()
            .map_err(|err| format!("读取 Codex 日志失败 {}: {err}", self.path.display()))?
        {
            let seconds: i64 = row
                .get(0)
                .map_err(|err| format!("读取 Codex 日志时间失败: {err}"))?;
            let nanos: i64 = row
                .get(1)
                .map_err(|err| format!("读取 Codex 日志时间失败: {err}"))?;
            let at = seconds * 1000 + nanos.div_euclid(1_000_000);
            if at > started_ms && at <= completed_ms {
                return Ok(Some(at));
            }
        }
        Ok(None)
    }
}

/// `logs_2.sqlite` today; the number goes up when Codex changes the schema.
fn newest_log_database(codex_home: &Path) -> Result<Option<PathBuf>, String> {
    let entries = match fs::read_dir(codex_home) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "读取 Codex 目录失败 {}: {err}",
                codex_home.display()
            ))
        }
    };
    let mut newest: Option<(u32, PathBuf)> = None;
    for entry in entries {
        let entry = entry.map_err(|err| format!("读取 Codex 目录条目失败: {err}"))?;
        let name = entry.file_name();
        let Some(version) = name
            .to_str()
            .and_then(|name| name.strip_prefix("logs_"))
            .and_then(|rest| rest.strip_suffix(".sqlite"))
            .and_then(|version| version.parse::<u32>().ok())
        else {
            continue;
        };
        if newest.as_ref().is_none_or(|(found, _)| version > *found) {
            newest = Some((version, entry.path()));
        }
    }
    Ok(newest.map(|(_, path)| path))
}
