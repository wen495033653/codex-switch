use crate::{codex_launcher::codex_desktop_cli_source_path, paths::app_data_dir};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Component, Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError},
        Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const APP_SERVER_CACHE_DIR: &str = "codex-app-server";
const CACHED_CLI_FILE_PREFIX: &str = "codex-desktop-app-server-";
const THREAD_LIST_PAGE_SIZE: u32 = 100;
const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const CLIENT_NAME: &str = "codex-switch";

static CLI_CACHE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexDesktopThread {
    pub(crate) id: String,
    pub(crate) name: Option<String>,
    pub(crate) preview: String,
    pub(crate) cwd: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) updated_at: i64,
    pub(crate) recency_at: Option<i64>,
    pub(crate) archived: bool,
}

#[derive(Debug)]
struct ThreadListPage {
    threads: Vec<CodexDesktopThread>,
    next_cursor: Option<String>,
}

enum StdoutEvent {
    Line(String),
    Error(String),
    Closed,
}

struct AppServerClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout_receiver: Receiver<StdoutEvent>,
    stderr: Arc<Mutex<String>>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    next_request_id: u64,
}

pub(crate) fn list_interactive_threads(root: &Path) -> Result<Vec<CodexDesktopThread>, String> {
    validate_codex_root(root)?;
    let cli_path = cached_codex_desktop_cli()?;
    let mut client = AppServerClient::start(&cli_path, root)?;

    let initialize_result = client.request(
        "initialize",
        json!({
            "clientInfo": {
                "name": CLIENT_NAME,
                "title": "Codex Switch",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "experimentalApi": true
            }
        }),
    )?;
    validate_initialized_codex_home(root, &initialize_result)?;

    let mut threads = list_threads_for_archive_state(&mut client, root, false)?;
    threads.extend(list_threads_for_archive_state(&mut client, root, true)?);
    Ok(threads)
}

fn validate_codex_root(root: &Path) -> Result<(), String> {
    if !root.is_absolute() {
        return Err(format!(
            "Codex Desktop app-server 要求绝对 CODEX_HOME 路径: {}",
            root.display()
        ));
    }
    if !root.is_dir() {
        return Err(format!(
            "Codex Desktop app-server 的 CODEX_HOME 不存在或不是目录: {}",
            root.display()
        ));
    }
    Ok(())
}

fn validate_initialized_codex_home(root: &Path, result: &Value) -> Result<(), String> {
    let codex_home = result
        .get("codexHome")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Codex Desktop app-server initialize 响应缺少 codexHome".to_string())?;
    if !paths_equal(root, Path::new(codex_home)) {
        return Err(format!(
            "Codex Desktop app-server 使用了错误的 CODEX_HOME：期望 {}，实际 {}",
            root.display(),
            codex_home
        ));
    }
    Ok(())
}

fn list_threads_for_archive_state(
    client: &mut AppServerClient,
    root: &Path,
    archived: bool,
) -> Result<Vec<CodexDesktopThread>, String> {
    let mut threads = Vec::new();
    let mut cursor = None;
    let mut seen_cursors = HashSet::new();

    loop {
        let result = client.request(
            "thread/list",
            thread_list_params(archived, cursor.as_deref()),
        )?;
        let page = parse_thread_list_page(root, archived, result)?;
        threads.extend(page.threads);

        let Some(next_cursor) = page.next_cursor else {
            break;
        };
        if !seen_cursors.insert(next_cursor.clone()) {
            return Err(format!(
                "Codex Desktop app-server thread/list 返回了重复分页 cursor（archived={archived}）"
            ));
        }
        cursor = Some(next_cursor);
    }

    Ok(threads)
}

fn thread_list_params(archived: bool, cursor: Option<&str>) -> Value {
    json!({
        "archived": archived,
        "cursor": cursor,
        "limit": THREAD_LIST_PAGE_SIZE,
        "sourceKinds": [],
        "modelProviders": [],
        "sortKey": "recency_at",
        "useStateDbOnly": false
    })
}

