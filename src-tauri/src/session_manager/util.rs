use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};
use time::OffsetDateTime;

pub(super) fn dedupe_strings(items: &mut Vec<String>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.clone()));
}

pub(super) fn unique_sibling_path(path: &Path, base_name: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    for index in 0..1000 {
        let file_name = if index == 0 {
            base_name.to_string()
        } else {
            format!("{base_name}-{index:03}")
        };
        let candidate = parent.join(file_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{base_name}-overflow"))
}

pub(super) fn first_non_empty(values: &[String]) -> Option<String> {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub(super) fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.trim().chars();
    let mut result = String::new();
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return result;
        };
        result.push(ch);
    }
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

pub(super) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|err| format!("读取文件失败 {}: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let len = file
            .read(&mut buffer)
            .map_err(|err| format!("读取文件失败 {}: {err}", path.display()))?;
        if len == 0 {
            break;
        }
        hasher.update(&buffer[..len]);
    }
    Ok(hex_bytes(&hasher.finalize()))
}

pub(super) fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_bytes(&hasher.finalize())
}

pub(super) fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

pub(super) fn system_time_to_rfc3339(time: Option<SystemTime>) -> Option<String> {
    time.map(OffsetDateTime::from).and_then(|time| {
        time.format(&time::format_description::well_known::Rfc3339)
            .ok()
    })
}

pub(super) fn timestamp_millis_to_rfc3339(milliseconds: i64) -> Option<String> {
    let nanoseconds = i128::from(milliseconds).checked_mul(1_000_000)?;
    OffsetDateTime::from_unix_timestamp_nanos(nanoseconds)
        .ok()
        .and_then(|time| {
            time.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
}

pub(super) fn timestamp_seconds_to_rfc3339(seconds: i64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|time| {
            time.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
}

pub(super) fn backup_stamp() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}
