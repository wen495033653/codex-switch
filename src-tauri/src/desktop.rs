mod autostart;
mod runtime;
mod tray;
mod window;

const MAIN_WINDOW_LABEL: &str = "main";

pub(crate) use autostart::{
    sync_system_auto_start, sync_system_auto_start_from_settings, AUTO_START_LAUNCH_ARG,
};
pub(crate) use runtime::AppRuntime;
pub(crate) use tray::setup_tray;
pub(crate) use window::{
    apply_main_window_startup_behavior, focus_main_window, handle_main_window_event,
    restore_main_window_state,
};
