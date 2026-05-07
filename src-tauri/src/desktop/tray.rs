use super::{focus_main_window, AppRuntime};
use std::sync::atomic::Ordering;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

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

    let icon = Image::from_bytes(include_bytes!("../../../build/icon.png"))
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
