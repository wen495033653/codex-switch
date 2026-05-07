use serde_json::{json, Value};
use tauri::{AppHandle, State};

use super::{
    emit_oauth_update,
    runtime::{cancel_oauth_flow, OAuthRuntime},
    OAUTH_CANCEL_MESSAGE,
};

pub(super) fn oauth_cancel_impl(app: AppHandle, runtime: State<'_, OAuthRuntime>) -> Value {
    let canceled = cancel_oauth_flow(&runtime);

    if canceled {
        emit_oauth_update(
            &app,
            json!({
                "running": false,
                "url": "",
                "success": false,
                "error": OAUTH_CANCEL_MESSAGE,
                "errorCode": "OAUTH_CANCELED"
            }),
        );
    }

    json!({
        "ok": true,
        "canceled": canceled,
        "message": if canceled {
            "已取消 OAuth 登录"
        } else {
            "当前没有进行中的 OAuth 登录"
        }
    })
}
