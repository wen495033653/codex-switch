use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;

use super::{
    callback_server::parse_manual_callback_url,
    runtime::{submit_oauth_callback, OAuthRuntime},
};

pub(super) fn oauth_submit_callback_impl(
    runtime: State<'_, Arc<OAuthRuntime>>,
    callback_url: String,
) -> Result<Value, String> {
    let url = parse_manual_callback_url(&callback_url)?;
    submit_oauth_callback(runtime.as_ref(), url.to_string())?;

    Ok(json!({
        "ok": true,
        "message": "回调 URL 已提交，正在完成 OAuth 登录"
    }))
}
