use super::super::{defaults::default_settings, normalize::normalize_settings};
use crate::{
    json_file::{read_json_file, write_json_file},
    paths::settings_path,
};
use serde_json::Value;

pub(crate) fn read_settings_value() -> Result<Value, String> {
    let path = settings_path()?;
    if !path.exists() {
        let settings = default_settings();
        write_settings_value(&settings)?;
        return Ok(settings);
    }

    let parsed = read_json_file(&path, "settings.json")?;
    Ok(normalize_settings(&parsed))
}

pub(super) fn write_settings_value(settings: &Value) -> Result<(), String> {
    let path = settings_path()?;
    let normalized = normalize_settings(settings);
    write_json_file(&path, "settings.json", &normalized)
}
