use super::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{Duration as StdDuration, SystemTime},
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
                rollout_path TEXT NOT NULL,
                model_provider TEXT NOT NULL,
                cwd TEXT NOT NULL
            )",
            [],
        )
        .unwrap();
    for (id, provider, cwd, rollout_path) in [
        (
            "thread-api",
            "api",
            "E:\\Project\\api",
            "E:\\Project\\api\\rollout-api.jsonl",
        ),
        (
            "thread-openai",
            "openai",
            "D:\\Workspace\\openai",
            "D:\\Workspace\\openai\\rollout-openai.jsonl",
        ),
        (
            "thread-custom",
            "custom-provider",
            "F:\\Work\\custom",
            "F:\\Work\\custom\\rollout-custom.jsonl",
        ),
    ] {
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, cwd, rollout_path) VALUES (?1, ?2, ?3, ?4)",
                (id, provider, cwd, rollout_path),
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
fn sync_session_provider_updates_sessions_and_archived_sessions() {
    let root = unique_sessions_dir("sessions-and-archived");
    let sessions_dir = root.join("sessions");
    let archived_dir = root.join("archived_sessions");
    let sessions_file = sessions_dir.join("rollout-active.jsonl");
    let archived_file = archived_dir.join("rollout-archived.jsonl");
    write_rollout_file(&sessions_file, "openai", "E:\\Project\\active");
    write_rollout_file(&archived_file, "openai", "E:\\Project\\archived");

    let updated = sync_codex_session_rollout_dirs_to_provider(
        &[sessions_dir, archived_dir, root.join("missing_sessions")],
        "api",
        &[],
    )
    .unwrap();

    let sessions_line = fs::read_to_string(&sessions_file)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let archived_line = fs::read_to_string(&archived_file)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let sessions_meta: Value = serde_json::from_str(&sessions_line).unwrap();
    let archived_meta: Value = serde_json::from_str(&archived_line).unwrap();

    fs::remove_dir_all(&root).unwrap();

    assert_eq!(updated, 2);
    assert_eq!(sessions_meta["payload"]["model_provider"], "api");
    assert_eq!(sessions_meta["payload"]["cwd"], "E:\\Project\\active");
    assert_eq!(archived_meta["payload"]["model_provider"], "api");
    assert_eq!(archived_meta["payload"]["cwd"], "E:\\Project\\archived");
}

