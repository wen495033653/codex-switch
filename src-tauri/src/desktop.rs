use crate::{
    json_util::{bool_field, string_field},
    settings::{read_settings_value, update_settings_value},
};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::Duration as StdDuration,
};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, CloseRequestApi, Manager, PhysicalSize, Size, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as AutoStartManagerExt;

const MAIN_WINDOW_LABEL: &str = "main";

#[tauri::command]
pub(crate) async fn open_dev_log_window(
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    open_dev_log_window_impl(app)
}

#[tauri::command]
pub(crate) async fn hide_dev_log_window(
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    hide_dev_log_window_impl(app)
}

pub(crate) const AUTO_START_LAUNCH_ARG: &str = "--codex-switch-autostart";

const DEV_AUTO_START_UNSUPPORTED_MESSAGE: &str = "开发模式不支持开机自启，请使用安装后的正式版本。";

fn system_auto_start_supported() -> bool {
    !cfg!(debug_assertions)
}

fn validate_system_auto_start_for_mode(enabled: bool, supported: bool) -> Result<(), String> {
    if enabled && !supported {
        return Err(DEV_AUTO_START_UNSUPPORTED_MESSAGE.to_string());
    }
    Ok(())
}

pub(crate) fn validate_system_auto_start(enabled: bool) -> Result<(), String> {
    validate_system_auto_start_for_mode(enabled, system_auto_start_supported())
}

pub(crate) fn sync_system_auto_start(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    if enabled {
        if let Err(err) = validate_system_auto_start(enabled) {
            if autolaunch
                .is_enabled()
                .map_err(|err| format!("检查开机自启状态失败: {err}"))?
            {
                autolaunch
                    .disable()
                    .map_err(|err| format!("关闭开机自启失败: {err}"))?;
            }
            return Err(err);
        }
        autolaunch
            .enable()
            .map_err(|err| format!("启用开机自启失败: {err}"))
    } else if autolaunch
        .is_enabled()
        .map_err(|err| format!("检查开机自启状态失败: {err}"))?
    {
        autolaunch
            .disable()
            .map_err(|err| format!("关闭开机自启失败: {err}"))
    } else {
        Ok(())
    }
}

pub(crate) fn sync_system_auto_start_from_settings(app: &AppHandle) -> Result<(), String> {
    let settings = read_settings_value()?;
    sync_system_auto_start(app, bool_field(&settings, "auto_start"))
}

#[derive(Default)]
pub(crate) struct AppRuntime {
    pub(super) is_quitting: AtomicBool,
    pub(crate) window_state_save_generation: AtomicU64,
    pub(crate) window_state_save_worker_running: AtomicBool,
}

const TRAY_SHOW_MAIN_WINDOW_ID: &str = "tray-show-main-window";

const TRAY_QUIT_ID: &str = "tray-quit";

fn request_app_quit(app: &AppHandle) {
    app.state::<AppRuntime>()
        .is_quitting
        .store(true, Ordering::SeqCst);
    app.exit(0);
}

pub(crate) fn setup_tray(app: &AppHandle) -> Result<(), String> {
    let show_item = MenuItem::with_id(
        app,
        TRAY_SHOW_MAIN_WINDOW_ID,
        "显示主窗口",
        true,
        None::<&str>,
    )
    .map_err(|err| format!("创建托盘菜单失败: {err}"))?;
    let quit_item = MenuItem::with_id(app, TRAY_QUIT_ID, "退出", true, None::<&str>)
        .map_err(|err| format!("创建托盘菜单失败: {err}"))?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])
        .map_err(|err| format!("创建托盘菜单失败: {err}"))?;

    app.on_menu_event(|app, event| match event.id().as_ref() {
        TRAY_SHOW_MAIN_WINDOW_ID => focus_main_window(app),
        TRAY_QUIT_ID => request_app_quit(app),
        _ => {}
    });

    let icon = Image::from_bytes(include_bytes!("../../build/icon.png"))
        .map_err(|err| format!("加载托盘图标失败: {err}"))?;
    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Codex Switch")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => focus_main_window(tray.app_handle()),
            _ => {}
        })
        .build(app)
        .map_err(|err| format!("创建托盘图标失败: {err}"))?;
    Ok(())
}

const WINDOW_STATE_PERSIST_DEBOUNCE_MS: u64 = 800;

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(MAIN_WINDOW_LABEL)
}

fn dev_log_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("dev-log")
}

fn dev_log_webview_url(app: &AppHandle) -> WebviewUrl {
    if cfg!(debug_assertions) {
        if let Some(dev_url) = app.config().build.dev_url.as_ref() {
            let mut url = dev_url.clone();
            url.set_query(Some("window=dev-log"));
            return WebviewUrl::External(url);
        }
    }

    WebviewUrl::App("index.html".into())
}

pub(crate) fn focus_main_window(app: &AppHandle) {
    let Some(window) = main_window(app) else {
        return;
    };
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    let _ = window.show();
    let _ = window.set_focus();
}

fn open_dev_log_window_impl(app: AppHandle) -> Result<Value, String> {
    if !cfg!(debug_assertions) {
        return Ok(json!({
            "ok": false,
            "message": "开发日志仅在开发版本可用"
        }));
    }

    if let Some(window) = dev_log_window(&app) {
        if window.is_minimized().unwrap_or(false) {
            let _ = window.unminimize();
        }
        window
            .show()
            .map_err(|err| format!("显示开发日志窗口失败: {err}"))?;
        window
            .set_focus()
            .map_err(|err| format!("聚焦开发日志窗口失败: {err}"))?;
        return Ok(json!({ "ok": true, "reused": true }));
    }

    WebviewWindowBuilder::new(&app, "dev-log", dev_log_webview_url(&app))
        .title("开发日志")
        .inner_size(920.0, 560.0)
        .min_inner_size(520.0, 320.0)
        .resizable(true)
        .initialization_script("window.__CODEX_SWITCH_WINDOW_LABEL = 'dev-log';")
        .build()
        .map_err(|err| format!("打开开发日志窗口失败: {err}"))?;

    Ok(json!({ "ok": true, "reused": false }))
}

