use crate::{
    json_util::bool_field,
    paths::{codex_dir, config_path},
    session_sync_diagnostics::log_session_sync_event,
    settings::{read_settings_value, update_settings_value},
    time_util::now_string,
};
use serde_json::{json, Value};
use std::{
    cmp::Ordering,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    thread,
    time::Duration as StdDuration,
};

const SETTING_KEY: &str = "codex_computer_use_repair_guard_enabled";
const OPENAI_BUNDLED_MARKETPLACE: &str = "openai-bundled";
const COMPUTER_USE_PLUGIN: &str = "computer-use";
const MIN_PLUGIN_FILE_COUNT: usize = 20;
const GUARD_INTERVAL_MS: u64 = 30_000;

#[derive(Clone, Debug)]
struct PluginCandidate {
    path: PathBuf,
    origin: String,
    version: String,
    valid: bool,
    file_count: usize,
    missing: Vec<String>,
}

#[derive(Clone, Debug)]
struct MarketplaceLocation {
    path: PathBuf,
    source: String,
    configured_source: Option<String>,
}

#[derive(Clone, Debug)]
struct RepairState {
    codex_home: PathBuf,
    marketplace: MarketplaceLocation,
    active: PluginCandidate,
    source: Option<PluginCandidate>,
    candidates: Vec<PluginCandidate>,
    guard_enabled: bool,
}

static GUARD_STARTED: OnceLock<()> = OnceLock::new();

#[tauri::command]
pub(crate) fn get_computer_use_repair_status() -> Result<Value, String> {
    Ok(repair_state_value(&scan_repair_state()?))
}

#[tauri::command]
pub(crate) fn repair_computer_use_plugin() -> Result<Value, String> {
    repair_computer_use_plugin_with_trigger("manual")
}

#[tauri::command]
pub(crate) fn set_computer_use_repair_guard_enabled(enabled: bool) -> Result<Value, String> {
    let settings = update_settings_value(&json!({ SETTING_KEY: enabled }))?;
    if enabled {
        start_computer_use_repair_guard();
    }
    Ok(json!({
        "settings": settings,
        "status": repair_state_value(&scan_repair_state()?),
        "message": if enabled { "Computer Use 自动守护修复已启用" } else { "Computer Use 自动守护修复已关闭" }
    }))
}

pub(crate) fn start_computer_use_repair_guard() {
    GUARD_STARTED.get_or_init(|| {
        thread::spawn(run_computer_use_repair_guard);
    });
}

fn run_computer_use_repair_guard() {
    loop {
        if guard_enabled_from_settings() {
            match repair_computer_use_if_needed("guard") {
                Ok(Some(value)) => {
                    log_session_sync_event("computer_use_repair_guard_repaired", value);
                }
                Ok(None) => {}
                Err(err) => {
                    eprintln!("Computer Use 自动守护修复失败: {err}");
                    log_session_sync_event(
                        "computer_use_repair_guard_error",
                        json!({ "error": err }),
                    );
                }
            }
        }
        thread::sleep(StdDuration::from_millis(GUARD_INTERVAL_MS));
    }
}

fn repair_computer_use_if_needed(trigger: &str) -> Result<Option<Value>, String> {
    let state = scan_repair_state()?;
    if state.active.valid || state.source.is_none() {
        return Ok(None);
    }
    repair_computer_use_plugin_from_state(state, trigger).map(Some)
}

fn repair_computer_use_plugin_with_trigger(trigger: &str) -> Result<Value, String> {
    let state = scan_repair_state()?;
    if state.active.valid {
        return Ok(json!({
            "ok": true,
            "action": "noop",
            "message": "Computer Use 当前已可用，无需修复",
            "status": repair_state_value(&state)
        }));
    }
    repair_computer_use_plugin_from_state(state, trigger)
}

