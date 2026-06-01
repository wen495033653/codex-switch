use super::*;
use crate::json_file::{read_json_file, write_json_file};

pub(super) fn import_accounts_impl(app: AppHandle) -> Result<Value, String> {
    let selected = app
        .dialog()
        .file()
        .set_title("导入账号")
        .add_filter("JSON", &["json"])
        .blocking_pick_file()
        .ok_or_else(|| "导入已取消".to_string())?;
    let path = selected
        .into_path()
        .map_err(|err| format!("导入文件路径无效: {err}"))?;

    let incoming = read_json_file(&path, "导入文件")?;
    let refresh_tokens = extract_refresh_tokens_from_data(&incoming);
    if refresh_tokens.is_empty() {
        return Err("导入文件不包含 refresh_token".to_string());
    }

    let choice = app
        .dialog()
        .message(format!(
            "来源文件：{}\n识别账号数量：{} 个\n\n导入规则：账号 ID 已存在时自动覆盖，不存在时自动合并新增。",
            path.display(),
            refresh_tokens.len()
        ))
        .title("确认导入账号")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "导入".to_string(),
            "取消".to_string(),
        ))
        .blocking_show_with_result();
    match choice {
        MessageDialogResult::Ok => {}
        MessageDialogResult::Custom(label) if label == "导入" => {}
        _ => return Err("导入已取消".to_string()),
    }

    let payload = import_accounts_from_refresh_tokens(refresh_tokens, false)?;
    let _ = read_store_with_active_sync();
    Ok(payload)
}

pub(super) fn export_accounts_impl(app: AppHandle) -> Result<Value, String> {
    let default_name = format!("codex_accounts_{}.json", local_date_for_filename());
    let selected = app
        .dialog()
        .file()
        .set_title("导出 refresh_token")
        .set_file_name(default_name)
        .add_filter("JSON", &["json"])
        .blocking_save_file()
        .ok_or_else(|| "导出已取消".to_string())?;
    let path = selected
        .into_path()
        .map_err(|err| format!("导出文件路径无效: {err}"))?;

    let store = read_store_with_active_sync()?;
    let mut seen = HashSet::new();
    let mut export_accounts = Vec::new();
    for account in store
        .get("accounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let Some(item) = build_export_account_item(&account) else {
            continue;
        };
        let refresh_token = raw_string_field(&item, "refresh_token");
        if refresh_token.is_empty() || !seen.insert(refresh_token) {
            continue;
        }
        export_accounts.push(item);
    }

    let payload = json!({
        "version": 2,
        "accounts": export_accounts
    });
    write_json_file(&path, "导出文件", &payload)?;

    Ok(store_payload_from_store(
        store,
        Some(&format!(
            "导出成功（{} 个 refresh_token）",
            payload
                .get("accounts")
                .and_then(Value::as_array)
                .map(|accounts| accounts.len())
                .unwrap_or(0)
        )),
    ))
}
