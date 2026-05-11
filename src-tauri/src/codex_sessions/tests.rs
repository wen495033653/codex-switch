use super::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn unique_sessions_dir(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!("codex-switch-session-sync-{name}-{stamp}"))
}

fn write_rollout_file(path: &Path, provider: &str, cwd: &str) {
    write_rollout_file_with_timestamp(path, provider, cwd, "2026-05-07T00:00:00.000Z");
}

fn write_rollout_file_with_timestamp(path: &Path, provider: &str, cwd: &str, timestamp: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let session_meta = json!({
        "timestamp": timestamp,
        "type": "session_meta",
        "payload": {
            "id": "session-id",
            "cwd": cwd,
            "model_provider": provider
        }
    });
    fs::write(
        path,
        format!("{session_meta}\n{{\"type\":\"event_msg\",\"payload\":{{}}}}\n"),
    )
    .unwrap();
}

fn create_state_db(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = Connection::open(path).unwrap();
    connection
        .execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT NOT NULL,
                cwd TEXT NOT NULL
            )",
            [],
        )
        .unwrap();
    for (id, provider, cwd) in [
        ("thread-api", "api", "E:\\Project\\api"),
        ("thread-openai", "openai", "D:\\Workspace\\openai"),
        ("thread-custom", "custom-provider", "F:\\Work\\custom"),
    ] {
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, cwd) VALUES (?1, ?2, ?3)",
                (id, provider, cwd),
            )
            .unwrap();
    }
}

#[test]
fn sync_session_provider_updates_rollout_meta_without_touching_cwd() {
    let sessions_dir = unique_sessions_dir("all");
    let api_file = sessions_dir
        .join("2026")
        .join("05")
        .join("07")
        .join("rollout-api.jsonl");
    let openai_file = sessions_dir
        .join("2026")
        .join("05")
        .join("06")
        .join("rollout-openai.jsonl");
    let ignored_file = sessions_dir
        .join("2026")
        .join("05")
        .join("07")
        .join("other.jsonl");
    write_rollout_file(&api_file, "api", "E:\\Project\\ai");
    write_rollout_file(&openai_file, "openai", "D:\\Workspace\\other");
    fs::write(&ignored_file, "{\"type\":\"session_meta\"}\n").unwrap();

    let updated = sync_codex_session_rollouts_to_provider(&sessions_dir, "openai").unwrap();

    let api_line = fs::read_to_string(&api_file)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let openai_line = fs::read_to_string(&openai_file)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let api_meta: Value = serde_json::from_str(&api_line).unwrap();
    let openai_meta: Value = serde_json::from_str(&openai_line).unwrap();

    fs::remove_dir_all(&sessions_dir).unwrap();

    assert_eq!(
        updated, 1,
        "only the rollout file with a different provider should be rewritten"
    );
    assert_eq!(api_meta["payload"]["model_provider"], "openai");
    assert_eq!(api_meta["payload"]["cwd"], "E:\\Project\\ai");
    assert_eq!(openai_meta["payload"]["model_provider"], "openai");
    assert_eq!(openai_meta["payload"]["cwd"], "D:\\Workspace\\other");
}

#[test]
fn sync_session_provider_adds_missing_rollout_provider() {
    let sessions_dir = unique_sessions_dir("missing");
    let path = sessions_dir.join("rollout-missing-provider.jsonl");
    fs::create_dir_all(&sessions_dir).unwrap();
    fs::write(
        &path,
        r#"{"type":"session_meta","payload":{"id":"session-id","cwd":"E:\\Project\\ai"}}"#,
    )
    .unwrap();

    let updated = sync_codex_session_rollouts_to_provider(&sessions_dir, "api").unwrap();
    let line = fs::read_to_string(&path).unwrap();
    let meta: Value = serde_json::from_str(&line).unwrap();

    fs::remove_dir_all(&sessions_dir).unwrap();

    assert_eq!(updated, 1);
    assert_eq!(meta["payload"]["model_provider"], "api");
    assert_eq!(meta["payload"]["cwd"], "E:\\Project\\ai");
}