fn hide_dev_log_window_impl(app: AppHandle) -> Result<Value, String> {
    if let Some(window) = dev_log_window(&app) {
        window
            .hide()
            .map_err(|err| format!("隐藏开发日志窗口失败: {err}"))?;
    }
    Ok(json!({ "ok": true }))
}

pub(crate) fn restore_main_window_state(app: &AppHandle) -> Result<(), String> {
    let Some(window) = main_window(app) else {
        return Ok(());
    };
    let settings = read_settings_value()?;
    let bounds = settings.get("window_bounds").unwrap_or(&Value::Null);
    let width = bounds.get("width").and_then(Value::as_u64).unwrap_or(0);
    let height = bounds.get("height").and_then(Value::as_u64).unwrap_or(0);
    if width > 0 && height > 0 {
        let width = width.min(u32::MAX as u64) as u32;
        let height = height.min(u32::MAX as u64) as u32;
        window
            .set_size(Size::Physical(PhysicalSize::new(width, height)))
            .map_err(|err| format!("恢复窗口尺寸失败: {err}"))?;
    }
    if bool_field(&settings, "window_is_maximized") {
        window
            .maximize()
            .map_err(|err| format!("恢复窗口最大化状态失败: {err}"))?;
    }
    Ok(())
}

fn should_start_hidden(args: &[String]) -> Result<bool, String> {
    if !args.iter().any(|arg| arg == AUTO_START_LAUNCH_ARG) {
        return Ok(false);
    }
    let settings = read_settings_value()?;
    Ok(string_field(&settings, "auto_start_launch_mode") == "tray")
}

pub(crate) fn apply_main_window_startup_behavior(
    app: &AppHandle,
    args: &[String],
) -> Result<(), String> {
    if !should_start_hidden(args)? {
        return Ok(());
    }
    let Some(window) = main_window(app) else {
        return Ok(());
    };
    window
        .hide()
        .map_err(|err| format!("收起启动窗口失败: {err}"))?;
    Ok(())
}

fn persist_main_window_state(window: &tauri::Window) -> Result<(), String> {
    if window.is_minimized().unwrap_or(false) {
        return Ok(());
    }
    let maximized = window.is_maximized().unwrap_or(false);
    if maximized {
        update_settings_value(&json!({ "window_is_maximized": true }))?;
        return Ok(());
    }

    let size = window
        .inner_size()
        .map_err(|err| format!("读取窗口尺寸失败: {err}"))?;
    update_settings_value(&json!({
        "window_bounds": {
            "width": size.width,
            "height": size.height
        },
        "window_is_maximized": false
    }))?;
    Ok(())
}

fn schedule_main_window_state_persist(window: &tauri::Window) {
    let app = window.app_handle().clone();
    let window = window.clone();
    let runtime = app.state::<AppRuntime>();
    runtime
        .window_state_save_generation
        .fetch_add(1, Ordering::SeqCst);
    if runtime
        .window_state_save_worker_running
        .swap(true, Ordering::SeqCst)
    {
        return;
    }

    thread::spawn(move || {
        let runtime = app.state::<AppRuntime>();
        loop {
            let observed = runtime.window_state_save_generation.load(Ordering::SeqCst);
            thread::sleep(StdDuration::from_millis(WINDOW_STATE_PERSIST_DEBOUNCE_MS));
            if runtime.window_state_save_generation.load(Ordering::SeqCst) != observed {
                continue;
            }
            if let Err(err) = persist_main_window_state(&window) {
                eprintln!("保存窗口状态失败: {err}");
            }
            runtime
                .window_state_save_worker_running
                .store(false, Ordering::SeqCst);
            if runtime.window_state_save_generation.load(Ordering::SeqCst) == observed {
                break;
            }
            if runtime
                .window_state_save_worker_running
                .swap(true, Ordering::SeqCst)
            {
                break;
            }
        }
    });
}

pub(crate) fn handle_main_window_event(window: &tauri::Window, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW_LABEL {
        return;
    }
    match event {
        WindowEvent::Resized(_) => schedule_main_window_state_persist(window),
        WindowEvent::CloseRequested { api, .. } => handle_main_window_close(window, api),
        _ => {}
    }
}

fn handle_main_window_close(window: &tauri::Window, api: &CloseRequestApi) {
    if let Err(err) = persist_main_window_state(window) {
        eprintln!("保存窗口状态失败: {err}");
    }

    let app = window.app_handle().clone();
    if app.state::<AppRuntime>().is_quitting.load(Ordering::SeqCst) {
        return;
    }
    api.prevent_close();
    let _ = window.hide();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_system_auto_start_rejects_enabled_in_dev_mode() {
        assert_eq!(
            validate_system_auto_start_for_mode(true, false),
            Err(DEV_AUTO_START_UNSUPPORTED_MESSAGE.to_string())
        );
    }

    #[test]
    fn validate_system_auto_start_allows_disabled_in_dev_mode() {
        assert_eq!(validate_system_auto_start_for_mode(false, false), Ok(()));
    }

    #[test]
    fn validate_system_auto_start_allows_enabled_in_supported_mode() {
        assert_eq!(validate_system_auto_start_for_mode(true, true), Ok(()));
    }
}
