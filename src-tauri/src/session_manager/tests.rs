use super::{
    backup::{sanitize_backup_reason, session_manager_data_dir},
    catalog::*,
    codex_home::*,
    delete::*,
    legacy_migration::*,
    model::*,
    preview::*,
    rollout::*,
    state_db::*,
    status::*,
    transfer::*,
    util::*,
    zip::*,
};
use crate::codex_app_server::{CodexDesktopThread, ThreadDeleteBatch};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::env;
use std::io::Write;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_path(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!("codex-switch-session-manager-{name}-{stamp}"))
}

fn sample_thread_metadata(rollout_path: PathBuf) -> ThreadMetadata {
    ThreadMetadata {
        id: "thread-1".to_string(),
        rollout_path,
        created_at: 1_700_000_000,
        updated_at: 1_700_000_120,
        source: "codex".to_string(),
        model_provider: "openai".to_string(),
        cwd: "C:\\work".to_string(),
        title: "Imported thread".to_string(),
        sandbox_policy: "workspace-write".to_string(),
        approval_mode: "on-request".to_string(),
        has_user_event: 1,
        archived: 0,
        archived_at: None,
        cli_version: "0.144.1".to_string(),
        first_user_message: "hello".to_string(),
        agent_nickname: None,
        agent_role: None,
        model: Some("gpt-5.2".to_string()),
        reasoning_effort: Some("high".to_string()),
        agent_path: None,
        thread_source: Some("user".to_string()),
        preview: "hello".to_string(),
        history_mode: "legacy".to_string(),
        parent_thread_id: None,
        dynamic_tools: Vec::new(),
    }
}

fn create_current_state_db(root: &Path) -> Connection {
    fs::create_dir_all(root).unwrap();
    let connection = Connection::open(root.join("state_5.sqlite")).unwrap();
    connection
        .execute_batch(
            r#"
                CREATE TABLE _sqlx_migrations (
                    version INTEGER PRIMARY KEY,
                    success INTEGER NOT NULL
                );
                INSERT INTO _sqlx_migrations (version, success) VALUES (40, 1);
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    title TEXT NOT NULL DEFAULT '',
                    cwd TEXT NOT NULL DEFAULT '',
                    archived INTEGER NOT NULL DEFAULT 0,
                    archived_at INTEGER,
                    updated_at INTEGER NOT NULL DEFAULT 0,
                    updated_at_ms INTEGER NOT NULL DEFAULT 0,
                    preview TEXT NOT NULL DEFAULT '',
                    recency_at INTEGER NOT NULL DEFAULT 0,
                    recency_at_ms INTEGER NOT NULL DEFAULT 0,
                    history_mode TEXT NOT NULL DEFAULT 'legacy'
                );
                "#,
        )
        .unwrap();
    connection
}

fn write_test_session(root: &Path, relative: &Path, id: &str, label: &str) -> PathBuf {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, test_session_content(id, label)).unwrap();
    path
}

fn write_compressed_test_session(root: &Path, relative: &Path, id: &str, label: &str) -> PathBuf {
    let path = compressed_rollout_path(&root.join(relative));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes = zstd::encode_all(test_session_content(id, label).as_bytes(), 0).unwrap();
    fs::write(&path, bytes).unwrap();
    path
}

fn test_session_content(id: &str, label: &str) -> String {
    format!(
        "{}\n{}\n{}\n",
        json!({
            "timestamp": "2026-07-13T00:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": id,
                "cwd": "C:\\work",
                "originator": "codex-switch-test"
            }
        }),
        json!({
            "timestamp": "2026-07-13T00:00:01Z",
            "type": "event_msg",
            "payload": {"type": "user_message", "message": label}
        }),
        json!({
            "timestamp": "2026-07-13T00:00:02Z",
            "type": "event_msg",
            "payload": {"type": "agent_message", "message": format!("reply-{label}")}
        })
    )
}

#[test]
fn permanent_delete_reports_rpc_failures_and_cleans_only_successes() {
    let root = temp_path("permanent-delete-report");
    fs::create_dir_all(&root).unwrap();
    let state = root.join(".codex-global-state.json");
    fs::write(
        &state,
        r#"{"pinned-thread-ids":["local:deleted-id","refused-id"],"threads":{"deleted-id":true,"refused-id":true}}"#,
    )
    .unwrap();

    let result = finish_delete(
        &root,
        ThreadDeleteBatch {
            deleted: 1,
            removed_ids: HashSet::from(["deleted-id".to_string()]),
            errors: vec![
                "会话 refused-id: code -32600: forked history still references it".to_string(),
            ],
        },
    );

    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["report"]["deleted"], 1);
    assert_eq!(result["report"]["failed"], 1);
    assert!(result["message"].as_str().unwrap().contains("code -32600"));
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(state).unwrap()).unwrap(),
        json!({ "pinned-thread-ids": ["refused-id"], "threads": { "refused-id": true } })
    );
    assert!(!session_manager_data_dir()
        .unwrap()
        .join("deleted-sessions")
        .exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
#[ignore = "starts the installed Codex Desktop app-server; run with isolated HOME and APPDATA"]
fn permanent_delete_uses_official_app_server_in_isolated_home() {
    let root = temp_path("permanent-delete-official-rpc");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let stamp = "2026-11-17T00:00:00Z";
    let write_rollout = |id: &str,
                         label: &str,
                         archived: bool,
                         paginated: bool,
                         base: Option<(&str, u64)>,
                         parent: Option<&str>| {
        let folder = root.join(if archived {
            "archived_sessions"
        } else {
            "sessions/2026/11/17"
        });
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join(format!("rollout-2026-11-17T00-00-00-{id}.jsonl"));
        let mut meta = json!({
            "session_id": id, "id": id, "timestamp": stamp, "cwd": root,
            "originator": "codex_fixture", "cli_version": "0.169.0", "source": "vscode",
            "model_provider": "openai", "history_mode": if paginated { "paginated" } else { "legacy" }
        });
        if let Some((base_id, end)) = base {
            meta["history_base"] = json!({
                "thread_id": base_id, "end_ordinal_exclusive": 2, "end_byte_offset": end
            });
            meta["forked_from_id"] = json!(base_id);
            meta["forked_from_ordinal_exclusive"] = json!(2);
        }
        if let Some(parent) = parent {
            meta["source"] = json!({ "subagent": { "thread_spawn": {
                "parent_thread_id": parent, "depth": 1, "agent_path": "/root/test-fixture",
                "agent_nickname": "Fixture", "agent_role": null
            }}});
        }
        let mut first = json!({ "timestamp": stamp, "type": "session_meta", "payload": meta });
        let mut second = json!({
            "timestamp": stamp, "type": "event_msg",
            "payload": { "type": "user_message", "message": label, "kind": "plain" }
        });
        if paginated {
            first["ordinal"] = json!(0);
            second["ordinal"] = json!(1);
        }
        fs::write(&path, format!("{first}\n{second}\n")).unwrap();
        path
    };
    let [active, archive, base, fork, keep, child, child_unselected] =
        [1, 2, 3, 4, 5, 6, 7].map(|n| format!("019f0000-0000-7000-8000-{n:012}"));
    let active_path = write_rollout(&active, "active", false, false, None, None);
    let archive_path = write_rollout(&archive, "archived", true, false, None, None);
    let base_path = write_rollout(&base, "fork source", false, true, None, None);
    let fork_path = write_rollout(
        &fork,
        "fork",
        false,
        true,
        Some((&base, base_path.metadata().unwrap().len())),
        None,
    );
    let keep_path = write_rollout(&keep, "unrelated", false, false, None, None);
    let child_path = write_rollout(&child, "selected child", false, false, None, Some(&active));
    let child_unselected_path = write_rollout(
        &child_unselected,
        "unselected child",
        false,
        false,
        None,
        Some(&active),
    );
    let index = root.join("session_index.jsonl");
    let ids = [
        &active,
        &archive,
        &base,
        &fork,
        &keep,
        &child,
        &child_unselected,
    ];
    fs::write(
        &index,
        ids.iter()
            .map(|id| {
                format!(
                    "{}\n",
                    json!({"id": id, "thread_name": id, "updated_at": stamp})
                )
            })
            .collect::<String>(),
    )
    .unwrap();
    let global = root.join(".codex-global-state.json");
    fs::write(
        &global,
        json!({ "pinned-thread-ids": ids, "unrelated": "keep" }).to_string(),
    )
    .unwrap();

    let root_path = root.to_string_lossy().into_owned();
    let before = scan_conversations_impl(Some(root_path.clone())).unwrap();
    assert_eq!(
        before["conversations"].as_array().unwrap().len(),
        7,
        "{before}"
    );
    {
        let connection = Connection::open(root.join("state_5.sqlite")).unwrap();
        for child in [&child, &child_unselected] {
            connection.execute(
                "INSERT INTO thread_spawn_edges(parent_thread_id,child_thread_id,status) VALUES (?1,?2,'closed')",
                params![active, child],
            ).unwrap();
        }
    }
    let result = delete_conversations_impl(
        root_path.clone(),
        vec![active.clone(), child, base.clone(), archive.clone(), active],
    )
    .unwrap();
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["report"]["deleted"], 3);
    assert_eq!(result["report"]["failed"], 1);
    let error = result["report"]["errors"][0].as_str().unwrap();
    assert!(
        error.contains("-32600") && error.contains("forked history"),
        "{error}"
    );
    assert!(!active_path.exists() && !archive_path.exists());
    assert!(!child_path.exists() && !child_unselected_path.exists());
    assert!(base_path.exists() && fork_path.exists() && keep_path.exists());
    let index_ids = read_session_index(&root, &mut Vec::new());
    let after = scan_conversations_impl(Some(root_path)).unwrap();
    let survivors: HashSet<&str> = [&base, &fork, &keep]
        .into_iter()
        .map(String::as_str)
        .collect();
    {
        let connection = Connection::open(root.join("state_5.sqlite")).unwrap();
        let mut query = connection.prepare("SELECT id FROM threads").unwrap();
        let stored_ids = query
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<HashSet<_>, _>>()
            .unwrap();
        assert_eq!(
            stored_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>(),
            survivors
        );
    }
    assert_eq!(
        after["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<HashSet<_>>(),
        survivors,
    );
    assert_eq!(index_ids.len(), 6); // each surviving ID has a bare and a local variant
    assert!(survivors.iter().all(|id| index_ids.contains_key(*id)));
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(global).unwrap()).unwrap(),
        json!({ "pinned-thread-ids": [base, fork, keep], "unrelated": "keep" }),
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn relative_path_rejects_traversal() {
    assert!(normalize_relative_path("../sessions/a.jsonl").is_err());
    assert!(normalize_relative_path("sessions/2026/05/01/a.jsonl").is_ok());
}

#[cfg(windows)]
#[test]
fn path_identity_normalizes_windows_extended_prefix_and_case() {
    assert_eq!(
        normalized_path_identity(Path::new(r"\\?\C:\Profiles\Example\.codex")),
        normalized_path_identity(Path::new(r"c:\profiles\example\.codex"))
    );
    assert_eq!(
        normalized_path_identity(Path::new(r"\\?\UNC\server\share\folder")),
        normalized_path_identity(Path::new(r"\\server\share\folder"))
    );
}

#[test]
fn parser_prefers_event_messages_over_response_items() {
    let path = temp_path("parse.jsonl");
    fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-05-01T00:00:00Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"fallback\"}]}}\n",
                "{\"timestamp\":\"2026-05-01T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"hello\"}}\n",
                "{\"timestamp\":\"2026-05-01T00:00:02Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"world\"}}\n"
            ),
        )
        .unwrap();

    let summary = parse_session_file(&path, true).unwrap();
    fs::remove_file(&path).unwrap();

    assert_eq!(summary.messages.len(), 2);
    assert_eq!(summary.messages[0].role, "user");
    assert_eq!(summary.messages[0].text, "hello");
    assert_eq!(summary.messages[1].role, "assistant");
}

