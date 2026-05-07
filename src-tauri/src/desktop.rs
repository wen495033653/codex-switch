mod autostart;
mod runtime;
mod tray;
mod window;

const MAIN_WINDOW_LABEL: &str = "main";

pub(crate) use autostart::{sync_system_auto_start, sync_system_auto_start_from_settings};
pub(crate) use runtime::AppRuntime;
pub(crate) use tray::setup_tray;
pub(crate) use window::{focus_main_window, handle_main_window_event, restore_main_window_state};
