mod callback_server;
mod cancel;
mod runtime;
mod start;
mod submit;

use cancel::oauth_cancel_impl;
use serde_json::Value;
use start::oauth_start_impl;
use submit::oauth_submit_callback_impl;
use tauri::{AppHandle, Emitter, State};

pub(crate) use runtime::OAuthRuntime;

const OAUTH_CALLBACK_PORT: u16 = 1455;
const OAUTH_CANCEL_MESSAGE: &str = "OAuth 登录已取消";

pub(crate) fn emit_oauth_update(app: &AppHandle, payload: Value) {
    let _ = app.emit("oauth-update", payload);
}

#[tauri::command]
pub(crate) fn oauth_start(
    app: AppHandle,
    runtime: State<'_, OAuthRuntime>,
    payload: Option<Value>,
) -> Result<Value, String> {
    oauth_start_impl(app, runtime, payload)
}

#[tauri::command]
pub(crate) fn oauth_cancel(app: AppHandle, runtime: State<'_, OAuthRuntime>) -> Value {
    oauth_cancel_impl(app, runtime)
}

#[tauri::command]
pub(crate) fn oauth_submit_callback(
    runtime: State<'_, OAuthRuntime>,
    callback_url: String,
) -> Result<Value, String> {
    oauth_submit_callback_impl(runtime, callback_url)
}