fn repair_computer_use_plugin_from_state(
    state: RepairState,
    trigger: &str,
) -> Result<Value, String> {
    let source = state
        .source
        .clone()
        .ok_or_else(|| "没有找到可用于修复的 Computer Use 本机来源".to_string())?;
    if !source.valid {
        return Err("Computer Use 修复来源不完整".to_string());
    }

    let active_path = state.active.path.clone();
    let backup_path = if active_path.exists() {
        let target = unique_backup_path(
            &state.codex_home,
            &format!("codex-switch-computer-use-repair-{}", safe_timestamp()),
        )
        .join(COMPUTER_USE_PLUGIN);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "创建 Computer Use 修复备份目录失败 {}: {err}",
                    parent.display()
                )
            })?;
        }
        fs::rename(&active_path, &target).map_err(|err| {
            format!(
                "备份现有 Computer Use 插件失败 {} -> {}: {err}",
                active_path.display(),
                target.display()
            )
        })?;
        Some(target)
    } else {
        None
    };

    if let Some(parent) = active_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "创建 Computer Use active 插件目录失败 {}: {err}",
                parent.display()
            )
        })?;
    }
    let files_copied = copy_dir_recursive(&source.path, &active_path)?;
    let cache_copy = ensure_cache_copy_for_source(&state.codex_home, &source)?;
    let next_state = scan_repair_state()?;
    let result = json!({
        "ok": next_state.active.valid,
        "action": "repaired",
        "trigger": trigger,
        "message": if next_state.active.valid { "Computer Use 已从本机缓存修复" } else { "Computer Use 已复制，但检测仍未通过" },
        "source": source.to_value(),
        "filesCopied": files_copied,
        "backupPath": backup_path.as_ref().map(|path| display_path(path)),
        "cacheCopy": cache_copy,
        "status": repair_state_value(&next_state)
    });
    log_session_sync_event("computer_use_repair_finished", result.clone());
    Ok(result)
}

fn scan_repair_state() -> Result<RepairState, String> {
    let codex_home = codex_dir()?;
    let marketplace = active_marketplace_location(&codex_home)?;
    let active_path = marketplace.path.join("plugins").join(COMPUTER_USE_PLUGIN);
    let active = plugin_candidate(active_path, "active", "");
    let mut candidates = repair_source_candidates(&codex_home)?;
    candidates.sort_by(compare_candidates);
    let source = candidates.iter().find(|candidate| candidate.valid).cloned();
    Ok(RepairState {
        codex_home,
        marketplace,
        active,
        source,
        candidates,
        guard_enabled: guard_enabled_from_settings(),
    })
}

fn active_marketplace_location(codex_home: &Path) -> Result<MarketplaceLocation, String> {
    if let Some(configured_source) = read_openai_bundled_source()? {
        let path = normalize_configured_path(&configured_source);
        return Ok(MarketplaceLocation {
            path,
            source: "config".to_string(),
            configured_source: Some(configured_source),
        });
    }

    Ok(MarketplaceLocation {
        path: codex_home
            .join(".tmp")
            .join("bundled-marketplaces")
            .join(OPENAI_BUNDLED_MARKETPLACE),
        source: "default".to_string(),
        configured_source: None,
    })
}

fn read_openai_bundled_source() -> Result<Option<String>, String> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("读取 config.toml 失败 {}: {err}", path.display()))?;
    let mut in_openai_bundled = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_openai_bundled = trimmed == "[marketplaces.openai-bundled]";
            continue;
        }
        if !in_openai_bundled || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim() != "source" {
            continue;
        }
        let source = unquote_toml_string(value.trim());
        if !source.trim().is_empty() {
            return Ok(Some(source));
        }
    }
    Ok(None)
}

fn unquote_toml_string(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        return trimmed[1..trimmed.len() - 1].to_string();
    }
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        return trimmed[1..trimmed.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
    }
    trimmed.to_string()
}

fn normalize_configured_path(value: &str) -> PathBuf {
    let trimmed = value.trim();
    let without_file_scheme = trimmed.strip_prefix("file://").unwrap_or(trimmed);
    let without_verbatim = strip_windows_verbatim_drive_prefix(without_file_scheme);
    PathBuf::from(without_verbatim)
}

fn strip_windows_verbatim_drive_prefix(value: &str) -> &str {
    let Some(rest) = value.strip_prefix("\\\\?\\") else {
        return value;
    };
    let bytes = rest.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        rest
    } else {
        value
    }
}

fn repair_source_candidates(codex_home: &Path) -> Result<Vec<PluginCandidate>, String> {
    let mut candidates = Vec::new();
    candidates.extend(cache_source_candidates(codex_home)?);
    candidates.extend(staging_source_candidates(codex_home)?);
    Ok(candidates)
}

fn cache_source_candidates(codex_home: &Path) -> Result<Vec<PluginCandidate>, String> {
    let root = codex_home
        .join("plugins")
        .join("cache")
        .join(OPENAI_BUNDLED_MARKETPLACE)
        .join(COMPUTER_USE_PLUGIN);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&root)
        .map_err(|err| format!("读取 Computer Use cache 失败 {}: {err}", root.display()))?
    {
        let entry = entry.map_err(|err| format!("读取 Computer Use cache 条目失败: {err}"))?;
        let path = entry.path();
        if path.is_dir() {
            let version_hint = entry.file_name().to_string_lossy().to_string();
            candidates.push(plugin_candidate(path, "cache", &version_hint));
        }
    }
    Ok(candidates)
}

