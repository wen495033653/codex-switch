mod callback_server;
mod cancel;
mod runtime;
mod start;

use cancel::oauth_cancel_impl;
use serde_json::Value;
use start::oauth_start_impl;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

pub(crate) use runtime::OAuthRuntime;

const OAUTH_CALLBACK_PORT: u16 = 1455;
const OAUTH_CANCEL_MESSAGE: &str = "OAuth 登录已取消";

pub(crate) fn emit_oauth_update(app: &AppHandle, payload: Value) {
    let _ = app.emit("oauth-update", payload);
}

#[tauri::command(async)]
pub(crate) fn oauth_start(
    app: AppHandle,
    runtime: State<'_, Arc<OAuthRuntime>>,
    payload: Option<Value>,
) -> Result<Value, String> {
    oauth_start_impl(app, runtime, payload)
}

#[tauri::command(async)]
pub(crate) fn oauth_cancel(app: AppHandle, runtime: State<'_, Arc<OAuthRuntime>>) -> Value {
    oauth_cancel_impl(app, runtime)
}
