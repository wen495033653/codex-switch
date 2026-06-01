use super::*;
use crate::session_sync_diagnostics::log_session_sync_event;

const CODEX_PROXY_ENV_NAMES: [&str; 4] = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"];
const CODEX_NO_PROXY_VALUE: &str = "localhost,127.0.0.1,::1";

struct CodexProxyEnvState {
    enabled: bool,
    proxy_url: String,
}

fn codex_env_path() -> Result<PathBuf, String> {
    Ok(codex_dir()?.join(".env"))
}

fn managed_proxy_env_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let assignment = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let (name, _) = assignment.split_once('=')?;
    let name = name.trim();
    CODEX_PROXY_ENV_NAMES
        .iter()
        .any(|expected| name.eq_ignore_ascii_case(expected))
        .then_some(name)
}

fn remove_managed_proxy_lines(content: &str) -> Vec<&str> {
    content
        .lines()
        .filter(|line| managed_proxy_env_name(line).is_none())
        .collect()
}

fn build_codex_env_content(existing: &str, proxy_url: &str) -> String {
    let mut output = remove_managed_proxy_lines(existing).join("\n");
    if !output.trim().is_empty() {
        output.push('\n');
    }
    output.push_str(&format!(
        "HTTP_PROXY={proxy_url}\nHTTPS_PROXY={proxy_url}\nALL_PROXY={proxy_url}\nNO_PROXY={CODEX_NO_PROXY_VALUE}\n"
    ));
    output
}

fn read_codex_env_content(path: &Path) -> Result<String, String> {
    if !path.exists() {
        return Ok(String::new());
    }
    fs::read_to_string(path)
        .map_err(|err| format!("读取 Codex app 代理配置失败 {}: {err}", path.display()))
}

fn write_codex_env_content(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建 Codex 目录失败: {err}"))?;
    }
    fs::write(path, content)
        .map_err(|err| format!("保存 Codex app 代理配置失败 {}: {err}", path.display()))
}

fn set_codex_proxy_env_file_enabled(enabled: bool, proxy_url: &str) -> Result<String, String> {
    let path = codex_env_path()?;
    let existing = read_codex_env_content(&path)?;
    if enabled {
        let normalized_proxy_url = normalize_proxy_url(proxy_url)?;
        if normalized_proxy_url.is_empty() {
            return Err("代理地址不能为空".to_string());
        }
        let content = build_codex_env_content(&existing, &normalized_proxy_url);
        write_codex_env_content(&path, &content)?;
        return Ok(normalize_proxy_display_url(&normalized_proxy_url));
    }

    let kept_lines = remove_managed_proxy_lines(&existing);
    if kept_lines.iter().all(|line| line.trim().is_empty()) {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|err| format!("关闭 Codex app 代理失败 {}: {err}", path.display()))?;
        }
        return Ok(String::new());
    }

    let mut content = kept_lines.join("\n");
    content.push('\n');
    write_codex_env_content(&path, &content)?;
    Ok(String::new())
}

fn read_codex_proxy_env_state() -> Result<CodexProxyEnvState, String> {
    let path = codex_env_path()?;
    let content = read_codex_env_content(&path)?;
    let mut proxy_url = String::new();
    let mut has_http_proxy = false;
    let mut has_https_proxy = false;
    let mut has_all_proxy = false;

    for line in content.lines() {
        let Some(name) = managed_proxy_env_name(line) else {
            continue;
        };
        let value = line
            .split_once('=')
            .map(|(_, value)| value.trim().trim_matches('"').trim_matches('\''))
            .unwrap_or("");
        if name.eq_ignore_ascii_case("HTTP_PROXY") {
            has_http_proxy = !value.is_empty();
            proxy_url = value.to_string();
        } else if name.eq_ignore_ascii_case("HTTPS_PROXY") {
            has_https_proxy = !value.is_empty();
            if proxy_url.is_empty() {
                proxy_url = value.to_string();
            }
        } else if name.eq_ignore_ascii_case("ALL_PROXY") {
            has_all_proxy = !value.is_empty();
            if proxy_url.is_empty() {
                proxy_url = value.to_string();
            }
        }
    }

    Ok(CodexProxyEnvState {
        enabled: has_http_proxy && has_https_proxy && has_all_proxy,
        proxy_url: normalize_proxy_display_url(&proxy_url),
    })
}

pub(crate) fn apply_codex_proxy_env_state_to_settings(
    mut settings: Value,
) -> Result<Value, String> {
    let state = read_codex_proxy_env_state()?;
    let Some(settings) = settings.as_object_mut() else {
        return Ok(settings);
    };

    settings.insert(
        "codex_proxy_env_enabled".to_string(),
        Value::Bool(state.enabled),
    );
    if state.enabled && !state.proxy_url.is_empty() {
        settings.insert(
            "codex_proxy_url".to_string(),
            Value::String(state.proxy_url),
        );
    }
    Ok(Value::Object(settings.clone()))
}

#[tauri::command]
pub(crate) fn set_codex_proxy_env_enabled(
    enabled: bool,
    proxy_url: String,
) -> Result<Value, String> {
    let proxy_url = set_codex_proxy_env_file_enabled(enabled, &proxy_url)?;
    let mut patch = json!({
        "codex_proxy_env_enabled": enabled
    });
    if enabled {
        patch["codex_proxy_url"] = Value::String(proxy_url.clone());
    }
    let settings = apply_codex_proxy_env_state_to_settings(update_settings_value(&patch)?)?;
    let remote_control_runtime = sync_remote_control_runtime_after_proxy_change(&settings);

    Ok(json!({
        "ok": true,
        "message": if enabled {
            "Codex app 代理已启用"
        } else {
            "Codex app 代理已关闭"
        },
        "settings": settings,
        "env_path": codex_env_path()?.to_string_lossy().to_string(),
        "proxy_url": proxy_url,
        "remoteControl": remote_control_runtime
    }))
}

fn sync_remote_control_runtime_after_proxy_change(settings: &Value) -> Value {
    if !remote_control_enabled_from_settings(settings) {
        return json!({ "changed": false });
    }

    match restart_remote_control_runtime_for_current_settings("set_codex_proxy_env_enabled") {
        Ok(changed) => json!({ "changed": changed }),
        Err(err) => {
            let error = err.clone();
            log_session_sync_event(
                "codex_remote_control_helper_error",
                json!({
                    "context": "set_codex_proxy_env_enabled",
                    "error": error
                }),
            );
            json!({ "changed": false, "error": err })
        }
    }
}
