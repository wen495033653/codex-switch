use crate::{
    accounts::{
        account_from_exchange_syncing, add_account_to_store, build_oauth_auth_url, generate_pkce,
        random_urlsafe, set_subscription_mode,
    },
    events::emit_store_updated,
    json_util::string_field,
    quota::sync_account_usage_in_background,
};
use serde_json::{json, Value};
use std::{
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
        Arc,
    },
    thread,
};
use tauri::{AppHandle, State};

use super::{
    callback_server::wait_for_oauth_exchange,
    emit_oauth_update,
    runtime::{finish_oauth_flow, start_oauth_flow, OAuthRuntime},
    OAUTH_CALLBACK_PORT, OAUTH_CANCEL_MESSAGE,
};

pub(super) fn oauth_start_impl(
    app: AppHandle,
    runtime: State<'_, Arc<OAuthRuntime>>,
    payload: Option<Value>,
) -> Result<Value, String> {
    let _ = payload;
    let runtime = Arc::clone(runtime.inner());
    let (flow_id, canceled, manual_callbacks) = start_oauth_flow(runtime.as_ref())?;

    thread::spawn(move || {
        let result = run_oauth_flow(app.clone(), Arc::clone(&canceled), manual_callbacks);

        finish_oauth_flow(runtime.as_ref(), flow_id);
        if let Err(message) = &result {
            if message == OAUTH_CANCEL_MESSAGE {
                return;
            }

            let error_code = if message.contains("授权超时") {
                "OAUTH_TIMEOUT"
            } else {
                ""
            };
            emit_oauth_update(
                &app,
                json!({
                    "running": false,
                    "url": "",
                    "success": false,
                    "error": message,
                    "errorCode": error_code
                }),
            );
        }
    });

    Ok(json!({
        "ok": true,
        "started": true
    }))
}

fn run_oauth_flow(
    app: AppHandle,
    canceled: Arc<AtomicBool>,
    manual_callbacks: Receiver<String>,
) -> Result<(), String> {
    set_subscription_mode()?;
    let listener =
        TcpListener::bind(("127.0.0.1", OAUTH_CALLBACK_PORT)).map_err(|err| match err.kind() {
            std::io::ErrorKind::AddrInUse => format!(
                "OAuth 回调端口 {} 已被占用，请关闭占用进程后重试",
                OAUTH_CALLBACK_PORT
            ),
            std::io::ErrorKind::PermissionDenied => format!(
                "OAuth 回调端口 {} 不可用（EACCES），可能被系统策略限制",
                OAUTH_CALLBACK_PORT
            ),
            _ => format!("启动 OAuth 回调服务失败: {err}"),
        })?;

    let (verifier, challenge) = generate_pkce();
    let state = random_urlsafe(32);
    let auth_url = build_oauth_auth_url(OAUTH_CALLBACK_PORT, &challenge, &state)?;
    emit_oauth_update(
        &app,
        json!({
            "running": true,
            "url": auth_url,
            "success": false,
            "error": "",
            "errorCode": ""
        }),
    );

    open_oauth_url_in_background(app.clone(), auth_url.clone());

    let exchange = wait_for_oauth_exchange(
        listener,
        &state,
        &verifier,
        Arc::clone(&canceled),
        manual_callbacks,
    )?;
    if canceled.load(Ordering::SeqCst) {
        return Err(OAUTH_CANCEL_MESSAGE.to_string());
    }

    let account_id = string_field(&exchange, "account_id");
    let access_token = string_field(&exchange, "access_token");
    let account = account_from_exchange_syncing(&exchange, None)?;
    let store = add_account_to_store(account, false)?;
    emit_store_updated(&app, store);
    emit_oauth_update(
        &app,
        json!({
            "running": false,
            "url": "",
            "success": true,
            "error": "",
            "errorCode": "",
            "message": "账号已添加，正在同步配额"
        }),
    );
    sync_account_usage_in_background(app.clone(), account_id, access_token);
    Ok(())
}

fn open_oauth_url_in_background(app: AppHandle, auth_url: String) {
    thread::spawn(move || {
        if open::that(&auth_url).is_err() {
            emit_oauth_update(
                &app,
                json!({
                    "running": true,
                    "url": auth_url,
                    "success": false,
                    "error": "未能自动打开浏览器，请复制链接后手动在浏览器中打开",
                    "errorCode": ""
                }),
            );
        }
    });
}
