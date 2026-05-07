use super::*;
use crate::{accounts::normalize_custom, time_util::now_string};

#[derive(Clone, Debug)]
pub(super) struct StoreAccount {
    tokens: AccountTokens,
    custom: Value,
}

impl StoreAccount {
    pub(super) fn normalize(value: &Value) -> Result<Self, String> {
        let tokens = AccountTokens::from_account_value(value)?;
        let mut custom = normalize_custom(value.get(CUSTOM_FIELD));
        if raw_string_field(&custom, CREATED_AT_FIELD).is_empty() {
            custom[CREATED_AT_FIELD] = Value::String(now_string());
        }
        if raw_string_field(&custom, LAST_USED_AT_FIELD).is_empty() {
            custom[LAST_USED_AT_FIELD] = custom
                .get(CREATED_AT_FIELD)
                .cloned()
                .unwrap_or_else(|| Value::String(now_string()));
        }
        Ok(Self { tokens, custom })
    }

    pub(super) fn account_id(&self) -> &str {
        self.tokens.account_id()
    }

    pub(super) fn last_used_at(&self) -> &str {
        self.custom
            .get(LAST_USED_AT_FIELD)
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub(super) fn to_value(&self) -> Value {
        json!({
            TOKENS_FIELD: self.tokens.to_value(),
            CUSTOM_FIELD: self.custom
        })
    }
}
