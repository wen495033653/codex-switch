use super::*;

#[derive(Clone, Debug)]
pub(super) struct AccountStore {
    active_id: String,
    accounts: Vec<StoreAccount>,
}

impl AccountStore {
    pub(super) fn empty() -> Self {
        Self {
            active_id: String::new(),
            accounts: Vec::new(),
        }
    }

    pub(super) fn normalize(data: &Value) -> Result<Self, String> {
        let version = data.get("version").and_then(Value::as_i64).unwrap_or(0);
        if version != STORE_VERSION {
            return Err(format!(
                "accounts.json 版本不匹配：期望 {STORE_VERSION}，实际 {}",
                if version == 0 {
                    "unknown".to_string()
                } else {
                    version.to_string()
                }
            ));
        }

        let active_id = raw_string_field(data, "active_id");
        let mut accounts: Vec<StoreAccount> = Vec::new();
        for item in data
            .get("accounts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let account = StoreAccount::normalize(&item)?;
            if let Some(index) = accounts
                .iter()
                .position(|existing| existing.profile_id() == account.profile_id())
            {
                accounts[index] = account;
            } else {
                accounts.push(account);
            }
        }
        accounts.sort_by(|a, b| b.last_used_at().cmp(a.last_used_at()));
        let active_id = if active_id.is_empty()
            || accounts
                .iter()
                .any(|account| account.profile_id() == active_id)
        {
            active_id
        } else {
            accounts
                .iter()
                .find(|account| account.account_id() == active_id)
                .map(|account| account.profile_id().to_string())
                .unwrap_or(active_id)
        };
        Ok(Self {
            active_id,
            accounts,
        })
    }

    pub(super) fn to_value(&self) -> Value {
        json!({
            "version": STORE_VERSION,
            "active_id": self.active_id,
            "accounts": self.accounts.iter().map(StoreAccount::to_value).collect::<Vec<_>>()
        })
    }
}