#[test]
fn parser_preserves_current_thread_metadata_and_dynamic_tools() {
    let path = temp_path("parse-current-metadata.jsonl");
    fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-07-10T08:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"child-thread\",\"cwd\":\"C:\\\\work\",\"model_provider\":\"openai\",\"cli_version\":\"0.144.1\",\"source\":{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"parent-thread\"}}},\"thread_source\":\"subagent\",\"parent_thread_id\":\"parent-thread\",\"forked_from_id\":\"parent-thread\",\"agent_nickname\":\"Curie\",\"agent_role\":\"explorer\",\"agent_path\":\"/root/audit\",\"history_mode\":\"legacy\",\"dynamic_tools\":[{\"type\":\"namespace\",\"name\":\"codex_app\",\"tools\":[{\"type\":\"function\",\"name\":\"read_thread\",\"description\":\"Read a thread\",\"inputSchema\":{\"type\":\"object\"},\"deferLoading\":true}]}]}}\n",
                "{\"timestamp\":\"2026-07-10T08:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"hello current schema\"}}\n"
            ),
        )
        .unwrap();

    let summary = parse_session_file(&path, true).unwrap();
    fs::remove_file(&path).unwrap();

    assert_eq!(summary.thread_source.as_deref(), Some("subagent"));
    assert_eq!(summary.parent_thread_id.as_deref(), Some("parent-thread"));
    assert_eq!(summary.agent_nickname.as_deref(), Some("Curie"));
    assert_eq!(summary.agent_role.as_deref(), Some("explorer"));
    assert_eq!(summary.agent_path.as_deref(), Some("/root/audit"));
    assert_eq!(summary.history_mode.as_deref(), Some("legacy"));
    assert_eq!(summary.preview.as_deref(), Some("hello current schema"));
    assert_eq!(summary.dynamic_tools.len(), 1);
    assert_eq!(summary.dynamic_tools[0].name, "read_thread");
    assert_eq!(
        summary.dynamic_tools[0].namespace.as_deref(),
        Some("codex_app")
    );
    assert!(summary.dynamic_tools[0].defer_loading);
    assert!(summary
        .source
        .as_deref()
        .is_some_and(|source| source.contains("parent-thread")));
}