fn parse_thread_list_page(
    root: &Path,
    archived: bool,
    result: Value,
) -> Result<ThreadListPage, String> {
    let data = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "Codex Desktop app-server thread/list 响应缺少 data 数组（archived={archived}）"
            )
        })?;
    let mut threads = Vec::with_capacity(data.len());

    for value in data {
        let id = required_non_empty_string(value, "id", archived)?;
        let ephemeral = value
            .get("ephemeral")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let path = value
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let Some(path) = path else {
            if ephemeral {
                continue;
            }
            return Err(format!(
                "Codex Desktop app-server thread/list 中的非 ephemeral 会话 {id} 缺少 path"
            ));
        };
        let path = PathBuf::from(path);
        validate_thread_path(root, &path, &id)?;

        let name = value
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        let preview = required_string(value, "preview", archived)?;
        let cwd = PathBuf::from(required_non_empty_string(value, "cwd", archived)?);
        if !cwd.is_absolute() {
            return Err(format!(
                "Codex Desktop app-server thread/list 中会话 {id} 的 cwd 不是绝对路径: {}",
                cwd.display()
            ));
        }
        let updated_at = required_i64(value, "updatedAt", &id)?;
        let recency_at = optional_i64(value, "recencyAt", &id)?;

        threads.push(CodexDesktopThread {
            id,
            name,
            preview,
            cwd,
            path,
            updated_at,
            recency_at,
            archived,
        });
    }

    let next_cursor = match result.get("nextCursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if value.trim().is_empty() => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(format!(
                "Codex Desktop app-server thread/list 的 nextCursor 类型无效（archived={archived}）"
            ));
        }
    };

    Ok(ThreadListPage {
        threads,
        next_cursor,
    })
}

fn required_string(value: &Value, field: &str, archived: bool) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "Codex Desktop app-server thread/list 响应中的 {field} 无效（archived={archived}）"
            )
        })
}

fn required_non_empty_string(value: &Value, field: &str, archived: bool) -> Result<String, String> {
    required_string(value, field, archived).and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            Err(format!(
                "Codex Desktop app-server thread/list 响应中的 {field} 不能为空（archived={archived}）"
            ))
        } else {
            Ok(trimmed.to_string())
        }
    })
}

fn required_i64(value: &Value, field: &str, id: &str) -> Result<i64, String> {
    value
        .get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("Codex Desktop app-server thread/list 中会话 {id} 的 {field} 无效"))
}

fn optional_i64(value: &Value, field: &str, id: &str) -> Result<Option<i64>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_i64().map(Some).ok_or_else(|| {
            format!("Codex Desktop app-server thread/list 中会话 {id} 的 {field} 无效")
        }),
    }
}

fn validate_thread_path(root: &Path, path: &Path, id: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!(
            "Codex Desktop app-server thread/list 中会话 {id} 的 path 不是绝对路径: {}",
            path.display()
        ));
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(format!(
            "Codex Desktop app-server thread/list 中会话 {id} 的 path 包含父目录跳转: {}",
            path.display()
        ));
    }
    if !path_is_within(root, path) {
        return Err(format!(
            "Codex Desktop app-server thread/list 中会话 {id} 的 path 不属于当前 CODEX_HOME: {}",
            path.display()
        ));
    }
    Ok(())
}

fn cached_codex_desktop_cli() -> Result<PathBuf, String> {
    let source = codex_desktop_cli_source_path()?;
    let cache_dir = app_data_dir()?.join(APP_SERVER_CACHE_DIR);
    let lock = CLI_CACHE_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| "Codex Desktop CLI 缓存锁已损坏".to_string())?;
    // WindowsApps 内的 packaged executable 可读取但不能直接作为普通进程启动。
    cache_cli_from_source(&source, &cache_dir)
}

