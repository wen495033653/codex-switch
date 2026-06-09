use crate::{json_util::bool_field, settings::read_settings_value};
use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt as AutoStartManagerExt;

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