fn staging_source_candidates(codex_home: &Path) -> Result<Vec<PluginCandidate>, String> {
    let root = codex_home.join(".tmp").join("bundled-marketplaces");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&root)
        .map_err(|err| format!("读取 bundled marketplaces 失败 {}: {err}", root.display()))?
    {
        let entry = entry.map_err(|err| format!("读取 bundled marketplace 条目失败: {err}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("openai-bundled.staging-") {
            continue;
        }
        let path = entry.path().join("plugins").join(COMPUTER_USE_PLUGIN);
        candidates.push(plugin_candidate(path, "staging", ""));
    }
    Ok(candidates)
}

fn plugin_candidate(path: PathBuf, origin: &str, version_hint: &str) -> PluginCandidate {
    let manifest = path.join(".codex-plugin").join("plugin.json");
    let script = path.join("scripts").join("computer-use-client.mjs");
    let file_count = count_files(&path).unwrap_or(0);
    let version = manifest_version(&manifest)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| version_hint.to_string());
    let mut missing = Vec::new();
    if !path.is_dir() {
        missing.push("plugin_dir".to_string());
    }
    if !manifest.is_file() {
        missing.push("manifest".to_string());
    }
    if !script.is_file() {
        missing.push("client_script".to_string());
    }
    if file_count < MIN_PLUGIN_FILE_COUNT {
        missing.push("file_count".to_string());
    }
    PluginCandidate {
        path,
        origin: origin.to_string(),
        version,
        valid: missing.is_empty(),
        file_count,
        missing,
    }
}

fn manifest_version(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let parsed = serde_json::from_str::<Value>(&raw).ok()?;
    parsed
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn count_files(path: &Path) -> Result<usize, String> {
    if !path.exists() {
        return Ok(0);
    }
    if path.is_file() {
        return Ok(1);
    }
    let mut count = 0usize;
    for entry in
        fs::read_dir(path).map_err(|err| format!("读取目录失败 {}: {err}", path.display()))?
    {
        let entry = entry.map_err(|err| format!("读取目录条目失败: {err}"))?;
        let file_type = entry
            .file_type()
            .map_err(|err| format!("读取文件类型失败 {}: {err}", entry.path().display()))?;
        if file_type.is_dir() {
            count += count_files(&entry.path())?;
        } else if file_type.is_file() {
            count += 1;
        }
    }
    Ok(count)
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<usize, String> {
    if !source.is_dir() {
        return Err(format!("复制来源不是目录: {}", source.display()));
    }
    fs::create_dir_all(target)
        .map_err(|err| format!("创建目标目录失败 {}: {err}", target.display()))?;
    let mut copied = 0usize;
    for entry in fs::read_dir(source)
        .map_err(|err| format!("读取复制来源失败 {}: {err}", source.display()))?
    {
        let entry = entry.map_err(|err| format!("读取复制来源条目失败: {err}"))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|err| format!("读取文件类型失败 {}: {err}", source_path.display()))?;
        if file_type.is_dir() {
            copied += copy_dir_recursive(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path).map_err(|err| {
                format!(
                    "复制文件失败 {} -> {}: {err}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
            copied += 1;
        }
    }
    Ok(copied)
}

fn ensure_cache_copy_for_source(
    codex_home: &Path,
    source: &PluginCandidate,
) -> Result<Value, String> {
    if source.origin == "cache" || source.version.trim().is_empty() {
        return Ok(Value::Null);
    }
    let target = codex_home
        .join("plugins")
        .join("cache")
        .join(OPENAI_BUNDLED_MARKETPLACE)
        .join(COMPUTER_USE_PLUGIN)
        .join(&source.version);
    let target_candidate = plugin_candidate(target.clone(), "cache", &source.version);
    if target_candidate.valid {
        return Ok(json!({
            "path": display_path(&target),
            "action": "exists"
        }));
    }
    if target.exists() {
        let backup = unique_backup_path(
            codex_home,
            &format!(
                "codex-switch-computer-use-cache-repair-{}",
                safe_timestamp()
            ),
        )
        .join(&source.version);
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "创建 Computer Use cache 备份目录失败 {}: {err}",
                    parent.display()
                )
            })?;
        }
        fs::rename(&target, &backup).map_err(|err| {
            format!(
                "备份现有 Computer Use cache 失败 {} -> {}: {err}",
                target.display(),
                backup.display()
            )
        })?;
    }
    let copied = copy_dir_recursive(&source.path, &target)?;
    Ok(json!({
        "path": display_path(&target),
        "action": "copied",
        "filesCopied": copied
    }))
}

fn unique_backup_path(codex_home: &Path, name: &str) -> PathBuf {
    let root = codex_home.join("backups");
    let mut candidate = root.join(name);
    let mut index = 2usize;
    while candidate.exists() {
        candidate = root.join(format!("{name}-{index}"));
        index += 1;
    }
    candidate
}