fn cache_cli_from_source(source: &Path, cache_dir: &Path) -> Result<PathBuf, String> {
    let source_metadata = fs::metadata(source).map_err(|err| {
        format!(
            "读取 Codex Desktop CLI 信息失败 {}: {err}",
            source.display()
        )
    })?;
    if !source_metadata.is_file() {
        return Err(format!(
            "Codex Desktop CLI 路径不是文件: {}",
            source.display()
        ));
    }

    fs::create_dir_all(cache_dir).map_err(|err| {
        format!(
            "创建 Codex Desktop CLI 缓存目录失败 {}: {err}",
            cache_dir.display()
        )
    })?;
    let fingerprint = source_fingerprint(source, &source_metadata);
    let target = cache_dir.join(cache_file_name(fingerprint));
    if cached_file_matches(&target, source_metadata.len()) {
        cleanup_stale_cached_clis(cache_dir, &target);
        return Ok(target);
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = cache_dir.join(format!(
        ".codex-desktop-app-server-{}-{stamp}.tmp",
        std::process::id()
    ));
    fs::copy(source, &temporary).map_err(|err| {
        format!(
            "复制 Codex Desktop CLI 到缓存失败 {} -> {}: {err}",
            source.display(),
            temporary.display()
        )
    })?;

    let copied_result = (|| {
        let copied_metadata = fs::metadata(&temporary).map_err(|err| {
            format!(
                "校验 Codex Desktop CLI 缓存失败 {}: {err}",
                temporary.display()
            )
        })?;
        if copied_metadata.len() != source_metadata.len() {
            return Err(format!(
                "Codex Desktop CLI 缓存大小校验失败：源文件 {} bytes，缓存 {} bytes",
                source_metadata.len(),
                copied_metadata.len()
            ));
        }
        OpenOptions::new()
            .write(true)
            .open(&temporary)
            .and_then(|file| file.sync_all())
            .map_err(|err| {
                format!(
                    "同步 Codex Desktop CLI 缓存失败 {}: {err}",
                    temporary.display()
                )
            })?;
        fs::set_permissions(&temporary, source_metadata.permissions()).map_err(|err| {
            format!(
                "设置 Codex Desktop CLI 缓存权限失败 {}: {err}",
                temporary.display()
            )
        })?;

        if cached_file_matches(&target, source_metadata.len()) {
            return Ok(());
        }
        if target.exists() {
            fs::remove_file(&target).map_err(|err| {
                format!(
                    "替换旧 Codex Desktop CLI 缓存失败 {}: {err}",
                    target.display()
                )
            })?;
        }
        match fs::rename(&temporary, &target) {
            Ok(()) => Ok(()),
            Err(_err) if cached_file_matches(&target, source_metadata.len()) => Ok(()),
            Err(err) => Err(format!(
                "启用 Codex Desktop CLI 缓存失败 {} -> {}: {err}",
                temporary.display(),
                target.display()
            )),
        }
    })();

    if copied_result.is_err() || temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    copied_result?;
    cleanup_stale_cached_clis(cache_dir, &target);
    Ok(target)
}

fn source_fingerprint(source: &Path, metadata: &fs::Metadata) -> u64 {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    stable_hash(&format!(
        "{}|{}|{modified}",
        source.to_string_lossy(),
        metadata.len()
    ))
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(windows)]
fn cache_file_name(fingerprint: u64) -> String {
    format!("{CACHED_CLI_FILE_PREFIX}{fingerprint:016x}.exe")
}

#[cfg(not(windows))]
fn cache_file_name(fingerprint: u64) -> String {
    format!("{CACHED_CLI_FILE_PREFIX}{fingerprint:016x}")
}

fn cached_file_matches(path: &Path, expected_len: u64) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == expected_len)
}

fn cleanup_stale_cached_clis(cache_dir: &Path, current: &Path) {
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == current || !path.is_file() {
            continue;
        }
        let is_cached_cli = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(CACHED_CLI_FILE_PREFIX));
        if is_cached_cli {
            let _ = fs::remove_file(path);
        }
    }
}

