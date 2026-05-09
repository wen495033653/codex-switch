#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use tauri::Manager;

mod accounts;
mod api_config;
mod codex_config;
mod codex_launcher;
mod codex_session_usage;
mod codex_sessions;
mod commands;
mod desktop;
mod events;
mod json_file;
mod json_util;
mod oauth_flow;
mod paths;
mod proxy_config;
mod quota;
mod settings;
mod time_util;
mod updater;

use codex_launcher::IdeRuntime;
use desktop::{
    focus_main_window, handle_main_window_event, restore_main_window_state, setup_tray,
    sync_system_auto_start_from_settings, AppRuntime,
};
use oauth_flow::OAuthRuntime;
use quota::{
    start_account_token_auto_refresher, start_active_quota_auto_refresher,
    start_background_quota_auto_refresher, RefreshAllRuntime,
};
use updater::UpdateRuntime;

fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    tauri::Builder::default()
        .manage(AppRuntime::default())
        .manage(OAuthRuntime::default())
        .manage(Arc::new(RefreshAllRuntime::default()))
        .manage(Arc::new(UpdateRuntime::default()))
        .manage(Arc::new(IdeRuntime::default()))
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            focus_main_window(app);
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("Codex Switch")
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            restore_main_window_state(app.handle()).map_err(setup_error)?;
            setup_tray(app.handle()).map_err(setup_error)?;
            if let Err(err) = sync_system_auto_start_from_settings(app.handle()) {
                eprintln!("同步开机自启状态失败: {err}");
            }
            if let Err(err) = accounts::restore_api_mode_if_selected() {
                eprintln!("恢复 Codex API 模式失败: {err}");
            }
            start_account_token_auto_refresher(app.handle().clone());
            start_active_quota_auto_refresher(app.handle().clone());
            start_background_quota_auto_refresher(
                app.handle().clone(),
                Arc::clone(app.state::<Arc<RefreshAllRuntime>>().inner()),
            );
            codex_sessions::start_codex_session_sync_watcher();
            Ok(())
        })
        .on_window_event(|window, event| {
            handle_main_window_event(window, event);
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_store,
            commands::get_app_version,
            commands::get_data_dir,
            commands::open_data_dir,
            commands::get_refresh_all_status,
            commands::get_settings,
            commands::update_settings,
            commands::capture_current,
            commands::import_refresh_token,
            commands::delete_account,
            commands::switch_account,
            commands::switch_api_mode,
            codex_launcher::set_codex_proxy_env_enabled,
            codex_launcher::restart_open_ides,
            codex_launcher::discard_ide_snapshot,
            codex_sessions::sync_codex_sessions,
            commands::import_accounts,
            commands::export_accounts,
            commands::refresh_all_quotas,
            commands::refresh_account,
            commands::refresh_account_token,
            commands::copy_text,
            updater::check_update,
            updater::download_update,
            updater::install_update,
            updater::dismiss_update_version,
            commands::open_external_url,
            commands::open_codex_config_toml,
            commands::list_brand_voice_files,
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