fn safe_timestamp() -> String {
    now_string()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

fn guard_enabled_from_settings() -> bool {
    read_settings_value()
        .map(|settings| bool_field(&settings, SETTING_KEY))
        .unwrap_or(false)
}

fn compare_candidates(left: &PluginCandidate, right: &PluginCandidate) -> Ordering {
    origin_priority(&left.origin)
        .cmp(&origin_priority(&right.origin))
        .then_with(|| compare_versions(&right.version, &left.version))
        .then_with(|| right.file_count.cmp(&left.file_count))
        .then_with(|| display_path(&left.path).cmp(&display_path(&right.path)))
}

fn origin_priority(origin: &str) -> u8 {
    match origin {
        "cache" => 0,
        "staging" => 1,
        _ => 2,
    }
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let left_parts = numeric_version_parts(left);
    let right_parts = numeric_version_parts(right);
    left_parts.cmp(&right_parts).then_with(|| left.cmp(right))
}

fn numeric_version_parts(value: &str) -> Vec<u64> {
    value
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

fn repair_state_value(state: &RepairState) -> Value {
    let repair_available = state.source.as_ref().is_some_and(|source| source.valid);
    let needs_repair = !state.active.valid;
    let status = if state.active.valid {
        "ok"
    } else if repair_available {
        "repairable"
    } else {
        "missing_source"
    };
    let message = match status {
        "ok" => "Computer Use 当前可用",
        "repairable" => "Computer Use 缺失或不完整，可从本机缓存修复",
        _ => "Computer Use 缺失或不完整，且未找到完整本机来源",
    };

    json!({
        "status": status,
        "message": message,
        "needsRepair": needs_repair,
        "repairAvailable": repair_available,
        "guardEnabled": state.guard_enabled,
        "codexHome": display_path(&state.codex_home),
        "marketplace": {
            "path": display_path(&state.marketplace.path),
            "source": state.marketplace.source,
            "configuredSource": state.marketplace.configured_source
        },
        "active": state.active.to_value(),
        "source": state.source.as_ref().map(PluginCandidate::to_value),
        "candidates": state
            .candidates
            .iter()
            .map(PluginCandidate::to_value)
            .collect::<Vec<_>>()
    })
}

fn display_path(path: &Path) -> String {
    let value = path.display().to_string();
    strip_windows_verbatim_drive_prefix(&value).to_string()
}

impl PluginCandidate {
    fn to_value(&self) -> Value {
        json!({
            "path": display_path(&self.path),
            "origin": self.origin,
            "version": self.version,
            "valid": self.valid,
            "fileCount": self.file_count,
            "missing": self.missing
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("codex-switch-computer-use-{name}-{stamp}"))
    }

    #[test]
    fn unquote_toml_string_handles_single_quoted_windows_path() {
        assert_eq!(
            unquote_toml_string(
                r"'\\?\D:\CodexHome\.codex\.tmp\bundled-marketplaces\openai-bundled'"
            ),
            r"\\?\D:\CodexHome\.codex\.tmp\bundled-marketplaces\openai-bundled"
        );
    }

    #[test]
    fn normalize_configured_path_strips_windows_verbatim_drive_prefix() {
        assert_eq!(
            normalize_configured_path(r"\\?\D:\CodexHome\.codex")
                .display()
                .to_string(),
            r"D:\CodexHome\.codex"
        );
    }

    #[test]
    fn plugin_candidate_requires_manifest_script_and_enough_files() {
        let root = temp_path("candidate");
        fs::create_dir_all(root.join(".codex-plugin")).unwrap();
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::write(
            root.join(".codex-plugin").join("plugin.json"),
            r#"{"version":"1.2.3"}"#,
        )
        .unwrap();
        fs::write(root.join("scripts").join("computer-use-client.mjs"), "").unwrap();
        for index in 0..MIN_PLUGIN_FILE_COUNT {
            fs::write(root.join(format!("file-{index}.txt")), "").unwrap();
        }

        let candidate = plugin_candidate(root.clone(), "cache", "");

        assert!(candidate.valid);
        assert_eq!(candidate.version, "1.2.3");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn compare_candidates_prefers_cache_before_staging() {
        let cache = PluginCandidate {
            path: PathBuf::from("cache"),
            origin: "cache".to_string(),
            version: "1.0.0".to_string(),
            valid: true,
            file_count: 20,
            missing: Vec::new(),
        };
        let staging = PluginCandidate {
            path: PathBuf::from("staging"),
            origin: "staging".to_string(),
            version: "9.0.0".to_string(),
            valid: true,
            file_count: 20,
            missing: Vec::new(),
        };

        assert_eq!(compare_candidates(&cache, &staging), Ordering::Less);
    }
}