impl AppServerClient {
    fn start(cli_path: &Path, root: &Path) -> Result<Self, String> {
        let mut command = Command::new(cli_path);
        command
            .args(["app-server", "--stdio"])
            .env("CODEX_HOME", root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_command_window(&mut command);

        let mut child = command.spawn().map_err(|err| {
            format!(
                "启动 Codex Desktop app-server 失败 {}: {err}",
                cli_path.display()
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Codex Desktop app-server stdin 不可用".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Codex Desktop app-server stdout 不可用".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Codex Desktop app-server stderr 不可用".to_string())?;

        let (stdout_sender, stdout_receiver) = mpsc::channel();
        let stdout_thread = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = stdout_sender.send(StdoutEvent::Closed);
                        break;
                    }
                    Ok(_) => {
                        while line.ends_with(['\r', '\n']) {
                            line.pop();
                        }
                        if stdout_sender.send(StdoutEvent::Line(line)).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = stdout_sender.send(StdoutEvent::Error(err.to_string()));
                        break;
                    }
                }
            }
        });

        let stderr_buffer = Arc::new(Mutex::new(String::new()));
        let stderr_for_thread = Arc::clone(&stderr_buffer);
        let stderr_thread = thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Ok(mut output) = stderr_for_thread.lock() {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                }
            }
        });

        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout_receiver,
            stderr: stderr_buffer,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
            next_request_id: 1,
        })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        let request = json!({
            "id": request_id,
            "method": method,
            "params": params
        });
        self.write_message(&request).map_err(|err| {
            self.error_with_stderr(format!(
                "向 Codex Desktop app-server 发送 {method} 请求失败: {err}"
            ))
        })?;
        self.wait_for_response(request_id, method)
    }

    fn write_message(&mut self, message: &Value) -> Result<(), String> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| "stdin 已关闭".to_string())?;
        serde_json::to_writer(&mut *stdin, message)
            .map_err(|err| format!("序列化 JSON-RPC 请求失败: {err}"))?;
        stdin
            .write_all(b"\n")
            .and_then(|_| stdin.flush())
            .map_err(|err| err.to_string())
    }

    fn wait_for_response(&mut self, request_id: u64, method: &str) -> Result<Value, String> {
        let deadline = Instant::now() + RPC_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.error_with_stderr(format!(
                    "Codex Desktop app-server {method} 请求超时（{} 秒）",
                    RPC_TIMEOUT.as_secs()
                )));
            }
            let event = match self.stdout_receiver.recv_timeout(remaining) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(self.error_with_stderr(format!(
                        "Codex Desktop app-server {method} 请求超时（{} 秒）",
                        RPC_TIMEOUT.as_secs()
                    )));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.error_with_stderr(format!(
                        "Codex Desktop app-server 在 {method} 响应前关闭了 stdout"
                    )));
                }
            };

            let line = match event {
                StdoutEvent::Line(line) if line.trim().is_empty() => continue,
                StdoutEvent::Line(line) => line,
                StdoutEvent::Error(err) => {
                    return Err(self.error_with_stderr(format!(
                        "读取 Codex Desktop app-server {method} 响应失败: {err}"
                    )));
                }
                StdoutEvent::Closed => {
                    let status = self
                        .child
                        .try_wait()
                        .ok()
                        .flatten()
                        .map(|status| status.to_string())
                        .unwrap_or_else(|| "未知状态".to_string());
                    return Err(self.error_with_stderr(format!(
                        "Codex Desktop app-server 在 {method} 响应前退出（{status}）"
                    )));
                }
            };

            let message: Value = serde_json::from_str(&line).map_err(|err| {
                self.error_with_stderr(format!(
                    "解析 Codex Desktop app-server {method} 响应失败: {err}；输出: {line}"
                ))
            })?;
            if message.get("method").is_some() && message.get("id").is_some() {
                self.reject_server_request(&message)?;
                continue;
            }
            if !rpc_id_matches(message.get("id"), request_id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(self.error_with_stderr(format!(
                    "Codex Desktop app-server {method} 请求失败: {}",
                    format_rpc_error(error)
                )));
            }
            return message.get("result").cloned().ok_or_else(|| {
                self.error_with_stderr(format!("Codex Desktop app-server {method} 响应缺少 result"))
            });
        }
    }

    fn reject_server_request(&mut self, message: &Value) -> Result<(), String> {
        let Some(id) = message.get("id").cloned() else {
            return Ok(());
        };
        self.write_message(&json!({
            "id": id,
            "error": {
                "code": -32601,
                "message": "Codex Switch session listing does not support server requests"
            }
        }))
        .map_err(|err| {
            self.error_with_stderr(format!(
                "拒绝 Codex Desktop app-server 的未支持请求失败: {err}"
            ))
        })
    }

    fn error_with_stderr(&self, message: String) -> String {
        let stderr = self
            .stderr
            .lock()
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        if stderr.is_empty() {
            message
        } else {
            format!("{message}\napp-server stderr: {stderr}")
        }
    }
}

