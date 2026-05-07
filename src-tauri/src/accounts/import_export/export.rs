use crate::{
    accounts::decode_jwt_payload,
    json_util::{raw_string_field, string_field},
};
use serde_json::{json, Value};
use time::OffsetDateTime;

pub(crate) fn local_date_for_filename() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    )
}

pub(crate) fn build_export_account_item(account: &Value) -> Option<Value> {
    let tokens = account.get("tokens").unwrap_or(&Value::Null);
    let refresh_token = raw_string_field(tokens, "refresh_token");
    if refresh_token.is_empty() {
        return None;
    }

    let email = decode_jwt_payload(&raw_string_field(tokens, "id_token"))
        .ok()
        .map(|claims| string_field(&claims, "email"))
        .unwrap_or_default();

    Some(json!({
        "email": email,
        "account_id": raw_string_field(tokens, "account_id"),
        "refresh_token": refresh_token
    }))
}
