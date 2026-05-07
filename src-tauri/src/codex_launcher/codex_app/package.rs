use super::*;

pub(super) fn get_codex_package_info() -> Result<Value, String> {
    let output = run_pwsh(GET_CODEX_PACKAGE_INFO)?;
    let info = parse_json_output(&output, Value::Null)?;
    if string_field(&info, "ExecutablePath").is_empty()
        || string_field(&info, "InstallLocation").is_empty()
    {
        return Err("Codex package info is incomplete.".to_string());
    }
    Ok(info)
}