impl Drop for AppServerClient {
    fn drop(&mut self) {
        self.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if let Some(thread) = self.stdout_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
}

fn rpc_id_matches(value: Option<&Value>, expected: u64) -> bool {
    value.is_some_and(|value| {
        value.as_u64() == Some(expected)
            || value.as_str().and_then(|value| value.parse::<u64>().ok()) == Some(expected)
    })
}

fn format_rpc_error(error: &Value) -> String {
    if let Some(message) = error.get("message").and_then(Value::as_str) {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .map(|code| format!("code {code}, "))
            .unwrap_or_default();
        let data = error
            .get("data")
            .filter(|value| !value.is_null())
            .map(|value| format!("; data={value}"))
            .unwrap_or_default();
        return format!("{code}{message}{data}");
    }
    error.to_string()
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    let left = normalized_path(left);
    let right = normalized_path(right);
    if cfg!(windows) {
        left.eq_ignore_ascii_case(&right)
    } else {
        left == right
    }
}

fn path_is_within(root: &Path, path: &Path) -> bool {
    let root = normalized_path(root);
    let path = normalized_path(path);
    let (root, path) = if cfg!(windows) {
        (root.to_ascii_lowercase(), path.to_ascii_lowercase())
    } else {
        (root, path)
    };
    path == root || path.starts_with(&format!("{root}/"))
}

fn normalized_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    let value = if cfg!(windows) {
        if let Some(path) = value.strip_prefix("//?/UNC/") {
            format!("//{path}")
        } else if let Some(path) = value.strip_prefix("//?/") {
            path.to_string()
        } else {
            value
        }
    } else {
        value
    };
    value.trim_end_matches('/').to_string()
}

#[cfg(windows)]
fn hide_command_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}