#[test]
fn preview_reads_recent_messages_in_pages_without_duplicates() {
    let path = temp_path("preview-pages.jsonl");
    let content = (0..10)
        .map(|index| {
            json!({
                "timestamp": format!("2026-07-10T08:00:{index:02}Z"),
                "type": "event_msg",
                "payload": {
                    "type": if index % 2 == 0 { "user_message" } else { "agent_message" },
                    "message": format!("message-{index}")
                }
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, format!("{content}\n")).unwrap();

    let latest = read_preview_message_page(&path, None, None, Some(3), None, None).unwrap();
    assert_eq!(
        latest
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>(),
        vec!["message-7", "message-8", "message-9"]
    );
    assert!(latest.has_more);
    assert_eq!(latest.source, PreviewMessageSource::Event);

    let earlier = read_preview_message_page(
        &path,
        latest.next_before,
        Some(latest.file_size),
        Some(3),
        Some(latest.source.as_str()),
        None,
    )
    .unwrap();
    assert_eq!(
        earlier
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>(),
        vec!["message-4", "message-5", "message-6"]
    );
    let latest_offsets = latest
        .messages
        .iter()
        .filter_map(|message| message.offset)
        .collect::<HashSet<_>>();
    assert!(earlier
        .messages
        .iter()
        .filter_map(|message| message.offset)
        .all(|offset| !latest_offsets.contains(&offset)));

    fs::remove_file(path).unwrap();
}

#[test]
fn preview_uses_response_items_when_event_messages_are_absent() {
    let path = temp_path("preview-response-fallback.jsonl");
    fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-07-10T08:00:00Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"text\":\"fallback-user\"}]}}\n",
                "{\"timestamp\":\"2026-07-10T08:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"text\":\"fallback-assistant\"}]}}\n"
            ),
        )
        .unwrap();

    let page = read_preview_message_page(&path, None, None, Some(10), None, None).unwrap();
    assert_eq!(page.source, PreviewMessageSource::Response);
    assert_eq!(page.messages.len(), 2);
    assert_eq!(page.messages[0].text, "fallback-user");
    assert_eq!(page.messages[1].text, "fallback-assistant");

    fs::remove_file(path).unwrap();
}

#[test]
fn compressed_rollout_preview_matches_plain_message_pages() {
    let root = temp_path("compressed-preview-pages");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("rollout-preview.jsonl");
    let compressed = compressed_rollout_path(&path);
    let compare = |content: &[u8], source: PreviewMessageSource| {
        fs::write(&path, content).unwrap();
        fs::write(&compressed, zstd::encode_all(content, 0).unwrap()).unwrap();
        let read = |path: &Path| {
            let latest = read_preview_message_page(path, None, None, Some(2), None, None).unwrap();
            assert_eq!(latest.source, source);
            assert!(latest.has_more);
            let earlier = read_preview_message_page(
                path,
                latest.next_before,
                Some(latest.file_size),
                Some(2),
                Some(latest.source.as_str()),
                None,
            )
            .unwrap();
            assert!(!earlier.has_more);
            let stale = read_preview_message_page(
                path,
                None,
                Some(latest.file_size + 1),
                Some(2),
                None,
                None,
            )
            .unwrap_err();
            assert!(stale.contains("会话文件已变化"));
            (
                latest.next_before,
                json!([earlier.messages, latest.messages]),
            )
        };
        assert_eq!(read(&path), read(&compressed));
    };
    let events = (0..4).map(|index| {
        format!(
            "{}\n",
            json!({"type": "event_msg", "payload": {"type": "user_message", "message": format!("event {index}")}})
        )
    }).collect::<String>();
    let responses = (0..4).map(|index| {
        format!(
            "{}\n",
            json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"text": format!("response {index}")}]}})
        )
    }).collect::<String>();
    compare(
        responses.trim_end().as_bytes(),
        PreviewMessageSource::Response,
    );
    // Event messages are preferred even when response items also appear.
    compare(
        format!("{responses}{events}").as_bytes(),
        PreviewMessageSource::Event,
    );

    fs::write(&compressed, b"invalid zstd").unwrap();
    let error =
        read_preview_message_page(&compressed, None, None, Some(2), None, None).unwrap_err();
    assert!(error.contains("压缩会话文件"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn preview_snapshot_allows_append_and_rejects_truncate() {
    let path = temp_path("preview-snapshot.jsonl");
    let initial = (0..5)
        .map(|index| {
            json!({
                "timestamp": format!("2026-07-10T08:00:{index:02}Z"),
                "type": "event_msg",
                "payload": {"type": "user_message", "message": format!("m{index}")}
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, format!("{initial}\n")).unwrap();
    let latest = read_preview_message_page(&path, None, None, Some(2), None, None).unwrap();

    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-07-10T08:01:00Z",
            "type": "event_msg",
            "payload": {"type": "user_message", "message": "appended"}
        })
    )
    .unwrap();
    drop(file);
    let earlier = read_preview_message_page(
        &path,
        latest.next_before,
        Some(latest.file_size),
        Some(2),
        Some(latest.source.as_str()),
        None,
    )
    .unwrap();
    assert_eq!(earlier.file_size, latest.file_size);
    assert!(earlier
        .messages
        .iter()
        .all(|message| message.text != "appended"));

    fs::write(&path, "{}\n").unwrap();
    let stale = read_preview_message_page(
        &path,
        latest.next_before,
        Some(latest.file_size),
        Some(2),
        Some(latest.source.as_str()),
        None,
    )
    .unwrap_err();
    assert!(stale.contains("会话文件已变化"));

    fs::remove_file(path).unwrap();
}

#[test]
fn zip_store_round_trip_reads_entries() {
    let path = temp_path("archive.zip");
    write_zip_store(
        &path,
        &[
            ("manifest.json".to_string(), br#"{"ok":true}"#.to_vec()),
            (
                "sessions/2026/05/01/rollout-test.jsonl".to_string(),
                b"{}\n".to_vec(),
            ),
        ],
    )
    .unwrap();

    let archive = ZipArchiveLite::open(&path).unwrap();
    let manifest = archive.read_entry("manifest.json").unwrap();
    let session = archive
        .read_entry("sessions/2026/05/01/rollout-test.jsonl")
        .unwrap();
    fs::remove_file(&path).unwrap();

    assert_eq!(manifest, br#"{"ok":true}"#);
    assert_eq!(session, b"{}\n");
}

#[test]
fn status_change_updates_current_state_without_rewriting_session_index() {
    let root = temp_path("status-current-state");
    let session_id = "019e20f9-34b7-7a82-a95b-fe461de8983a";
    let file_name = "rollout-2026-05-13T18-54-23-019e20f9-34b7-7a82-a95b-fe461de8983a.jsonl";
    let active_relative = PathBuf::from("sessions")
        .join("2026")
        .join("05")
        .join("13")
        .join(file_name);
    let active_path = root.join(&active_relative);
    fs::create_dir_all(active_path.parent().unwrap()).unwrap();
    fs::write(
        &active_path,
        format!(
            "{}\n{}\n",
            json!({
                "timestamp": "2026-05-13T10:54:26.757Z",
                "type": "session_meta",
                "payload": {
                    "id": session_id,
                    "cwd": "C:\\Users\\yuhon\\Documents\\Codex\\hello"
                }
            }),
            json!({
                "timestamp": "2026-05-13T10:54:27.000Z",
                "type": "event_msg",
                "payload": {
                    "type": "thread_name_updated",
                    "thread_name": "测试归档索引"
                }
            })
        ),
    )
    .unwrap();
    let connection = create_current_state_db(&root);
    connection
            .execute(
                "INSERT INTO threads
                 (id, rollout_path, title, updated_at, updated_at_ms, preview, recency_at, recency_at_ms)
                 VALUES (?1, ?2, '新版标题', 1, 1000, 'preview', 1, 1000)",
                params![session_id, active_path.to_string_lossy()],
            )
            .unwrap();
    drop(connection);
    fs::write(
        root.join("session_index.jsonl"),
        format!(
            "{}\n",
            json!({
                "id": session_id,
                "thread_name": "测试归档索引",
                "updated_at": "2026-05-13T10:54:27.000Z"
            })
        ),
    )
    .unwrap();

    set_conversation_status_impl(
        root.to_string_lossy().to_string(),
        vec![path_to_slash(&active_relative)],
        "archived".to_string(),
        None,
    )
    .unwrap();
    let archived_relative = PathBuf::from("archived_sessions").join(file_name);
    let archived_path = root.join(&archived_relative);
    assert!(archived_path.exists());
    let archived_row: (i64, String) = Connection::open(root.join("state_5.sqlite"))
        .unwrap()
        .query_row(
            "SELECT archived, rollout_path FROM threads WHERE id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(archived_row.0, 1);
    assert_eq!(
        conversation_path_key(Path::new(&archived_row.1)),
        conversation_path_key(&archived_path)
    );

    set_conversation_status_impl(
        root.to_string_lossy().to_string(),
        vec![path_to_slash(&archived_relative)],
        "active".to_string(),
        None,
    )
    .unwrap();
    let active_row: (i64, String) = Connection::open(root.join("state_5.sqlite"))
        .unwrap()
        .query_row(
            "SELECT archived, rollout_path FROM threads WHERE id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let session_index = fs::read_to_string(root.join("session_index.jsonl")).unwrap();

    fs::remove_dir_all(&root).unwrap();

    assert_eq!(active_row.0, 0);
    assert_eq!(
        conversation_path_key(Path::new(&active_row.1)),
        conversation_path_key(&active_path)
    );
    assert!(session_index.contains(session_id));
    assert!(session_index.contains("测试归档索引"));
}

#[test]
fn scan_catalog_uses_only_codex_desktop_thread_list() {
    let root = temp_path("scan-desktop-thread-list");
    let session_id = "019e20f9-34b7-7a82-a95b-fe461de8983a";
    let file_name = "rollout-2026-05-13T18-54-23-019e20f9-34b7-7a82-a95b-fe461de8983a.jsonl";
    let archived_path = root.join("archived_sessions").join(file_name);
    fs::create_dir_all(archived_path.parent().unwrap()).unwrap();
    fs::write(&archived_path, b"{}\n").unwrap();
    let unreturned_path = root.join("sessions").join("subagent.jsonl");
    fs::create_dir_all(unreturned_path.parent().unwrap()).unwrap();
    fs::write(&unreturned_path, b"{}\n").unwrap();

    let desktop_threads = vec![CodexDesktopThread {
        id: session_id.to_string(),
        name: Some("Codex Desktop 标题".to_string()),
        preview: "Desktop 预览".to_string(),
        cwd: root.join("workspace"),
        path: archived_path.clone(),
        updated_at: 10,
        recency_at: Some(20),
        archived: true,
    }];
    let (conversations, warnings, errors) =
        conversations_from_desktop_threads(&root, desktop_threads);

    fs::remove_dir_all(&root).unwrap();

    assert!(warnings.is_empty());
    assert!(errors.is_empty());
    assert_eq!(conversations.len(), 1);
    assert_eq!(conversations[0].id, session_id);
    assert_eq!(conversations[0].title, "Codex Desktop 标题");
    assert_eq!(conversations[0].preview.as_deref(), Some("Desktop 预览"));
    assert_eq!(conversations[0].status, "archived");
    assert_eq!(
        conversations[0].source_path,
        archived_path.to_string_lossy()
    );
}

#[test]
fn compressed_rollout_stays_in_catalog_with_its_logical_path() {
    let root = temp_path("compressed-list");
    let relative = Path::new("sessions/rollout-compressed.jsonl");
    let logical = root.join(relative);
    let file = compressed_rollout_path(&logical);
    fs::create_dir_all(logical.parent().unwrap()).unwrap();
    let bytes = zstd::encode_all(
        &br#"{"type":"event_msg","payload":{"type":"user_message","message":"compressed history"}}"#[..],
        0,
    ).unwrap();
    fs::write(&file, &bytes).unwrap();
    let thread = CodexDesktopThread {
        id: "compressed-thread".to_string(),
        name: Some("Compressed thread".to_string()),
        preview: "compressed history".to_string(),
        cwd: root.join("workspace"),
        path: logical.clone(),
        updated_at: 1,
        recency_at: Some(1),
        archived: false,
    };
    let mut duplicate = thread.clone();
    duplicate.id = "duplicate-thread".to_string();
    duplicate.path = file.clone();
    let mut missing = thread.clone();
    missing.id = "missing-thread".to_string();
    missing.path = logical.with_file_name("rollout-missing.jsonl");
    let (items, warnings, errors) =
        conversations_from_desktop_threads(&root, vec![thread, duplicate, missing]);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].relative_path, path_to_slash(relative));
    assert_eq!(items[0].source_path, file.to_string_lossy());
    assert_eq!(items[0].size_bytes, bytes.len() as u64);
    assert_eq!(warnings.len(), 1);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("missing-thread"));
    let db = create_current_state_db(&root);
    db.execute(
        "INSERT INTO threads (id, rollout_path, title) VALUES (?1, ?2, 'From state DB')",
        params!["compressed-thread", logical.to_string_lossy()],
    )
    .unwrap();
    drop(db);
    let preview = preview_conversation_impl(
        root.to_string_lossy().into_owned(),
        path_to_slash(relative),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(preview["conversation"]["title"], "From state DB");
    assert_eq!(preview["messages"][0]["text"], "compressed history");
    assert!(!logical.exists());
    assert_eq!(fs::read(&file).unwrap(), bytes);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "starts the installed Codex Desktop app-server; run with isolated HOME and APPDATA"]
fn app_server_lists_and_previews_compressed_rollouts_across_pages() {
    let fixture = temp_path("compressed-catalog-official-rpc");
    let root = fixture.join("codex");
    let active = root.join("sessions/2026/11/17");
    let archived = root.join("archived_sessions");
    let originals = fixture.join("originals");
    for dir in [&active, &archived, &originals] {
        fs::create_dir_all(dir).unwrap();
    }
    let root = root.canonicalize().unwrap();
    let mut files_to_compress = Vec::new();
    for i in 0..180 {
        let id = format!("019f0000-0000-7000-8000-{i:012}");
        let dir = if i < 160 { &active } else { &archived };
        let path = dir.join(format!("rollout-2026-11-17T00-00-00-{id}.jsonl"));
        let stamp = "2026-11-17T00:00:00Z";
        let meta = json!({
            "type": "session_meta", "timestamp": stamp,
            "payload": {
                "id": id, "timestamp": stamp, "cwd": root, "source": "vscode",
                "model_provider": "openai", "originator": "codex_fixture",
                "cli_version": "0.169.0", "history_mode": "legacy"
            }
        });
        let message = json!({
            "type": "event_msg", "timestamp": stamp,
            "payload": { "type": "user_message", "message": "fixture message", "kind": "plain" }
        });
        fs::write(&path, format!("{meta}\n{message}\n")).unwrap();
        if i >= 50 {
            files_to_compress.push(path);
        }
    }
    let scan = || scan_conversations_impl(Some(root.to_string_lossy().into_owned())).unwrap();
    assert_eq!(scan()["conversations"].as_array().unwrap().len(), 180);
    // Match Codex's storage: move the originals out of CODEX_HOME, leaving their logical
    // paths in the state DB while the bytes on disk are only in .jsonl.zst.
    for path in &files_to_compress {
        let bytes = fs::read(path).unwrap();
        fs::write(
            compressed_rollout_path(path),
            zstd::encode_all(bytes.as_slice(), 0).unwrap(),
        )
        .unwrap();
        fs::rename(path, originals.join(path.file_name().unwrap())).unwrap();
    }
    let response = scan();
    let conversations = response["conversations"].as_array().unwrap();
    let errors = response["errors"].as_array().unwrap();
    assert!(
        errors.is_empty(),
        "{} errors; first: {:?}",
        errors.len(),
        errors.first()
    );
    assert_eq!(conversations.len(), 180);
    assert_eq!(
        conversations
            .iter()
            .filter(|row| row["status"] == "archived")
            .count(),
        20
    );
    assert_eq!(
        conversations
            .iter()
            .filter(|row| row["source_path"].as_str().unwrap().ends_with(".jsonl.zst"))
            .count(),
        130
    );
    let item = conversations
        .iter()
        .find(|row| row["status"] == "archived")
        .unwrap();
    let preview = preview_conversation_impl(
        root.to_string_lossy().into_owned(),
        item["relative_path"].as_str().unwrap().to_string(),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(preview["messages"][0]["text"], "fixture message");
    fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn session_index_uses_latest_title_across_local_id_variants() {
    let root = temp_path("session-index-title");
    let session_id = "019e20f9-34b7-7a82-a95b-fe461de8983a";
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("session_index.jsonl"),
        format!(
            "{}\n{}\n{}\n",
            json!({
                "id": session_id,
                "thread_name": "初始标题",
                "updated_at": "2026-05-13T10:54:27.000Z"
            }),
            json!({
                "id": format!("local:{session_id}"),
                "thread_name": "Codex 最新标题",
                "updated_at": "2026-05-13T10:55:27.000Z"
            }),
            json!({
                "id": session_id,
                "thread_name": "",
                "updated_at": "2026-05-13T10:56:27.000Z"
            })
        ),
    )
    .unwrap();

    let mut warnings = Vec::new();
    let index = read_session_index(&root, &mut warnings);
    fs::remove_dir_all(&root).unwrap();

    assert!(warnings.is_empty());
    assert_eq!(
        session_index_title(&index, session_id).as_deref(),
        Some("Codex 最新标题")
    );
    assert_eq!(
        session_index_title(&index, &format!("local:{session_id}")).as_deref(),
        Some("Codex 最新标题")
    );
}

#[test]
fn preview_uses_codex_session_index_title() {
    let root = temp_path("preview-session-index-title");
    let session_id = "019e20f9-34b7-7a82-a95b-fe461de8983a";
    let relative_path = PathBuf::from("sessions").join("rollout-preview-title.jsonl");
    let path = root.join(&relative_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            "{}\n{}\n",
            json!({
                "timestamp": "2026-05-13T10:54:26.757Z",
                "type": "session_meta",
                "payload": {"id": session_id, "cwd": "C:\\work"}
            }),
            json!({
                "timestamp": "2026-05-13T10:54:27.000Z",
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "原始长消息"}
            })
        ),
    )
    .unwrap();
    let connection = create_current_state_db(&root);
    connection
            .execute(
                "INSERT INTO threads
                 (id, rollout_path, title, updated_at, updated_at_ms, preview, recency_at, recency_at_ms)
                 VALUES (?1, ?2, '数据库长标题', 1, 1000, '原始长消息', 1, 1000)",
                params![session_id, path.to_string_lossy()],
            )
            .unwrap();
    drop(connection);
    fs::write(
        root.join("session_index.jsonl"),
        format!(
            "{}\n",
            json!({
                "id": session_id,
                "thread_name": "Codex 原生标题",
                "updated_at": "2026-05-13T10:54:27.000Z"
            })
        ),
    )
    .unwrap();

    let result = preview_conversation_impl(
        root.to_string_lossy().to_string(),
        path_to_slash(&relative_path),
        None,
        None,
        Some(10),
        None,
        None,
    )
    .unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(
        result["conversation"]["title"],
        Value::String("Codex 原生标题".to_string())
    );
}

#[test]
fn current_state_title_falls_back_when_session_index_has_no_match() {
    let root = temp_path("current-state-title-fallback");
    let session_id = "019e20f9-34b7-7a82-a95b-fe461de8983a";
    let path = root.join("sessions").join("rollout-title-fallback.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{}\n").unwrap();
    let connection = create_current_state_db(&root);
    connection
            .execute(
                "INSERT INTO threads
                 (id, rollout_path, title, updated_at, updated_at_ms, preview, recency_at, recency_at_ms)
                 VALUES (?1, ?2, '数据库标题', 1, 1000, '预览', 1, 1000)",
                params![session_id, path.to_string_lossy()],
            )
            .unwrap();
    drop(connection);

    let catalog = read_current_state_conversations(&root, &SessionIndex::new()).unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(catalog.conversations.len(), 1);
    assert_eq!(catalog.conversations[0].title, "数据库标题");
}

#[test]
fn upsert_state_threads_supports_older_thread_schema() {
    let root = temp_path("upsert-old-schema");
    fs::create_dir_all(&root).unwrap();
    let state_db = root.join("state_5.sqlite");
    let connection = Connection::open(&state_db).unwrap();
    connection
        .execute_batch(
            r#"
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    title TEXT,
                    updated_at INTEGER
                );
                "#,
        )
        .unwrap();
    drop(connection);

    let item = sample_thread_metadata(root.join("sessions/rollout-thread-1.jsonl"));
    let updated = upsert_state_threads(&root, &[item]).unwrap();
    let connection = Connection::open(&state_db).unwrap();
    let row = connection
        .query_row(
            "SELECT rollout_path, title, updated_at FROM threads WHERE id = 'thread-1'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap();

    assert_eq!(updated, 1);
    assert!(row.0.ends_with("sessions/rollout-thread-1.jsonl"));
    assert_eq!(row.1, "Imported thread");
    assert_eq!(row.2, 1_700_000_120);

    drop(connection);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn upsert_state_threads_ignores_legacy_nested_state_db() {
    let root = temp_path("upsert-current-state-db");
    let current_db = root.join("state_5.sqlite");
    let legacy_db = root.join("sqlite").join("state_5.sqlite");
    for state_db in [&current_db, &legacy_db] {
        fs::create_dir_all(state_db.parent().unwrap()).unwrap();
        let connection = Connection::open(state_db).unwrap();
        connection
            .execute_batch(
                r#"
                    CREATE TABLE threads (
                        id TEXT PRIMARY KEY,
                        rollout_path TEXT,
                        title TEXT,
                        updated_at INTEGER
                    );
                    "#,
            )
            .unwrap();
    }

    let item = sample_thread_metadata(root.join("sessions/rollout-thread-1.jsonl"));
    let updated = upsert_state_threads(&root, &[item]).unwrap();
    let current = Connection::open(&current_db).unwrap();
    let legacy = Connection::open(&legacy_db).unwrap();
    let current_count: i64 = current
        .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
        .unwrap();
    let legacy_count: i64 = legacy
        .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
        .unwrap();

    assert_eq!(updated, 1);
    assert_eq!(current_count, 1);
    assert_eq!(legacy_count, 0);

    drop(current);
    drop(legacy);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn upsert_state_threads_writes_current_schema_relationships() {
    let root = temp_path("upsert-current-schema");
    fs::create_dir_all(&root).unwrap();
    let state_db = root.join("state_5.sqlite");
    let connection = Connection::open(&state_db).unwrap();
    connection
        .execute_batch(
            r#"
                PRAGMA foreign_keys = ON;
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    source TEXT NOT NULL,
                    updated_at INTEGER NOT NULL,
                    title TEXT NOT NULL,
                    cli_version TEXT NOT NULL DEFAULT '',
                    agent_nickname TEXT,
                    agent_role TEXT,
                    agent_path TEXT,
                    thread_source TEXT,
                    preview TEXT NOT NULL DEFAULT '',
                    recency_at INTEGER NOT NULL DEFAULT 0,
                    recency_at_ms INTEGER NOT NULL DEFAULT 0,
                    history_mode TEXT NOT NULL DEFAULT 'legacy'
                );
                CREATE TABLE thread_spawn_edges (
                    parent_thread_id TEXT NOT NULL,
                    child_thread_id TEXT NOT NULL PRIMARY KEY,
                    status TEXT NOT NULL
                );
                CREATE TABLE thread_dynamic_tools (
                    thread_id TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL,
                    input_schema TEXT NOT NULL,
                    defer_loading INTEGER NOT NULL DEFAULT 0,
                    namespace TEXT,
                    PRIMARY KEY(thread_id, position),
                    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
                );
                "#,
        )
        .unwrap();
    drop(connection);

    let mut item = sample_thread_metadata(root.join("sessions/rollout-thread-1.jsonl"));
    item.source =
        r#"{"subagent":{"thread_spawn":{"parent_thread_id":"parent-thread"}}}"#.to_string();
    item.agent_nickname = Some("Curie".to_string());
    item.agent_role = Some("explorer".to_string());
    item.agent_path = Some("/root/audit".to_string());
    item.thread_source = Some("subagent".to_string());
    item.preview = "hello current schema".to_string();
    item.parent_thread_id = Some("parent-thread".to_string());
    item.dynamic_tools = vec![ThreadDynamicToolMetadata {
        name: "read_thread".to_string(),
        description: "Read a thread".to_string(),
        input_schema: r#"{"type":"object"}"#.to_string(),
        defer_loading: true,
        namespace: Some("codex_app".to_string()),
    }];

    assert_eq!(upsert_state_threads(&root, &[item]).unwrap(), 1);
    let connection = Connection::open(&state_db).unwrap();
    let thread = connection
        .query_row(
            "SELECT source, cli_version, agent_nickname, agent_role, agent_path,
                        thread_source, preview, recency_at, recency_at_ms, history_mode
                 FROM threads WHERE id = 'thread-1'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .unwrap();
    let edge = connection
            .query_row(
                "SELECT parent_thread_id, status FROM thread_spawn_edges WHERE child_thread_id = 'thread-1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
    let tool = connection
        .query_row(
            "SELECT name, description, input_schema, defer_loading, namespace
                 FROM thread_dynamic_tools WHERE thread_id = 'thread-1' AND position = 0",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .unwrap();

    assert!(thread.0.contains("parent-thread"));
    assert_eq!(thread.1, "0.144.1");
    assert_eq!(thread.2.as_deref(), Some("Curie"));
    assert_eq!(thread.3.as_deref(), Some("explorer"));
    assert_eq!(thread.4.as_deref(), Some("/root/audit"));
    assert_eq!(thread.5.as_deref(), Some("subagent"));
    assert_eq!(thread.6, "hello current schema");
    assert_eq!(thread.7, 1_700_000_120);
    assert_eq!(thread.8, 1_700_000_120_000);
    assert_eq!(thread.9, "legacy");
    assert_eq!(edge, ("parent-thread".to_string(), "closed".to_string()));
    assert_eq!(tool.0, "read_thread");
    assert_eq!(tool.1, "Read a thread");
    assert_eq!(tool.2, r#"{"type":"object"}"#);
    assert_eq!(tool.3, 1);
    assert_eq!(tool.4.as_deref(), Some("codex_app"));

    drop(connection);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn insert_missing_state_threads_never_overwrites_current_rows_or_relationships() {
    let root = temp_path("insert-missing-current-wins");
    fs::create_dir_all(&root).unwrap();
    let state_db = root.join("state_5.sqlite");
    let connection = Connection::open(&state_db).unwrap();
    connection
            .execute_batch(
                r#"
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    title TEXT NOT NULL DEFAULT '',
                    updated_at INTEGER NOT NULL DEFAULT 0,
                    preview TEXT NOT NULL DEFAULT '',
                    recency_at INTEGER NOT NULL DEFAULT 0,
                    recency_at_ms INTEGER NOT NULL DEFAULT 0,
                    history_mode TEXT NOT NULL DEFAULT 'legacy'
                );
                CREATE TABLE thread_spawn_edges (
                    parent_thread_id TEXT NOT NULL,
                    child_thread_id TEXT NOT NULL PRIMARY KEY,
                    status TEXT NOT NULL
                );
                CREATE TABLE thread_dynamic_tools (
                    thread_id TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL,
                    input_schema TEXT NOT NULL,
                    defer_loading INTEGER NOT NULL DEFAULT 0,
                    namespace TEXT,
                    PRIMARY KEY(thread_id, position)
                );
                INSERT INTO threads
                  (id, rollout_path, title, updated_at, preview, recency_at, recency_at_ms, history_mode)
                VALUES
                  ('thread-1', 'sessions/current.jsonl', 'Current title', 99, 'Current preview', 99, 99000, 'full');
                INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
                VALUES ('current-parent', 'thread-1', 'ready');
                INSERT INTO thread_dynamic_tools
                  (thread_id, position, name, description, input_schema, defer_loading, namespace)
                VALUES ('thread-1', 0, 'current-tool', 'current', '{}', 0, NULL);
                "#,
            )
            .unwrap();
    drop(connection);

    let mut item = sample_thread_metadata(root.join("sessions/replacement.jsonl"));
    item.title = "Replacement title".to_string();
    item.preview = "Replacement preview".to_string();
    item.parent_thread_id = Some("replacement-parent".to_string());
    item.dynamic_tools = vec![ThreadDynamicToolMetadata {
        name: "replacement-tool".to_string(),
        description: "replacement".to_string(),
        input_schema: "{}".to_string(),
        defer_loading: true,
        namespace: None,
    }];

    assert_eq!(insert_missing_state_threads(&root, &[item]).unwrap(), 0);
    let connection = Connection::open(&state_db).unwrap();
    let thread: (String, String, i64, String) = connection
        .query_row(
            "SELECT rollout_path, title, updated_at, preview FROM threads WHERE id = 'thread-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    let edge: String = connection
        .query_row(
            "SELECT parent_thread_id FROM thread_spawn_edges WHERE child_thread_id = 'thread-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let tool: String = connection
        .query_row(
            "SELECT name FROM thread_dynamic_tools WHERE thread_id = 'thread-1' AND position = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(thread.0, "sessions/current.jsonl");
    assert_eq!(thread.1, "Current title");
    assert_eq!(thread.2, 99);
    assert_eq!(thread.3, "Current preview");
    assert_eq!(edge, "current-parent");
    assert_eq!(tool, "current-tool");

    drop(connection);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn upsert_state_threads_rejects_schemas_it_cannot_write() {
    for (name, schema, expected) in [
        (
            "unsupported-required",
            "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, unsupported TEXT NOT NULL);",
            "[unsupported]",
        ),
        (
            "missing-rollout-path",
            "CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT);",
            "[rollout_path]",
        ),
        (
            "missing-threads",
            "CREATE TABLE other (id TEXT PRIMARY KEY);",
            "缺少 threads 表",
        ),
    ] {
        let root = temp_path(&format!("upsert-schema-{name}"));
        fs::create_dir_all(&root).unwrap();
        let state_db = root.join("state_5.sqlite");
        Connection::open(&state_db)
            .unwrap()
            .execute_batch(schema)
            .unwrap();

        let item = sample_thread_metadata(root.join("sessions/rollout-thread-1.jsonl"));
        let err = upsert_state_threads(&root, &[item]).unwrap_err();
        let connection = Connection::open(&state_db).unwrap();
        let has_threads: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'threads'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        if has_threads == 1 {
            let count: i64 = connection
                .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 0, "{name}");
        }
        drop(connection);
        fs::remove_dir_all(&root).unwrap();

        assert!(err.contains(expected), "{name}: {err}");
    }
}

fn session_file_name(id: &str) -> String {
    format!("rollout-2026-05-13T18-54-23-{id}.jsonl")
}

fn count_files(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[test]
fn status_db_failure_moves_files_back_and_reports_reason() {
    assert_status_db_failure_moves_files_back(false);
}

#[test]
fn status_cross_format_overwrite_rolls_back_both_files_on_db_failure() {
    assert_status_db_failure_moves_files_back(true);
}

fn assert_status_db_failure_moves_files_back(with_compressed_overwrite: bool) {
    let root = temp_path("status-db-failure-rollback");
    let session_id = "019e20f9-34b7-7a82-a95b-fe461de89801";
    let active_relative = PathBuf::from("sessions/2026/05/13").join(session_file_name(session_id));
    let archived_relative = PathBuf::from("archived_sessions").join(session_file_name(session_id));
    let active_path = if with_compressed_overwrite {
        write_compressed_test_session(&root, &active_relative, session_id, "rollback me")
    } else {
        write_test_session(&root, &active_relative, session_id, "rollback me")
    };
    let overwritten_bytes = with_compressed_overwrite.then(|| {
        let path = write_test_session(&root, &archived_relative, session_id, "original target");
        fs::read(path).unwrap()
    });
    let original = fs::read(&active_path).unwrap();
    // A threads table without archived_at cannot record the new status.
    Connection::open(root.join("state_5.sqlite"))
        .unwrap()
        .execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL, archived INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();

    let result = set_conversation_status_impl(
        root.to_string_lossy().to_string(),
        vec![path_to_slash(&active_relative)],
        "archived".to_string(),
        with_compressed_overwrite.then(|| "overwrite".to_string()),
    )
    .unwrap();
    let archived_path = root.join(&archived_relative);
    let active_after = fs::read(&active_path).ok();
    let archived_after = fs::read(&archived_path).ok();
    let archive_files = count_files(archived_path.parent().unwrap());
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["report"]["changed"], 0);
    let message = result["message"].as_str().unwrap();
    assert!(message.contains("archived_at"), "{message}");
    assert!(message.contains("已撤销 1 个文件移动"), "{message}");
    assert_eq!(active_after, Some(original));
    assert_eq!(archived_after, overwritten_bytes);
    assert_eq!(archive_files, usize::from(with_compressed_overwrite));
}

#[test]
fn status_round_trip_preserves_the_compressed_rollout() {
    let root = temp_path("status-compressed-round-trip");
    let id = "019e20f9-34b7-7a82-a95b-fe461de89806";
    let file_name = session_file_name(id);
    let active_relative = PathBuf::from("sessions/2026/05/13").join(&file_name);
    let archived_relative = PathBuf::from("archived_sessions").join(&file_name);
    let active_path = write_compressed_test_session(&root, &active_relative, id, "preserve bytes");
    let archive_path = compressed_rollout_path(&root.join(&archived_relative));
    let original = fs::read(&active_path).unwrap();
    let connection = create_current_state_db(&root);
    connection
        .execute(
            "INSERT INTO threads (id, rollout_path) VALUES (?1, ?2)",
            params![id, root.join(&active_relative).to_string_lossy()],
        )
        .unwrap();
    drop(connection);
    for (relative, status, moved, from) in [
        (&active_relative, "archived", &archive_path, &active_path),
        (&archived_relative, "active", &active_path, &archive_path),
    ] {
        let result = set_conversation_status_impl(
            root.to_string_lossy().to_string(),
            vec![path_to_slash(relative)],
            status.to_string(),
            None,
        )
        .unwrap();
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["report"]["changed"], 1);
        assert_eq!(fs::read(moved).unwrap(), original);
        assert!(!from.exists());
        assert!(!logical_rollout_path(moved).exists());
        let row = Connection::open(root.join("state_5.sqlite"))
            .unwrap()
            .query_row(
                "SELECT archived, rollout_path FROM threads WHERE id = ?1",
                [id],
                |row| Ok((row.get::<_, bool>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, status == "archived");
        assert_eq!(
            conversation_path_key(Path::new(&row.1)),
            conversation_path_key(moved)
        );
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn status_modify_id_moves_child_rows_with_the_renamed_thread() {
    assert_status_modify_id_moves_child_rows(false);
}

#[test]
fn status_modify_id_rewrites_and_keeps_compression() {
    assert_status_modify_id_moves_child_rows(true);
}

fn assert_status_modify_id_moves_child_rows(compressed: bool) {
    let root = temp_path("status-modify-id-children");
    let session_id = "019e20f9-34b7-7a82-a95b-fe461de89802";
    let file_name = session_file_name(session_id);
    let active_relative = PathBuf::from("sessions/2026/05/13").join(&file_name);
    let active_path = if compressed {
        write_compressed_test_session(&root, &active_relative, session_id, "modify id")
    } else {
        write_test_session(&root, &active_relative, session_id, "modify id")
    };
    let existing_archived = write_test_session(
        &root,
        &PathBuf::from("archived_sessions").join(&file_name),
        session_id,
        "already archived",
    );
    let existing_archived_bytes = fs::read(&existing_archived).unwrap();
    let connection = create_current_state_db(&root);
    connection
        .execute_batch(
            r#"
            CREATE TABLE thread_dynamic_tools (
                thread_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                name TEXT NOT NULL,
                PRIMARY KEY(thread_id, position),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );
            CREATE TABLE thread_spawn_edges (
                parent_thread_id TEXT NOT NULL,
                child_thread_id TEXT NOT NULL PRIMARY KEY,
                status TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO threads (id, rollout_path, title) VALUES (?1, ?2, 'modify')",
            params![session_id, active_path.to_string_lossy()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO thread_dynamic_tools (thread_id, position, name) VALUES (?1, 0, 'tool')",
            [session_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status) VALUES ('parent', ?1, 'closed')",
            [session_id],
        )
        .unwrap();
    // Premise: renaming only threads.id breaks the foreign key (enforced by the bundled SQLite).
    let direct_rename = connection
        .execute(
            "UPDATE threads SET id = 'renamed-only-parent' WHERE id = ?1",
            [session_id],
        )
        .unwrap_err()
        .to_string();
    drop(connection);

    let result = set_conversation_status_impl(
        root.to_string_lossy().to_string(),
        vec![path_to_slash(&active_relative)],
        "archived".to_string(),
        Some("modify_id".to_string()),
    )
    .unwrap();
    let connection = Connection::open(root.join("state_5.sqlite")).unwrap();
    let (new_id, archived, rollout_path): (String, i64, String) = connection
        .query_row(
            "SELECT id, archived, rollout_path FROM threads",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let tool_owner: String = connection
        .query_row("SELECT thread_id FROM thread_dynamic_tools", [], |row| {
            row.get(0)
        })
        .unwrap();
    let edge_child: String = connection
        .query_row(
            "SELECT child_thread_id FROM thread_spawn_edges",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let fk_violations = connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |_| Ok(()))
        .unwrap()
        .count();
    drop(connection);
    let moved_summary = parse_session_file_for_list(Path::new(&rollout_path)).unwrap();
    let active_exists = active_path.exists();
    let existing_after = fs::read(&existing_archived).unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert!(direct_rename.contains("FOREIGN KEY"), "{direct_rename}");
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["report"]["changed"], 1);
    assert_ne!(new_id, session_id);
    assert_eq!(archived, 1);
    assert_eq!(is_compressed_rollout(Path::new(&rollout_path)), compressed);
    assert_eq!(moved_summary.id.as_deref(), Some(new_id.as_str()));
    assert_eq!(tool_owner, new_id);
    assert_eq!(edge_child, new_id);
    assert_eq!(fk_violations, 0);
    assert!(!active_exists);
    assert_eq!(existing_after, existing_archived_bytes);
}

#[test]
fn status_overwrite_removes_overwritten_rows_and_surfaces_global_state_failure() {
    let root = temp_path("status-overwrite-cleanup");
    let session_id = "019e20f9-34b7-7a82-a95b-fe461de89803";
    let overwritten_id = "019e20f9-34b7-7a82-a95b-fe461de89804";
    let file_name = session_file_name(session_id);
    let active_relative = PathBuf::from("sessions/2026/05/13").join(&file_name);
    let active_path = write_test_session(&root, &active_relative, session_id, "winner");
    let active_bytes = fs::read(&active_path).unwrap();
    let archived_path = write_test_session(
        &root,
        &PathBuf::from("archived_sessions").join(&file_name),
        overwritten_id,
        "overwritten",
    );
    let connection = create_current_state_db(&root);
    connection
        .execute_batch(
            "CREATE TABLE thread_dynamic_tools (
                thread_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                name TEXT NOT NULL,
                PRIMARY KEY(thread_id, position),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );",
        )
        .unwrap();
    for (id, path) in [(session_id, &active_path), (overwritten_id, &archived_path)] {
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path) VALUES (?1, ?2)",
                params![id, path.to_string_lossy()],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO thread_dynamic_tools (thread_id, position, name) VALUES (?1, 0, 'tool')",
            [overwritten_id],
        )
        .unwrap();
    drop(connection);
    fs::write(root.join(".codex-global-state.json"), b"not json").unwrap();

    let result = set_conversation_status_impl(
        root.to_string_lossy().to_string(),
        vec![path_to_slash(&active_relative)],
        "archived".to_string(),
        Some("overwrite".to_string()),
    )
    .unwrap();
    let connection = Connection::open(root.join("state_5.sqlite")).unwrap();
    let rows = connection
        .prepare("SELECT id, archived, rollout_path FROM threads")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let tools: i64 = connection
        .query_row("SELECT COUNT(*) FROM thread_dynamic_tools", [], |row| {
            row.get(0)
        })
        .unwrap();
    drop(connection);
    let archived_bytes = fs::read(&archived_path).unwrap();
    let archived_files = count_files(archived_path.parent().unwrap());
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["report"]["changed"], 1);
    assert!(result["message"].as_str().unwrap().contains("global state"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, session_id);
    assert_eq!(rows[0].1, 1);
    assert_eq!(
        conversation_path_key(Path::new(&rows[0].2)),
        conversation_path_key(&archived_path)
    );
    assert_eq!(tools, 0);
    assert_eq!(archived_bytes, active_bytes);
    assert_eq!(archived_files, 1, "overwrite backup must be removed");
}

#[test]
fn legacy_migration_rejects_unwritable_schema_without_marker_or_backup() {
    let root = temp_path("legacy-migration-unwritable");
    let marker = root.join("markers").join("migration.json");
    fs::create_dir_all(root.join("sqlite")).unwrap();
    Connection::open(root.join("state_5.sqlite"))
        .unwrap()
        .execute_batch(
            r#"
            CREATE TABLE _sqlx_migrations (version INTEGER PRIMARY KEY, success INTEGER NOT NULL);
            INSERT INTO _sqlx_migrations (version, success) VALUES (40, 1);
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                cwd TEXT NOT NULL DEFAULT '',
                archived INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT 0,
                updated_at_ms INTEGER NOT NULL DEFAULT 0,
                preview TEXT NOT NULL DEFAULT '',
                recency_at INTEGER NOT NULL DEFAULT 0,
                recency_at_ms INTEGER NOT NULL DEFAULT 0,
                history_mode TEXT NOT NULL DEFAULT 'legacy',
                unsupported TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
    Connection::open(root.join("sqlite").join("state_5.sqlite"))
        .unwrap()
        .execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL);")
        .unwrap();
    write_test_session(
        &root,
        Path::new("sessions/2026/07/13/rollout-unindexed.jsonl"),
        "019f0000-0000-7000-8000-000000000012",
        "unindexed",
    );
    let backup_dir = session_manager_data_dir()
        .unwrap()
        .join("backups")
        .join("desktop-final-v2-migration");
    let backups_before = count_files(&backup_dir);

    let err = migrate_legacy_codex_data_with_marker(&root, &marker).unwrap_err();
    let marker_exists = marker.exists();
    let backups_after = count_files(&backup_dir);
    fs::remove_dir_all(&root).unwrap();

    assert!(err.contains("[unsupported]"), "{err}");
    assert!(!marker_exists);
    assert_eq!(backups_after, backups_before);
}

#[test]
fn legacy_migration_writes_its_marker_and_runs_once() {
    let root = temp_path("legacy-migration-marker");
    let marker = root.join("markers").join("migration.json");
    drop(create_current_state_db(&root));

    let first = migrate_legacy_codex_data_with_marker(&root, &marker).unwrap();
    let marker_report: Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
    let leftovers = fs::read_dir(marker.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != "migration.json")
        .count();
    let second = migrate_legacy_codex_data_with_marker(&root, &marker).unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(first["completed"], true, "{first}");
    assert_eq!(marker_report["completed"], true);
    assert_eq!(leftovers, 0);
    assert_eq!(second, marker_report);
}

#[test]
fn export_bundle_manifest_matches_exported_bytes_and_imports_cleanly() {
    let base = temp_path("export-bundle");
    let source_root = base.join("source");
    let target_root = base.join("target");
    let indexed_relative = PathBuf::from("sessions/2026/07/13/rollout-export-indexed.jsonl");
    let unindexed_relative = PathBuf::from("archived_sessions/rollout-export-unindexed.jsonl");
    let indexed = write_test_session(
        &source_root,
        &indexed_relative,
        "019f0000-0000-7000-8000-000000000021",
        "indexed",
    );
    let compressed = write_compressed_test_session(
        &source_root,
        &unindexed_relative,
        "019f0000-0000-7000-8000-000000000022",
        "unindexed",
    );
    let original_compressed = fs::read(&compressed).unwrap();
    let connection = create_current_state_db(&source_root);
    connection
        .execute(
            "INSERT INTO threads (id, rollout_path, title) VALUES ('019f0000-0000-7000-8000-000000000021', ?1, 'Indexed')",
            [indexed.to_string_lossy()],
        )
        .unwrap();
    drop(connection);
    fs::create_dir_all(target_root.join("sessions")).unwrap();

    let bundle = collect_export_bundle(
        &source_root,
        vec![
            path_to_slash(&indexed_relative),
            path_to_slash(&unindexed_relative),
            path_to_slash(&indexed_relative),
            "../outside.jsonl".to_string(),
        ],
    )
    .unwrap();
    let zip_path = base.join("export.zip");
    write_zip_store(&zip_path, &bundle.entries).unwrap();
    let archive = ZipArchiveLite::open(&zip_path).unwrap();
    let candidates = bundle
        .sessions
        .iter()
        .map(|session| build_import_candidate(&target_root, &archive, session).unwrap())
        .collect::<Vec<_>>();
    let repeat = build_import_candidate(&source_root, &archive, &bundle.sessions[1]).unwrap();
    let repeat_action = repeat.action;
    let outcome = apply_import_candidates(&target_root, &candidates).unwrap();
    let repeat_outcome = apply_import_candidates(&source_root, &[repeat]).unwrap();
    let compressed_after = fs::read(&compressed).unwrap();
    let source_has_plain = source_root.join(&unindexed_relative).exists();
    let restored = fs::read(target_root.join(&unindexed_relative)).unwrap();
    fs::remove_dir_all(&base).unwrap();

    assert_eq!(bundle.sessions.len(), 2);
    assert_eq!(bundle.errors.len(), 1, "{:?}", bundle.errors);
    for (session, (name, data)) in bundle.sessions.iter().zip(&bundle.entries) {
        assert_eq!(&session.relative_path, name);
        assert_eq!(session.size_bytes, data.len() as u64);
        assert_eq!(session.sha256, sha256_bytes(data));
    }
    assert_eq!(
        bundle.total_size,
        bundle
            .entries
            .iter()
            .map(|(_, data)| data.len() as u64)
            .sum::<u64>()
    );
    for candidate in candidates {
        assert_eq!(candidate.action, ImportAction::Import);
    }
    assert_eq!(outcome.imported, 2, "{:?}", outcome.errors);
    assert_eq!(
        restored,
        test_session_content("019f0000-0000-7000-8000-000000000022", "unindexed").into_bytes()
    );
    assert_eq!(repeat_action, ImportAction::SkipSame);
    assert_eq!(repeat_outcome.skipped, 1, "{:?}", repeat_outcome.errors);
    assert!(
        repeat_outcome.sqlite_error.is_none(),
        "{:?}",
        repeat_outcome.sqlite_error
    );
    assert!(!source_has_plain);
    assert_eq!(compressed_after, original_compressed);
}

fn import_candidate(
    root: &Path,
    relative: &str,
    id: &str,
    action: ImportAction,
) -> ImportCandidate {
    let data = format!(
        "{}\n",
        json!({
            "timestamp": "2026-07-13T00:00:00Z",
            "type": "session_meta",
            "payload": {"id": id, "cwd": "C:\\work"}
        })
    )
    .into_bytes();
    ImportCandidate {
        manifest: ManifestSession {
            id: id.to_string(),
            title: format!("title-{id}"),
            updated_at: Some("2026-07-13T00:00:00Z".to_string()),
            status: "active".to_string(),
            relative_path: relative.to_string(),
            size_bytes: data.len() as u64,
            sha256: sha256_bytes(&data),
        },
        target_path: root.join(relative),
        data,
        action,
    }
}

#[test]
fn import_apply_never_overwrites_late_files_and_indexes_what_was_written() {
    let root = temp_path("import-apply");
    drop(create_current_state_db(&root));
    let written = import_candidate(
        &root,
        "sessions/2026/07/13/rollout-written.jsonl",
        "import-written",
        ImportAction::Import,
    );
    let late = import_candidate(
        &root,
        "sessions/2026/07/13/rollout-late.jsonl",
        "import-late",
        ImportAction::Import,
    );
    let late_compressed = import_candidate(
        &root,
        "sessions/2026/07/13/rollout-late-compressed.jsonl",
        "import-late-compressed",
        ImportAction::Import,
    );
    let blocked = import_candidate(
        &root,
        "sessions/2026/07/blocked/rollout-blocked.jsonl",
        "import-blocked",
        ImportAction::Import,
    );
    let same = import_candidate(
        &root,
        "sessions/2026/07/13/rollout-same.jsonl",
        "import-same",
        ImportAction::SkipSame,
    );
    fs::create_dir_all(late.target_path.parent().unwrap()).unwrap();
    // Appeared after the dialog classified it as new.
    fs::write(&late.target_path, b"written by someone else\n").unwrap();
    let compressed = compressed_rollout_path(&late_compressed.target_path);
    let late_compressed_bytes =
        zstd::encode_all(b"compressed by someone else\n".as_slice(), 0).unwrap();
    fs::write(&compressed, &late_compressed_bytes).unwrap();
    // A file where the parent directory should be makes the write fail.
    fs::write(root.join("sessions/2026/07/blocked"), b"not a directory").unwrap();
    fs::write(&same.target_path, &same.data).unwrap();

    let outcome = apply_import_candidates(
        &root,
        &[
            written.clone(),
            late.clone(),
            late_compressed.clone(),
            blocked.clone(),
            same.clone(),
        ],
    )
    .unwrap();
    let late_bytes = fs::read(&late.target_path).unwrap();
    assert!(!late_compressed.target_path.exists());
    assert_eq!(fs::read(compressed).unwrap(), late_compressed_bytes);
    let written_bytes = fs::read(&written.target_path).unwrap();
    let backup_exists = outcome
        .state_backup_path
        .as_ref()
        .is_some_and(|path| path.exists());
    let connection = Connection::open(root.join("state_5.sqlite")).unwrap();
    let mut ids = connection
        .prepare("SELECT id FROM threads ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    ids.sort();
    drop(connection);
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(outcome.imported, 1);
    assert_eq!(outcome.skipped, 1);
    assert_eq!(outcome.conflicts.len(), 2);
    assert_eq!(
        outcome.conflicts[0]["relative_path"],
        late.manifest.relative_path
    );
    assert_eq!(outcome.errors.len(), 1, "{:?}", outcome.errors);
    assert!(outcome.errors[0].contains("rollout-blocked.jsonl"));
    assert_eq!(outcome.sqlite_error, None);
    assert_eq!(outcome.sqlite_updated, 2);
    assert_eq!(ids, vec!["import-same", "import-written"]);
    assert_eq!(late_bytes, b"written by someone else\n");
    assert_eq!(written_bytes, written.data);
    assert!(backup_exists);
}

#[test]
fn state_db_backup_includes_uncheckpointed_wal_pages() {
    let root = temp_path("state-db-wal-backup");
    fs::create_dir_all(&root).unwrap();
    let state_db = root.join("state_5.sqlite");
    let writer = Connection::open(&state_db).unwrap();
    let mode: String = writer
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .unwrap();
    writer
        .execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY);
             INSERT INTO threads (id) VALUES ('a'), ('b'), ('c');",
        )
        .unwrap();
    // Premise: while the writer is open the rows live in state_5.sqlite-wal, so a plain copy of
    // the main file does not contain them.
    let plain_copy = root.join("plain-copy.sqlite");
    fs::copy(&state_db, &plain_copy).unwrap();
    let plain_rows = Connection::open(&plain_copy)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM threads", [], |row| {
            row.get::<_, i64>(0)
        })
        .ok();

    let backup = backup_state_database_file(&state_db, "wal-test").unwrap();
    let backup_rows: i64 = Connection::open(&backup)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
        .unwrap();
    drop(writer);
    fs::remove_file(&backup).unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(mode.to_ascii_lowercase(), "wal");
    assert_ne!(plain_rows, Some(3));
    assert_eq!(backup_rows, 3);
}

#[test]
fn preview_falls_back_to_rollout_when_state_db_has_no_row() {
    let root = temp_path("preview-fallback");
    let session_id = "019e20f9-34b7-7a82-a95b-fe461de89805";
    let relative = PathBuf::from("sessions/2026/05/13").join(session_file_name(session_id));
    write_test_session(&root, &relative, session_id, "fallback title");
    drop(create_current_state_db(&root));

    let result = preview_conversation_impl(
        root.to_string_lossy().to_string(),
        path_to_slash(&relative),
        None,
        None,
        Some(10),
        None,
        None,
    )
    .unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(result["conversation"]["id"], session_id);
    assert_eq!(result["conversation"]["title"], "fallback title");
    assert_eq!(result["messages"].as_array().unwrap().len(), 2);
}

#[test]
fn session_index_skips_non_utf8_lines_and_keeps_reading() {
    let root = temp_path("session-index-non-utf8");
    fs::create_dir_all(&root).unwrap();
    let mut content =
        format!("{}\n", json!({"id": "before", "thread_name": "Before"})).into_bytes();
    content.extend_from_slice(b"{\"id\":\"broken\",\"thread_name\":\"\xff\xfe\"}\n");
    content.extend_from_slice(
        format!("{}\n", json!({"id": "after", "thread_name": "After"})).as_bytes(),
    );
    fs::write(root.join("session_index.jsonl"), content).unwrap();

    let mut warnings = Vec::new();
    let index = read_session_index(&root, &mut warnings);
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(
        session_index_title(&index, "before").as_deref(),
        Some("Before")
    );
    assert_eq!(
        session_index_title(&index, "after").as_deref(),
        Some("After")
    );
    assert!(session_index_title(&index, "broken").is_none());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("第 2 行"), "{warnings:?}");
}

#[test]
fn global_state_cleanup_removes_pinned_thread_ids() {
    let mut value = json!({
        "pinned-thread-ids": ["keep", "remove"],
        "nested": {
            "pinnedThreadIds": ["remove", "keep"],
            "remove": { "title": "old" },
            "keep": { "title": "current" }
        }
    });
    let ids = HashSet::from(["remove"]);

    let removed = remove_matching_object_keys(&mut value, &ids);

    assert_eq!(removed, 3);
    assert_eq!(value["pinned-thread-ids"], json!(["keep"]));
    assert_eq!(value["nested"]["pinnedThreadIds"], json!(["keep"]));
    assert!(value["nested"].get("remove").is_none());
    assert!(value["nested"].get("keep").is_some());
}

#[test]
fn legacy_state_metadata_migration_preserves_current_rows_and_backfills_recency() {
    let root = temp_path("legacy-state-migration");
    fs::create_dir_all(root.join("sqlite")).unwrap();
    let current_path = root.join("state_5.sqlite");
    let legacy_path = root.join("sqlite").join("state_5.sqlite");
    let current_schema = r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                recency_at INTEGER NOT NULL DEFAULT 0,
                recency_at_ms INTEGER NOT NULL DEFAULT 0,
                history_mode TEXT NOT NULL DEFAULT 'legacy'
            );
            CREATE TABLE thread_spawn_edges (
                parent_thread_id TEXT NOT NULL,
                child_thread_id TEXT PRIMARY KEY,
                status TEXT NOT NULL
            );
            CREATE TABLE thread_dynamic_tools (
                thread_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                name TEXT NOT NULL,
                PRIMARY KEY(thread_id, position)
            );
            CREATE TABLE agent_jobs (id TEXT PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE agent_job_items (
                job_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                row_index INTEGER NOT NULL,
                PRIMARY KEY(job_id, item_id)
            );
        "#;
    let legacy_schema = r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE thread_spawn_edges (
                parent_thread_id TEXT NOT NULL,
                child_thread_id TEXT PRIMARY KEY,
                status TEXT NOT NULL
            );
            CREATE TABLE thread_dynamic_tools (
                thread_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                name TEXT NOT NULL,
                PRIMARY KEY(thread_id, position)
            );
            CREATE TABLE agent_jobs (id TEXT PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE agent_job_items (
                job_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                row_index INTEGER NOT NULL,
                PRIMARY KEY(job_id, item_id)
            );
        "#;
    let current = Connection::open(&current_path).unwrap();
    current.execute_batch(current_schema).unwrap();
    current
            .execute(
                "INSERT INTO threads (id, rollout_path, created_at, updated_at, title, recency_at, recency_at_ms)
                 VALUES ('same', 'sessions/current.jsonl', 1, 2, 'Current', 2, 2000)",
                [],
            )
            .unwrap();
    current
        .execute_batch(
            "INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
                 VALUES ('current-parent', 'same', 'current');
                 INSERT INTO thread_dynamic_tools (thread_id, position, name)
                 VALUES ('same', 0, 'current-tool');",
        )
        .unwrap();
    drop(current);
    let legacy = Connection::open(&legacy_path).unwrap();
    legacy.execute_batch(legacy_schema).unwrap();
    legacy
        .execute_batch(
            "INSERT INTO threads (id, rollout_path, created_at, updated_at, title)
                 VALUES ('same', 'sessions/legacy-same.jsonl', 1, 3, 'Legacy');
                 INSERT INTO threads (id, rollout_path, created_at, updated_at, title)
                 VALUES ('old', 'sessions/old.jsonl', 10, 20, 'Old');
                 INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
                 VALUES ('same', 'old', 'ready');
                 INSERT INTO thread_dynamic_tools (thread_id, position, name)
                 VALUES ('old', 0, 'tool');
                 INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
                 VALUES ('legacy-parent', 'same', 'legacy');
                 INSERT INTO thread_dynamic_tools (thread_id, position, name)
                 VALUES ('same', 0, 'legacy-tool');
                 INSERT INTO agent_jobs (id, name) VALUES ('job-old', 'Old job');
                 INSERT INTO agent_job_items (job_id, item_id, row_index)
                 VALUES ('job-old', 'item-1', 0);",
        )
        .unwrap();
    drop(legacy);

    let mut current = Connection::open(&current_path).unwrap();
    let schema = state_threads_schema(&current).unwrap().unwrap();
    let inserted = merge_legacy_state_metadata(&mut current, &legacy_path, &schema).unwrap();
    let same_title: String = current
        .query_row("SELECT title FROM threads WHERE id = 'same'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let old_row: (i64, i64, String) = current
        .query_row(
            "SELECT recency_at, recency_at_ms, history_mode FROM threads WHERE id = 'old'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let same_edge: (String, String) = current
            .query_row(
                "SELECT parent_thread_id, status FROM thread_spawn_edges WHERE child_thread_id = 'same'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
    let same_tool: String = current
        .query_row(
            "SELECT name FROM thread_dynamic_tools WHERE thread_id = 'same' AND position = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(inserted.get("threads"), Some(&1));
    assert_eq!(inserted.get("thread_spawn_edges"), Some(&1));
    assert_eq!(inserted.get("thread_dynamic_tools"), Some(&1));
    assert_eq!(inserted.get("agent_jobs"), Some(&1));
    assert_eq!(inserted.get("agent_job_items"), Some(&1));
    assert_eq!(same_title, "Current");
    assert_eq!(old_row, (20, 20_000, "legacy".to_string()));
    assert_eq!(
        same_edge,
        ("current-parent".to_string(), "current".to_string())
    );
    assert_eq!(same_tool, "current-tool");

    drop(current);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn status_change_without_state_rows_takes_no_backup() {
    let root = temp_path("status-no-state-rows");
    let session_id = "019e20f9-34b7-7a82-a95b-fe461de89806";
    let relative = PathBuf::from("sessions/2026/05/13").join(session_file_name(session_id));
    write_test_session(&root, &relative, session_id, "no row");
    drop(create_current_state_db(&root));

    let result = set_conversation_status_impl(
        root.to_string_lossy().to_string(),
        vec![path_to_slash(&relative)],
        "archived".to_string(),
        None,
    )
    .unwrap();
    let archived = root
        .join("archived_sessions")
        .join(session_file_name(session_id))
        .exists();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["report"]["changed"], 1);
    assert!(result["report"]["state_backup_path"].is_null());
    assert!(archived);
}

#[test]
fn status_overwrite_never_replaces_a_session_moved_earlier_in_the_batch() {
    let root = temp_path("status-same-batch-target");
    let file_name = session_file_name("019e20f9-34b7-7a82-a95b-fe461de89810");
    let first_relative = PathBuf::from("sessions/2026/05/13").join(&file_name);
    let second_relative = PathBuf::from("sessions/2026/05/14").join(&file_name);
    let first = write_test_session(&root, &first_relative, "first-id", "first");
    let second = write_test_session(&root, &second_relative, "second-id", "second");
    let first_bytes = fs::read(&first).unwrap();
    let second_bytes = fs::read(&second).unwrap();
    drop(create_current_state_db(&root));
    let paths = vec![
        path_to_slash(&first_relative),
        path_to_slash(&second_relative),
    ];

    let asked = set_conversation_status_impl(
        root.to_string_lossy().to_string(),
        paths.clone(),
        "archived".to_string(),
        None,
    )
    .unwrap();
    let overwritten = set_conversation_status_impl(
        root.to_string_lossy().to_string(),
        paths,
        "archived".to_string(),
        Some("overwrite".to_string()),
    )
    .unwrap();
    let archived = fs::read(root.join("archived_sessions").join(&file_name)).unwrap();
    let second_left = fs::read(&second).unwrap();
    let first_left = first.exists();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(asked["report"]["conflict_action_required"], true, "{asked}");
    assert_eq!(asked["report"]["conflicts"].as_array().unwrap().len(), 1);
    assert_eq!(overwritten["report"]["changed"], 1, "{overwritten}");
    assert!(overwritten["report"]["errors"][0]
        .as_str()
        .unwrap()
        .contains("重复目标"));
    assert_eq!(archived, first_bytes);
    assert_eq!(second_left, second_bytes);
    assert!(!first_left);
}

#[test]
fn single_path_lookup_matches_the_full_catalog() {
    let root = temp_path("catalog-single-path");
    let name = "rollout-2026-05-13T18-54-23-019e20f9-34b7-7a82-a95b-fe461de89807.jsonl";
    let a = write_test_session(
        &root,
        &Path::new("sessions/2026/05/13").join(name),
        "a",
        "a",
    );
    let same_name_elsewhere =
        write_test_session(&root, &Path::new("archived_sessions").join(name), "d", "d");
    let b = write_test_session(
        &root,
        Path::new("sessions/2026/05/14/rollout-b.jsonl"),
        "b",
        "b",
    );
    let unindexed = write_test_session(
        &root,
        Path::new("sessions/2026/05/15/rollout-c.jsonl"),
        "c",
        "c",
    );
    let connection = create_current_state_db(&root);
    let mut rows = vec![
        ("d", same_name_elsewhere.to_string_lossy().to_string(), 9),
        ("a-first", a.to_string_lossy().to_string(), 8),
        // Same file through a relative path: the catalog drops it as a duplicate.
        ("a-dup", format!("sessions/2026/05/13/{name}"), 7),
        ("b", "sessions/2026/05/14/rollout-b.jsonl".to_string(), 6),
        (
            "missing",
            root.join("sessions/gone.jsonl")
                .to_string_lossy()
                .to_string(),
            5,
        ),
        (
            "outside",
            std::env::temp_dir()
                .join("x.jsonl")
                .to_string_lossy()
                .to_string(),
            4,
        ),
    ];
    if cfg!(windows) {
        rows.push(("b-case", b.to_string_lossy().to_uppercase(), 10));
    }
    for (id, rollout_path, recency) in rows {
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, title, recency_at_ms) VALUES (?1, ?2, ?1, ?3)",
                params![id, rollout_path, recency],
            )
            .unwrap();
    }
    drop(connection);
    let index = SessionIndex::new();

    let catalog = read_current_state_conversations(&root, &index).unwrap();
    let mut compared = Vec::new();
    for target in [
        &a,
        &same_name_elsewhere,
        &b,
        &unindexed,
        &root.join("sessions/gone.jsonl"),
    ] {
        let expected = catalog
            .conversations
            .iter()
            .find(|item| {
                conversation_path_key(Path::new(&item.source_path)) == conversation_path_key(target)
            })
            .map(|item| serde_json::to_value(item).unwrap());
        let actual = current_state_conversation_for_path(&root, target, &index)
            .unwrap()
            .map(|item| serde_json::to_value(item).unwrap());
        compared.push((target.clone(), expected, actual));
    }
    fs::remove_dir_all(&root).unwrap();

    for (target, expected, actual) in &compared {
        assert_eq!(actual, expected, "{}", target.display());
    }
    assert_eq!(compared[0].1.as_ref().unwrap()["id"], "a-first");
    assert_eq!(compared[1].1.as_ref().unwrap()["id"], "d");
    assert!(compared[3].1.is_none());
}

#[test]
fn global_state_cleanup_writes_through_the_shared_rewrite_with_a_data_dir_backup() {
    let root = temp_path("global-state-cleanup-merge");
    fs::create_dir_all(&root).unwrap();
    let path = root.join(".codex-global-state.json");
    let original =
        json!({"pinned-thread-ids": ["keep", "remove"], "remove": {"title": "x"}}).to_string();
    fs::write(&path, &original).unwrap();
    let reason = format!(
        "gs-merge-test-{}",
        root.file_name().unwrap().to_string_lossy()
    );
    let backup_dir = session_manager_data_dir()
        .unwrap()
        .join("backups")
        .join(sanitize_backup_reason(&reason));

    remove_from_global_state(&root, &["missing".to_string()], &reason).unwrap();
    let dir_after_noop = backup_dir.exists();
    remove_from_global_state(&root, &["remove".to_string()], &reason).unwrap();
    let rewritten: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let backups = fs::read_dir(&backup_dir)
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect::<Vec<_>>();
    fs::remove_dir_all(&backup_dir).unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert!(!dir_after_noop);
    assert_eq!(rewritten, json!({"pinned-thread-ids": ["keep"]}));
    assert_eq!(backups, vec![original]);
}

#[test]
fn concurrent_backups_with_the_same_reason_get_distinct_files() {
    let root = temp_path("backup-name-race");
    fs::create_dir_all(&root).unwrap();
    let state_db = root.join("state_5.sqlite");
    Connection::open(&state_db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY); INSERT INTO threads VALUES ('a');",
        )
        .unwrap();
    // Short and unique per run: the backup path must stay under the Windows path limit.
    let reason = format!(
        "race-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            % 1_000_000_000
    );
    // Several backups inside one second share the stamp; before names were reserved with
    // create_new, most of these failed with "table threads already exists".
    let results = (0..8)
        .map(|_| {
            let state_db = state_db.clone();
            let reason = reason.clone();
            std::thread::spawn(move || {
                (0..4)
                    .map(|_| {
                        let connection = Connection::open(&state_db).unwrap();
                        backup_state_database_with_reason(&connection, &reason)
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flat_map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    let backup_dir = session_manager_data_dir()
        .unwrap()
        .join("backups")
        .join(sanitize_backup_reason(&reason));
    let files = count_files(&backup_dir);
    let rows = results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .map(|path| {
            Connection::open(path)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM threads", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        })
        .collect::<Vec<_>>();
    fs::remove_dir_all(&backup_dir).unwrap();
    fs::remove_dir_all(&root).unwrap();

    let errors = results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(files, 32);
    assert!(rows.iter().all(|count| *count == 1));
}

#[test]
fn per_session_failures_in_a_report_are_logged() {
    let failed = serde_json::json!({
        "ok": false,
        "report": { "errors": ["移动会话失败 rollout-x.jsonl: 拒绝访问"] }
    });
    super::log_item_errors(
        "session_manager_status_error",
        Some("C:/root-under-test"),
        &failed,
    );
    super::log_item_errors(
        "session_manager_delete_error",
        Some("C:/root-under-test"),
        &serde_json::json!({ "ok": true, "errors": [] }),
    );

    let entries = crate::app_log::get_dev_log_entries();
    let logged = entries.as_array().unwrap().iter().filter(|entry| {
        entry["details"]["event"] == "session_manager_status_error"
            && entry["details"]["details"]["root"] == "C:/root-under-test"
    });
    assert_eq!(logged.count(), 1, "{entries}");
    assert!(!entries.as_array().unwrap().iter().any(|entry| {
        entry["details"]["event"] == "session_manager_delete_error"
            && entry["details"]["details"]["root"] == "C:/root-under-test"
    }));
}
