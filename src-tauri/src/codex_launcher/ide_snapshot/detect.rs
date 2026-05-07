use super::*;

pub(crate) fn executable_leaf_name(name: &str, executable_path: &str) -> String {
    let trimmed = name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    Path::new(executable_path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string()
}

pub(crate) fn normalize_executable_path(path: &str) -> String {
    path.trim().to_ascii_lowercase().replace('/', "\\")
}

pub(crate) fn detect_ide_app(
    name: &str,
    executable_path: &str,
) -> Option<(&'static str, &'static str)> {
    let normalized_name = executable_leaf_name(name, executable_path).to_ascii_lowercase();
    let normalized_path = normalize_executable_path(executable_path);

    if normalized_name == "codex.exe"
        && normalized_path.contains("\\openai.codex_")
        && normalized_path.contains("\\app\\codex.exe")
        && !normalized_path.contains("\\app\\resources\\codex.exe")
    {
        return Some(("codex", "Codex"));
    }

    if normalized_name == "code.exe"
        && normalized_path.ends_with("\\code.exe")
        && (normalized_path.contains("\\microsoft vs code\\")
            || normalized_path.contains("\\microsoft vs code insiders\\"))
    {
        return Some(("vscode", "VS Code"));
    }

    None
}

pub(crate) fn process_entry_pid(value: &Value) -> u64 {
    value_u64_field(value, "pid")
        .or_else(|| value_u64_field(value, "ProcessId"))
        .unwrap_or(0)
}

fn process_entry_name(value: &Value) -> String {
    let name = raw_string_field(value, "name");
    if name.is_empty() {
        raw_string_field(value, "Name")
    } else {
        name
    }
}

pub(crate) fn process_entry_executable_path(value: &Value) -> String {
    let path = raw_string_field(value, "executablePath");
    if path.is_empty() {
        raw_string_field(value, "ExecutablePath")
    } else {
        path
    }
}

pub(crate) fn normalize_ide_entries(items: Vec<Value>) -> Vec<Value> {
    let mut entries = Vec::new();
    for item in items {
        let pid = process_entry_pid(&item);
        let raw_name = process_entry_name(&item);
        let executable_path = process_entry_executable_path(&item);
        if pid == 0 || executable_path.trim().is_empty() {
            continue;
        }

        let name = executable_leaf_name(&raw_name, &executable_path);
        let Some((kind, display_name)) = detect_ide_app(&name, &executable_path) else {
            continue;
        };
        entries.push(json!({
            "pid": pid,
            "name": name,
            "executablePath": executable_path,
            "kind": kind,
            "displayName": display_name
        }));
    }
    entries
}

pub(crate) fn build_ide_summary(entries: &[Value]) -> Value {
    let mut codex_paths = HashSet::new();
    let mut vscode_paths = HashSet::new();

    for entry in entries {
        let kind = string_field(entry, "kind");
        let executable_path = normalize_executable_path(&raw_string_field(entry, "executablePath"));
        if executable_path.is_empty() {
            continue;
        }
        match kind.as_str() {
            "codex" => {
                codex_paths.insert(executable_path);
            }
            "vscode" => {
                vscode_paths.insert(executable_path);
            }
            _ => {}
        }
    }

    let mut summary = Vec::new();
    if !codex_paths.is_empty() {
        summary.push(json!({
            "key": "codex",
            "displayName": "Codex",
            "count": codex_paths.len()
        }));
    }
    if !vscode_paths.is_empty() {
        summary.push(json!({
            "key": "vscode",
            "displayName": "VS Code",
            "count": vscode_paths.len()
        }));
    }
    Value::Array(summary)
}
