use super::{AppRuntime, MAIN_WINDOW_LABEL};
use crate::{
    json_util::bool_field,
    settings::{read_settings_value, update_settings_value},
};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, CloseRequestApi, Manager, PhysicalSize, Size, WebviewWindow, WindowEvent};

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(MAIN_WINDOW_LABEL)
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

pub(crate) fn handle_main_window_event(window: &tauri::Window, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW_LABEL {
        return;
    }
    match event {
        WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
            if let Err(err) = persist_main_window_state(window) {
                eprintln!("保存窗口状态失败: {err}");
            }
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