#[test]
fn sync_uses_combined_activity_time_limit() {
    let root = unique_sessions_dir("combined-activity-limit");
    let sessions_dir = root.join("sessions");
    let archived_dir = root.join("archived_sessions");
    let mut paths = Vec::new();
    let count = SESSION_SYNC_RECENT_ROLLOUT_LIMIT + 4;
    let base_time = 1_700_000_000u64;

    for index in 0..count {
        let dir = if index % 2 == 0 {
            &sessions_dir
        } else {
            &archived_dir
        };
        let path = dir.join(format!("rollout-{index:03}.jsonl"));
        let timestamp = format!("2026-05-07T00:{:02}:{:02}.000Z", index / 60, index % 60);
        write_rollout_file_with_timestamp(&path, "openai", "E:\\Project\\ai", &timestamp);
        let modified = SystemTime::UNIX_EPOCH
            + StdDuration::from_secs(base_time + count as u64 - index as u64);
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        paths.push((index, path));
    }

    let updated =
        sync_codex_session_rollout_dirs_to_provider(&[sessions_dir, archived_dir], "api", &[])
            .unwrap();

    assert_eq!(updated, SESSION_SYNC_RECENT_ROLLOUT_LIMIT);
    for (index, path) in paths {
        let line = fs::read_to_string(path)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_string();
        let meta: Value = serde_json::from_str(&line).unwrap();
        if index < 4 {
            assert_eq!(meta["payload"]["model_provider"], "openai");
        } else {
            assert_eq!(meta["payload"]["model_provider"], "api");
        }
    }

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn sync_always_updates_pinned_rollouts() {
    let root = unique_sessions_dir("pinned-rollouts");
    let sessions_dir = root.join("sessions");
    let archived_dir = root.join("archived_sessions");
    let pinned_path = sessions_dir.join("rollout-pinned.jsonl");
    let state_db = root.join("state_5.sqlite");
    let global_state = root.join(GLOBAL_STATE_FILE_NAME);
    let count = SESSION_SYNC_RECENT_ROLLOUT_LIMIT + 4;

    write_rollout_file_with_timestamp(
        &pinned_path,
        "openai",
        "E:\\Project\\pinned",
        "2026-05-01T00:00:00.000Z",
    );
    for index in 0..count {
        let dir = if index % 2 == 0 {
            &sessions_dir
        } else {
            &archived_dir
        };
        let path = dir.join(format!("rollout-{index:03}.jsonl"));
        let timestamp = format!("2026-05-07T00:{:02}:{:02}.000Z", index / 60, index % 60);
        write_rollout_file_with_timestamp(&path, "openai", "E:\\Project\\ai", &timestamp);
    }

    fs::create_dir_all(state_db.parent().unwrap()).unwrap();
    let connection = Connection::open(&state_db).unwrap();
    connection
        .execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL
            )",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO threads (id, rollout_path) VALUES (?1, ?2)",
            ("pinned-thread", pinned_path.to_string_lossy().to_string()),
        )
        .unwrap();
    drop(connection);
    fs::write(
        &global_state,
        json!({ "pinned-thread-ids": ["pinned-thread"] }).to_string(),
    )
    .unwrap();

    let pinned_rollouts = pinned_thread_rollout_paths(&global_state, &state_db).unwrap();
    let updated = sync_codex_session_rollout_dirs_to_provider(
        &[sessions_dir, archived_dir],
        "api",
        &pinned_rollouts,
    )
    .unwrap();

    let pinned_line = fs::read_to_string(&pinned_path)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let pinned_meta: Value = serde_json::from_str(&pinned_line).unwrap();

    fs::remove_dir_all(&root).unwrap();

    assert_eq!(pinned_rollouts, vec![pinned_path]);
    assert_eq!(updated, SESSION_SYNC_RECENT_ROLLOUT_LIMIT + 1);
    assert_eq!(pinned_meta["payload"]["model_provider"], "api");
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
fn sync_rollout_provider_preserves_modified_time() {
    let sessions_dir = unique_sessions_dir("mtime");
    let path = sessions_dir.join("rollout-mtime.jsonl");
    write_rollout_file(&path, "api", "E:\\Project\\ai");
    let original_modified = SystemTime::UNIX_EPOCH + StdDuration::from_secs(1_700_000_000);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original_modified)
        .unwrap();

    let updated = sync_codex_session_rollouts_to_provider(&sessions_dir, "openai").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let line = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let meta: Value = serde_json::from_str(&line).unwrap();

    fs::remove_dir_all(&sessions_dir).unwrap();

    let modified_delta = modified
        .duration_since(original_modified)
        .or_else(|_| original_modified.duration_since(modified))
        .unwrap();
    assert_eq!(updated, 1);
    assert_eq!(meta["payload"]["model_provider"], "openai");
    assert!(
        modified_delta < StdDuration::from_secs(1),
        "modified time should be preserved, delta was {modified_delta:?}"
    );
}

#[test]
fn sync_rollout_provider_only_updates_latest_activity_files() {
    let sessions_dir = unique_sessions_dir("latest-limit");
    let mut paths = Vec::new();
    for index in 0..(SESSION_SYNC_RECENT_ROLLOUT_LIMIT + 2) {
        let path = sessions_dir.join(format!("rollout-{index:03}.jsonl"));
        let timestamp = format!("2026-05-07T00:{:02}:{:02}.000Z", index / 60, index % 60);
        write_rollout_file_with_timestamp(&path, "openai", "E:\\Project\\ai", &timestamp);
        paths.push(path);
    }

    let updated = sync_codex_session_rollouts_to_provider(&sessions_dir, "api").unwrap();

    assert_eq!(updated, SESSION_SYNC_RECENT_ROLLOUT_LIMIT);
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