#[cfg(not(windows))]
fn hide_command_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "codex-switch-app-server-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    #[test]
    fn thread_list_params_match_codex_desktop_filters() {
        let value = thread_list_params(true, Some("cursor-1"));

        assert_eq!(value["archived"], true);
        assert_eq!(value["cursor"], "cursor-1");
        assert_eq!(value["limit"], THREAD_LIST_PAGE_SIZE);
        assert_eq!(value["sourceKinds"], json!([]));
        assert_eq!(value["modelProviders"], json!([]));
        assert_eq!(value["sortKey"], "recency_at");
        assert_eq!(value["useStateDbOnly"], false);
    }

    #[test]
    fn parses_thread_list_page_and_skips_ephemeral_without_path() {
        let root = test_root("parse");
        let path = root.join("sessions/2026/07/13/rollout.jsonl");
        let result = json!({
            "data": [
                {
                    "id": "thread-1",
                    "name": "Named thread",
                    "preview": "",
                    "cwd": root.join("workspace"),
                    "path": path,
                    "updatedAt": 20,
                    "recencyAt": 21,
                    "ephemeral": false
                },
                {
                    "id": "ephemeral-1",
                    "name": null,
                    "preview": "Temporary",
                    "cwd": root.join("workspace"),
                    "path": null,
                    "updatedAt": 30,
                    "recencyAt": null,
                    "ephemeral": true
                }
            ],
            "nextCursor": "next-1"
        });

        let page = parse_thread_list_page(&root, false, result).unwrap();

        assert_eq!(page.next_cursor.as_deref(), Some("next-1"));
        assert_eq!(page.threads.len(), 1);
        assert_eq!(page.threads[0].id, "thread-1");
        assert_eq!(page.threads[0].name.as_deref(), Some("Named thread"));
        assert_eq!(page.threads[0].preview, "");
        assert_eq!(page.threads[0].path, path);
        assert_eq!(page.threads[0].updated_at, 20);
        assert_eq!(page.threads[0].recency_at, Some(21));
        assert!(!page.threads[0].archived);
    }

    #[test]
    fn rejects_non_ephemeral_thread_without_path() {
        let root = test_root("missing-path");
        let result = json!({
            "data": [{
                "id": "thread-1",
                "name": null,
                "preview": "Preview",
                "cwd": root.join("workspace"),
                "path": null,
                "updatedAt": 20,
                "recencyAt": null,
                "ephemeral": false
            }],
            "nextCursor": null
        });

        let error = parse_thread_list_page(&root, true, result).unwrap_err();

        assert!(error.contains("非 ephemeral"));
        assert!(error.contains("thread-1"));
    }

    #[test]
    fn rejects_thread_path_outside_codex_home() {
        let root = test_root("outside-root");
        let outside = test_root("other-root").join("rollout.jsonl");
        let result = json!({
            "data": [{
                "id": "thread-1",
                "name": null,
                "preview": "Preview",
                "cwd": root.join("workspace"),
                "path": outside,
                "updatedAt": 20,
                "recencyAt": null,
                "ephemeral": false
            }],
            "nextCursor": null
        });

        let error = parse_thread_list_page(&root, false, result).unwrap_err();

        assert!(error.contains("不属于当前 CODEX_HOME"));
    }

    #[test]
    fn caches_desktop_cli_with_versioned_file_name() {
        let root = test_root("cache");
        let source = root.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        let cache_dir = root.join("cache");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(&source, b"desktop codex cli").unwrap();
        let stale = cache_dir.join(cache_file_name(1));
        fs::write(&stale, b"old desktop codex cli").unwrap();

        let first = cache_cli_from_source(&source, &cache_dir).unwrap();
        let second = cache_cli_from_source(&source, &cache_dir).unwrap();

        assert_eq!(first, second);
        assert_eq!(fs::read(&first).unwrap(), b"desktop codex cli");
        assert!(first.starts_with(&cache_dir));
        assert!(!stale.exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rpc_error_includes_code_message_and_data() {
        let error = json!({
            "code": -32602,
            "message": "invalid params",
            "data": { "field": "archived" }
        });

        let formatted = format_rpc_error(&error);

        assert!(formatted.contains("code -32602"));
        assert!(formatted.contains("invalid params"));
        assert!(formatted.contains("archived"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_extended_paths_compare_with_regular_paths() {
        let root = PathBuf::from(r"C:\")
            .join("Users")
            .join("Fixture")
            .join(".codex");
        let extended_root = PathBuf::from(r"\\?\C:\")
            .join("Users")
            .join("Fixture")
            .join(".codex");
        let extended_thread = extended_root.join("sessions/2026/07/13/rollout.jsonl");

        assert!(paths_equal(&root, &extended_root));
        assert!(path_is_within(&root, &extended_thread));
    }
}
