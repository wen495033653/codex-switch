mod parser;
mod scanner;

use serde_json::Value;

pub(crate) use parser::usage_info_fetched_at_seconds;

pub(crate) fn latest_usage_info() -> Result<Option<Value>, String> {
    let sessions_dir = scanner::codex_home_dir().join("sessions");
    if !sessions_dir.exists() {
        return Ok(None);
    }

    let files = scanner::collect_recent_files(&sessions_dir)?;

    let mut latest = None;
    for (_, path) in files {
        match parser::usage_info_from_file(&path) {
            Ok(Some(usage_info)) => {
                latest = parser::newer_usage_info(latest, usage_info);
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!("{err}");
            }
        }
    }
    Ok(latest)
}
