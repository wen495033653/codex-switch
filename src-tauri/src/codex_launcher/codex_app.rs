use super::*;

mod package;
mod process;
mod proxy;

const SHORTCUT_LAUNCH_ARG: &str = "--codex-switch-launch-codex";
const SHORTCUT_PROXY_URL_ARG: &str = "--codex-switch-proxy-url";

pub(crate) fn run_codex_shortcut_from_args<I>(args: I) -> Option<Result<(), String>>
where
    I: IntoIterator<Item = String>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    if !args.iter().any(|arg| arg == SHORTCUT_LAUNCH_ARG) {
        return None;
    }

    Some(run_codex_shortcut(&args))
}

fn run_codex_shortcut(args: &[String]) -> Result<(), String> {
    let proxy_url = shortcut_arg_value(args, SHORTCUT_PROXY_URL_ARG).unwrap_or_default();
    let settings = read_settings_value()?;
    let profile = settings.get("api_mode").unwrap_or(&Value::Null);
    let openai_base_url = string_field(profile, "base_url");

    if proxy_url.trim().is_empty() {
        proxy::launch_codex_plain_impl()?;
        return Ok(());
    }

    let normalized_proxy_url = normalize_proxy_url(&proxy_url)?;
    proxy::launch_codex_with_proxy_impl(&normalized_proxy_url, &openai_base_url)?;
    Ok(())
}

fn shortcut_arg_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|items| items.first().is_some_and(|item| item == name))
        .and_then(|items| items.get(1))
        .cloned()
}

#[tauri::command]
pub(crate) fn check_codex_proxy(proxy_url: String) -> Result<Value, String> {
    proxy::check_codex_proxy_impl(proxy_url)
}

#[tauri::command]
pub(crate) fn launch_codex_with_proxy(options: Option<Value>) -> Result<Value, String> {
    let opts = options.unwrap_or_else(|| json!({}));
    let settings = read_settings_value()?;
    let profile = settings.get("api_mode").unwrap_or(&Value::Null);
    let requested_proxy_url = {
        let snake_case = string_field(&opts, "proxy_url");
        if snake_case.is_empty() {
            string_field(&opts, "proxyUrl")
        } else {
            snake_case
        }
    };
    let saved_proxy_url = string_field(&settings, "codex_proxy_url");
    let proxy_url = if !requested_proxy_url.is_empty() {
        normalize_proxy_url(&requested_proxy_url)?
    } else if !saved_proxy_url.is_empty() {
        normalize_proxy_url(&saved_proxy_url)?
    } else {
        return Err("代理地址不能为空".to_string());
    };
    let openai_base_url = string_field(profile, "base_url");
    let result = proxy::launch_codex_with_proxy_impl(&proxy_url, &openai_base_url)?;
    let proxy_connected = bool_field(&result, "proxy_connected");
    Ok(json!({
        "ok": true,
        "message": if proxy_connected {
            "已启动 Codex，并为本次启动注入代理"
        } else {
            "Codex 已启动，但未观察到代理连接"
        },
        "result": result
    }))
}

#[tauri::command]
pub(crate) fn create_codex_proxy_desktop_shortcut(options: Option<Value>) -> Result<Value, String> {
    let opts = options.unwrap_or_else(|| json!({}));
    let settings = read_settings_value()?;
    let requested_proxy_url = if opts.get("proxy_url").is_some() {
        Some(string_field(&opts, "proxy_url"))
    } else if opts.get("proxyUrl").is_some() {
        Some(string_field(&opts, "proxyUrl"))
    } else {
        None
    };
    let saved_proxy_url = string_field(&settings, "codex_proxy_url");
    let proxy_url = match requested_proxy_url {
        Some(value) if !value.is_empty() => normalize_proxy_url(&value)?,
        Some(_) => String::new(),
        None if !saved_proxy_url.is_empty() => normalize_proxy_url(&saved_proxy_url)?,
        None => String::new(),
    };
    let result = proxy::create_codex_proxy_desktop_shortcut_impl(&proxy_url)?;
    let proxy_enabled = bool_field(&result, "proxy_enabled");
    Ok(json!({
        "ok": true,
        "message": if proxy_enabled {
            "已创建 Codex 代理启动图标"
        } else {
            "已创建 Codex 启动图标"
        },
        "result": result
    }))
}
