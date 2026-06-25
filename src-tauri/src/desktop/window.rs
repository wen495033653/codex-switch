use super::{AppRuntime, AUTO_START_LAUNCH_ARG, MAIN_WINDOW_LABEL};
use crate::{
    json_util::{bool_field, string_field},
    settings::{read_settings_value, update_settings_value},
};
use serde_json::{json, Value};
use std::{sync::atomic::Ordering, thread, time::Duration as StdDuration};
use tauri::{
    AppHandle, CloseRequestApi, Manager, PhysicalSize, Size, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

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

pub(crate) fn open_dev_log_window(app: AppHandle) -> Result<Value, String> {
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

pub(crate) fn hide_dev_log_window(app: AppHandle) -> Result<Value, String> {
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
        WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
            schedule_main_window_state_persist(window)
        }
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
