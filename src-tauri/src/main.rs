#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use serde_json::json;
use std::sync::Arc;
use tauri::Manager;

mod accounts;
mod api_config;
mod atomic_file;
mod blocking_task;
mod codex_app_server;
mod codex_config;
mod codex_launcher;
mod codex_session_usage;
mod codex_sessions;
mod commands;
mod desktop;
mod events;
mod json_file;
mod json_util;
mod model_instructions;
mod oauth_flow;
mod paths;
mod proxy_config;
mod quota;
mod session_manager;
mod session_sync_diagnostics;
mod settings;
mod time_util;
mod updater;
mod usage_stats;

use codex_launcher::IdeRuntime;
use desktop::{
    apply_main_window_startup_behavior, focus_main_window, handle_main_window_event,
    restore_main_window_state, setup_tray, sync_system_auto_start_from_settings, AppRuntime,
    AUTO_START_LAUNCH_ARG,
};
use oauth_flow::OAuthRuntime;
use quota::{
    start_account_token_auto_refresher, start_active_quota_auto_refresher,
    start_background_quota_auto_refresher, RefreshAllRuntime,
};
use updater::UpdateRuntime;

fn main() {
    let dev_preview =
        cfg!(debug_assertions) && std::env::var("CODEX_SWITCH_DEV_PREVIEW").as_deref() == Ok("1");
    let _ = rustls::crypto::ring::default_provider().install_default();

    tauri::Builder::default()
        .manage(AppRuntime::default())
        .manage(Arc::new(OAuthRuntime::default()))
        .manage(Arc::new(RefreshAllRuntime::default()))
        .manage(Arc::new(UpdateRuntime::default()))
        .manage(Arc::new(IdeRuntime::default()))
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            focus_main_window(app);
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .arg(AUTO_START_LAUNCH_ARG)
                .app_name(if dev_preview { "Codex Switch Dev" } else { "Codex Switch" })
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            session_sync_diagnostics::init_session_sync_diagnostics(app.handle().clone());
            session_sync_diagnostics::log_session_sync_event("app_start", json!({}));
            restore_main_window_state(app.handle()).map_err(setup_error)?;
            setup_tray(app.handle()).map_err(setup_error)?;
            let launch_args: Vec<String> = std::env::args().collect();
            apply_main_window_startup_behavior(app.handle(), &launch_args).map_err(setup_error)?;
            // 独立 Dev 预览只执行用户手动操作，不在启动时同步正式版状态。
            if dev_preview {
                session_sync_diagnostics::log_session_sync_event(
                    "app_dev_preview",
                    json!({ "automaticSync": false, "dataDir": paths::app_data_dir().map_err(setup_error)?, "codexHome": paths::codex_dir().map_err(setup_error)? }),
                );
                return Ok(());
            }
            if let Err(err) = sync_system_auto_start_from_settings(app.handle()) {
                eprintln!("同步开机自启状态失败: {err}");
            }
            if let Err(err) = accounts::restore_api_mode_if_selected() {
                eprintln!("恢复 Codex API 模式失败: {err}");
            }
            if let Err(err) =
                commands::sync_codex_model_instructions_config_for_current_settings(app.handle())
            {
                eprintln!("同步 gpt破限配置失败: {err}");
            }
            if let Err(err) = usage_stats::record_current_attribution_if_available() {
                eprintln!("记录当前 token 统计归属失败: {err}");
            }
            let codex_desktop_status = codex_launcher::codex_desktop_support_status();
            let codex_desktop_supported = codex_desktop_status
                .get("supported")
                .and_then(serde_json::Value::as_bool)
                == Some(true);
            session_sync_diagnostics::log_session_sync_event(
                "codex_desktop_support_status",
                codex_desktop_status,
            );
            if codex_desktop_supported {
                match session_manager::migrate_legacy_codex_data_for_current_home() {
                    Ok(report) => session_sync_diagnostics::log_session_sync_event(
                        "codex_desktop_data_migration",
                        report,
                    ),
                    Err(err) => session_sync_diagnostics::log_session_sync_event(
                        "codex_desktop_data_migration_error",
                        json!({ "error": err }),
                    ),
                }
                if let Err(err) =
                    codex_launcher::sync_remote_control_runtime_for_current_settings("app_start")
                {
                    session_sync_diagnostics::log_session_sync_event(
                        "codex_remote_control_helper_error",
                        json!({
                            "context": "app_start",
                            "error": err
                        }),
                    );
                }
            }
            start_account_token_auto_refresher(app.handle().clone());
            start_active_quota_auto_refresher(app.handle().clone());
            start_background_quota_auto_refresher(
                app.handle().clone(),
                Arc::clone(app.state::<Arc<RefreshAllRuntime>>().inner()),
            );
            codex_launcher::start_codex_app_watcher();
            Ok(())
        })
        .on_window_event(|window, event| {
            handle_main_window_event(window, event);
        })
        .invoke_handler(tauri::generate_handler![
            commands::general::get_store,
            commands::general::get_app_version,
            commands::general::get_data_dir,
            commands::general::open_data_dir,
            desktop::open_dev_log_window,
            desktop::hide_dev_log_window,
            commands::quota::get_refresh_all_status,
            commands::general::get_settings,
            commands::general::update_settings,
            commands::general::set_codex_model_instructions_enabled,
            commands::account::capture_current,
            commands::account::import_refresh_token,
            commands::account::delete_account,
            commands::account::switch_account,
            commands::account::switch_api_mode,
            codex_launcher::set_codex_proxy_env_enabled,
            codex_launcher::remote_control::set_codex_remote_control_account_id,
            codex_launcher::remote_control::set_codex_remote_control_enabled,
            codex_launcher::remote_control::get_codex_remote_control_status,
            codex_launcher::get_current_codex_app_processes,
            codex_launcher::restart_current_codex_app_normal,
            codex_launcher::open_codex_app_instance,
            codex_launcher::show_codex_app_instance,
            codex_launcher::get_codex_app_instance_status,
            codex_launcher::ide_snapshot::restart_open_ides,
            codex_launcher::ide_snapshot::discard_ide_snapshot,
            session_sync_diagnostics::get_dev_log_entries,
            commands::account::import_accounts,
            commands::account::export_accounts,
            commands::quota::refresh_all_quotas,
            commands::account::refresh_account,
            commands::account::refresh_account_token,
            commands::general::copy_text,
            updater::check_update,
            updater::download_update,
            updater::install_update,
            updater::dismiss_update_version,
            commands::general::test_api_base_url,
            commands::general::open_external_url,
            commands::general::open_codex_config_toml,
            commands::general::list_brand_voice_files,
            usage_stats::usage_stats_get,
            session_manager::session_manager_scan,
            session_manager::session_manager_preview,
            session_manager::session_manager_preview_deleted,
            session_manager::session_manager_export,
            session_manager::session_manager_import,
            session_manager::session_manager_delete,
            session_manager::session_manager_list_deleted,
            session_manager::session_manager_restore_deleted,
            session_manager::session_manager_purge_deleted,
            session_manager::session_manager_set_status,
            oauth_flow::oauth_start,
            oauth_flow::oauth_cancel,
            oauth_flow::oauth_submit_callback
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Codex Switch");
}

fn setup_error(message: String) -> Box<dyn std::error::Error> {
    std::io::Error::other(message).into()
}
