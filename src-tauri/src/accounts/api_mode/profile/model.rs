use crate::{
    api_config::{api_provider_name, normalize_api_base_url},
    json_util::string_field,
};
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ApiModeProfile {
    pub(super) name: String,
    pub(super) base_url: String,
    pub(super) api_key: String,
}

impl ApiModeProfile {
    pub(super) fn from_value(value: &Value) -> Result<Self, String> {
        Ok(Self {
            name: string_field(value, "name"),
            base_url: normalize_api_base_url(&string_field(value, "base_url"))?,
            api_key: string_field(value, "api_key"),
        })
    }

    pub(super) fn provider_name(&self) -> String {
        api_provider_name(&self.name)
    }
}
