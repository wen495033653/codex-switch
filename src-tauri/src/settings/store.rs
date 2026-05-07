mod io;
mod patch;

use super::normalize::normalize_settings;
pub(crate) use io::read_settings_value;
use io::write_settings_value;
use patch::apply_settings_patch;
use serde_json::Value;

pub(crate) fn update_settings_value(patch: &Value) -> Result<Value, String> {
    let mut settings = read_settings_value()?;
    let object = settings
        .as_object_mut()
        .ok_or_else(|| "settings 数据结构无效".to_string())?;
    apply_settings_patch(object, patch)?;

    let normalized = normalize_settings(&settings);
    write_settings_value(&normalized)?;
    Ok(normalized)
}
