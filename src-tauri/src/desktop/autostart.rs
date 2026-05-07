use crate::{json_util::bool_field, settings::read_settings_value};
use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt as AutoStartManagerExt;

pub(crate) fn sync_system_auto_start(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    if enabled {
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
