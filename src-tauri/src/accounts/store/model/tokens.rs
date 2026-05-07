use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AccountTokens {
    id_token: String,
    access_token: String,
    refresh_token: String,
    account_id: String,
}

impl AccountTokens {
    pub(super) fn from_value(value: Option<&Value>) -> Result<Self, String> {
        let raw = value.ok_or_else(|| "账号缺少 tokens".to_string())?;
        let tokens = Self {
            id_token: raw_string_field(raw, ID_TOKEN_FIELD),
            access_token: raw_string_field(raw, ACCESS_TOKEN_FIELD),
            refresh_token: raw_string_field(raw, REFRESH_TOKEN_FIELD),
            account_id: raw_string_field(raw, ACCOUNT_ID_FIELD),
        };
        tokens.validate()?;
        Ok(tokens)
    }

    pub(super) fn from_account_value(account: &Value) -> Result<Self, String> {
        Self::from_value(account.get(TOKENS_FIELD))
    }

    fn validate(&self) -> Result<(), String> {
        if self.id_token.is_empty() {
            return Err("账号缺少 tokens.id_token".to_string());
        }
        if self.access_token.is_empty() {
            return Err("账号缺少 tokens.access_token".to_string());
        }
        if self.refresh_token.is_empty() {
            return Err("账号缺少 tokens.refresh_token".to_string());
        }
        if self.account_id.is_empty() {
            return Err("账号缺少 tokens.account_id".to_string());
        }
        Ok(())
    }

    pub(super) fn account_id(&self) -> &str {
        &self.account_id
    }

    pub(super) fn to_value(&self) -> Value {
        json!({
            ID_TOKEN_FIELD: self.id_token,
            ACCESS_TOKEN_FIELD: self.access_token,
            REFRESH_TOKEN_FIELD: self.refresh_token,
            ACCOUNT_ID_FIELD: self.account_id
        })
    }
}
