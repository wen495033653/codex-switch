mod io;
mod patch;

use super::normalize::normalize_settings;
pub(crate) use io::read_settings_value;
use io::write_settings_value;
use patch::apply_settings_patch;
use serde_json::Value;
use std::sync::Mutex;

/// Serializes settings.json read-modify-write cycles. The window-state saver, background
/// account checks and commands patch different keys of the same file; without the lock the
/// later writer restores the keys the earlier one just changed.
static SETTINGS_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn update_settings_value(patch: &Value) -> Result<Value, String> {
    // The lock guards the file, which is replaced atomically; a poisoned lock carries no
    // half-written state.
    let _guard = SETTINGS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut settings = read_settings_value()?;
    let object = settings
        .as_object_mut()
        .ok_or_else(|| "settings 数据结构无效".to_string())?;
    apply_settings_patch(object, patch)?;

    let normalized = normalize_settings(&settings);
    write_settings_value(&normalized)?;
    Ok(normalized)
}
