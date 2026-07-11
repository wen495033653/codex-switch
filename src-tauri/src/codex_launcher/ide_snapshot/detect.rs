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
    path.trim().to_ascii_lowercase().replace('\\', "/")
}

const WINDOWS_CODEX_APP_PACKAGE_PATH_MARKER: &str = "/openai.codex_";
const MACOS_CHATGPT_EXECUTABLE_SUFFIXES: [&str; 2] = [
    "/chatgpt.app/contents/macos/chatgpt",
    "/codex.app/contents/macos/chatgpt",
];
const MACOS_VSCODE_EXECUTABLE_SUFFIXES: [&str; 2] = [
    "/visual studio code.app/contents/macos/electron",
    "/visual studio code - insiders.app/contents/macos/electron",
];

fn codex_desktop_executable_name(name: &str, executable_path: &str) -> Option<String> {
    let normalized_name = executable_leaf_name(name, executable_path).to_ascii_lowercase();
    let normalized_path = normalize_executable_path(executable_path);
    if normalized_name == "chatgpt.exe"
        && normalized_path.contains(WINDOWS_CODEX_APP_PACKAGE_PATH_MARKER)
        && normalized_path.ends_with("/app/chatgpt.exe")
    {
        return Some(normalized_name);
    }
    if normalized_name == "chatgpt"
        && MACOS_CHATGPT_EXECUTABLE_SUFFIXES
            .iter()
            .any(|suffix| normalized_path.ends_with(suffix))
    {
        return Some(normalized_name);
    }
    None
}

pub(crate) fn codex_desktop_display_name(_executable_path: &str) -> &'static str {
    "ChatGPT (Codex)"
}

pub(crate) fn detect_ide_app(
    name: &str,
    executable_path: &str,
) -> Option<(&'static str, &'static str)> {
    let normalized_name = executable_leaf_name(name, executable_path).to_ascii_lowercase();
    let normalized_path = normalize_executable_path(executable_path);

    if codex_desktop_executable_name(&normalized_name, &normalized_path).is_some() {
        return Some(("codex", "ChatGPT (Codex)"));
    }

    let windows_vscode = normalized_name == "code.exe"
        && normalized_path.ends_with("/code.exe")
        && (normalized_path.contains("/microsoft vs code/")
            || normalized_path.contains("/microsoft vs code insiders/"));
    let macos_vscode = normalized_name == "electron"
        && MACOS_VSCODE_EXECUTABLE_SUFFIXES
            .iter()
            .any(|suffix| normalized_path.ends_with(suffix));
    if windows_vscode || macos_vscode {
        return Some(("vscode", "VS Code"));
    }

    None
}

pub(crate) fn process_entry_pid(value: &Value) -> u64 {
    value_u64_field(value, "pid")
        .or_else(|| value_u64_field(value, "ProcessId"))
        .unwrap_or(0)
}

pub(crate) fn process_entry_parent_pid(value: &Value) -> u64 {
    value_u64_field(value, "parentPid")
        .or_else(|| value_u64_field(value, "ParentProcessId"))
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

fn process_entry_command_line(value: &Value) -> String {
    let command_line = raw_string_field(value, "commandLine");
    if command_line.is_empty() {
        raw_string_field(value, "CommandLine")
    } else {
        command_line
    }
}

pub(crate) fn normalize_ide_entries(items: Vec<Value>) -> Vec<Value> {
    let mut entries = Vec::new();
    for item in items {
        let pid = process_entry_pid(&item);
        let raw_name = process_entry_name(&item);
        let executable_path = process_entry_executable_path(&item);
        let command_line = process_entry_command_line(&item);
        if pid == 0 || executable_path.trim().is_empty() {
            continue;
        }

        let name = executable_leaf_name(&raw_name, &executable_path);
        let Some((kind, display_name)) = detect_ide_app(&name, &executable_path) else {
            continue;
        };
        entries.push(json!({
            "pid": pid,
            "parentPid": process_entry_parent_pid(&item),
            "name": name,
            "executablePath": executable_path,
            "commandLine": command_line,
            "kind": kind,
            "displayName": display_name
        }));
    }
    entries
}

pub(crate) fn build_ide_summary(entries: &[Value]) -> Value {
    let mut codex_paths = HashSet::new();
    let mut vscode_paths = HashSet::new();
    let mut codex_display_name = "Codex";

    for entry in entries {
        let kind = string_field(entry, "kind");
        let executable_path = normalize_executable_path(&raw_string_field(entry, "executablePath"));
        if executable_path.is_empty() {
            continue;
        }
        match kind.as_str() {
            "codex" => {
                codex_paths.insert(executable_path);
                if raw_string_field(entry, "displayName") == "ChatGPT (Codex)" {
                    codex_display_name = "ChatGPT (Codex)";
                }
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
            "displayName": codex_display_name,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_legacy_codex_desktop_executable() {
        let path = r"C:\Program Files\WindowsApps\OpenAI.Codex_26.623.5175.0_x64__2p2nqsd0c76g0\app\Codex.exe";

        assert_eq!(detect_ide_app("Codex.exe", path), None);
    }

    #[test]
    fn detects_chatgpt_hosted_codex_desktop_executable() {
        let path = r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.3748.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe";

        assert_eq!(
            detect_ide_app("ChatGPT.exe", path),
            Some(("codex", "ChatGPT (Codex)"))
        );
        assert_eq!(codex_desktop_display_name(path), "ChatGPT (Codex)");
    }

    #[test]
    fn detects_macos_chatgpt_and_vscode_apps() {
        assert_eq!(
            detect_ide_app(
                "ChatGPT",
                "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"
            ),
            Some(("codex", "ChatGPT (Codex)"))
        );
        assert_eq!(
            detect_ide_app("ChatGPT", "/Applications/Codex.app/Contents/MacOS/ChatGPT"),
            Some(("codex", "ChatGPT (Codex)"))
        );
        assert_eq!(
            detect_ide_app(
                "Electron",
                "/Applications/Visual Studio Code.app/Contents/MacOS/Electron"
            ),
            Some(("vscode", "VS Code"))
        );
    }

    #[test]
    fn rejects_codex_cli_and_unrelated_chatgpt_executables() {
        assert_eq!(
            detect_ide_app(
                "codex.exe",
                r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.3748.0_x64__2p2nqsd0c76g0\app\resources\codex.exe"
            ),
            None
        );
        assert_eq!(
            detect_ide_app("ChatGPT.exe", r"C:\Tools\ChatGPT\ChatGPT.exe"),
            None
        );
        assert_eq!(
            detect_ide_app("Codex", "/Applications/Codex.app/Contents/MacOS/Codex"),
            None
        );
    }
}