#[test]
fn sync_rollout_provider_preserves_final_line_without_newline() {
    let sessions_dir = unique_sessions_dir("no-newline");
    let path = sessions_dir.join("rollout-no-newline.jsonl");
    fs::create_dir_all(&sessions_dir).unwrap();
    fs::write(
        &path,
        r#"{"type":"session_meta","payload":{"model_provider":"openai"}}"#,
    )
    .unwrap();

    let updated = sync_codex_session_rollouts_to_provider(&sessions_dir, "api").unwrap();
    let content = fs::read_to_string(&path).unwrap();
    let meta: Value = serde_json::from_str(&content).unwrap();

    fs::remove_dir_all(&sessions_dir).unwrap();

    assert_eq!(updated, 1);
    assert_eq!(meta["payload"]["model_provider"], "api");
    assert!(!content.ends_with('\n'));
}

#[test]
fn sync_rollout_provider_only_updates_latest_activity_files() {
    let sessions_dir = unique_sessions_dir("latest-limit");
    let mut paths = Vec::new();
    for index in 0..102 {
        let path = sessions_dir.join(format!("rollout-{index:03}.jsonl"));
        let timestamp = format!("2026-05-07T00:{:02}:{:02}.000Z", index / 60, index % 60);
        write_rollout_file_with_timestamp(&path, "openai", "E:\\Project\\ai", &timestamp);
        paths.push(path);
    }

    let updated = sync_codex_session_rollouts_to_provider(&sessions_dir, "api").unwrap();

    assert_eq!(updated, 100);
    for (index, path) in paths.iter().enumerate() {
        let line = fs::read_to_string(path)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_string();
        let meta: Value = serde_json::from_str(&line).unwrap();
        if index < 2 {
            assert_eq!(meta["payload"]["model_provider"], "openai");
        } else {
            assert_eq!(meta["payload"]["model_provider"], "api");
        }
    }

    fs::remove_dir_all(&sessions_dir).unwrap();
}

#[test]
fn sync_state_provider_updates_all_threads_without_touching_cwd() {
    let temp_dir = unique_sessions_dir("state-db");
    let state_db = temp_dir.join("state_5.sqlite");
    create_state_db(&state_db);

    let updated = sync_codex_state_threads_to_provider(&state_db, "api").unwrap();
    let connection = Connection::open(&state_db).unwrap();
    let mut rows = connection
        .prepare("SELECT id, model_provider, cwd FROM threads ORDER BY id")
        .unwrap();
    let threads = rows
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    drop(rows);
    drop(connection);
    fs::remove_dir_all(&temp_dir).unwrap();

    assert_eq!(updated, 2);
    assert_eq!(
        threads,
        vec![
            (
                "thread-api".to_string(),
                "api".to_string(),
                "E:\\Project\\api".to_string()
            ),
            (
                "thread-custom".to_string(),
                "api".to_string(),
                "F:\\Work\\custom".to_string()
            ),
            (
                "thread-openai".to_string(),
                "api".to_string(),
                "D:\\Workspace\\openai".to_string()
            ),
        ]
    );
}

#[test]
fn missing_state_db_is_ignored() {
    let temp_dir = unique_sessions_dir("missing-state-db");
    let state_db = temp_dir.join("state_5.sqlite");

    let updated = sync_codex_state_threads_to_provider(&state_db, "api").unwrap();

    assert_eq!(updated, 0);
}

#[test]
fn nested_model_provider_json_lines_are_rewritten() {
    let line = r#"{"type":"response_item","payload":{"model_provider":"openai"}}"#;

    let updated = update_rollout_provider_line(line, "api").unwrap();
    let line = updated.unwrap();
    let value: Value = serde_json::from_str(&line).unwrap();

    assert_eq!(value["payload"]["model_provider"], "api");
}

#[test]
fn json_lines_without_model_provider_are_not_rewritten() {
    let line = r#"{"type":"response_item","payload":{"text":"ok"}}"#;

    let updated = update_rollout_provider_line(line, "api").unwrap();

    assert_eq!(updated, None);
}

#[test]
fn malformed_json_lines_are_ignored() {
    let updated = update_rollout_provider_line("{", "api").unwrap();

    assert_eq!(updated, None);
}
