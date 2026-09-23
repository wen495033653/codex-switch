use super::*;
use super::{db::*, model::*, parse::*, pricing::*, sources::*};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_root(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!("codex-switch-usage-stats-{name}-{stamp}"))
}

fn write_session(codex_home: &Path, day: &str, name: &str, lines: &[String]) -> PathBuf {
    let dir = codex_home
        .join("sessions")
        .join("2026")
        .join("06")
        .join(day);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.jsonl"));
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    path
}

fn session_meta_line(
    session_id: &str,
    provider: &str,
    timestamp: &str,
    model: Option<&str>,
) -> String {
    let mut payload = json!({
        "id": session_id,
        "model_provider": provider,
        "timestamp": timestamp
    });
    if let Some(model) = model {
        payload["model"] = json!(model);
    }
    json!({
        "timestamp": timestamp,
        "type": "session_meta",
        "payload": payload
    })
    .to_string()
}

fn turn_context_line(timestamp: &str, model: &str) -> String {
    json!({
        "timestamp": timestamp,
        "type": "turn_context",
        "payload": {
            "model": model
        }
    })
    .to_string()
}

fn token_count_line(
    timestamp: &str,
    input: u64,
    cached: u64,
    output: u64,
    reasoning: u64,
    total: u64,
    context_window: u64,
) -> String {
    json!({
        "timestamp": timestamp,
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "total_token_usage": {
                    "input_tokens": input,
                    "cached_input_tokens": cached,
                    "output_tokens": output,
                    "reasoning_output_tokens": reasoning,
                    "total_tokens": total
                },
                "model_context_window": context_window
            }
        }
    })
    .to_string()
}

fn set_stats_started_at(db_path: &Path, value: &str) {
    let connection = open_usage_connection(db_path, value).unwrap();
    connection
        .execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            params![value, META_STATS_STARTED_AT],
        )
        .unwrap();
}

fn scan_state_last_scanned_at(db_path: &Path, source_path: &Path) -> String {
    let connection = Connection::open(db_path).unwrap();
    connection
        .query_row(
            "SELECT last_scanned_at FROM session_scan_state WHERE source_path = ?1",
            [source_path.to_string_lossy().to_string()],
            |row| row.get(0),
        )
        .unwrap()
}

fn token_event_count(db_path: &Path, source_path: &Path) -> u64 {
    let connection = Connection::open(db_path).unwrap();
    connection
        .query_row(
            "SELECT COUNT(*) FROM session_token_events WHERE source_path = ?1",
            [source_path.to_string_lossy().to_string()],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| u64::try_from(value).unwrap())
        .unwrap()
}

fn window_total(response: &Value, owner_map: &str, owner_id: &str, window: &str) -> u64 {
    response
        .get(owner_map)
        .and_then(|map| map.get(owner_id))
        .and_then(|owner| owner.get(window))
        .and_then(|window| window.get("total_tokens"))
        .and_then(Value::as_u64)
        .unwrap()
}

fn model_window<'a>(
    response: &'a Value,
    owner_map: &str,
    owner_id: &str,
    window: &str,
    model: &str,
) -> &'a Value {
    response
        .get(owner_map)
        .and_then(|map| map.get(owner_id))
        .and_then(|owner| owner.get(window))
        .and_then(|window| window.get("by_model"))
        .and_then(|by_model| by_model.get(model))
        .unwrap()
}

fn write_instance_marker(instance_root: &Path, kind: &str, target_id: &str) {
    fs::create_dir_all(instance_root).unwrap();
    fs::write(
        instance_root.join(CODEX_APP_INSTANCE_MARKER_FILE),
        json!({
            "managedBy": "codex-switch",
            "kind": kind,
            "targetId": target_id,
            "instanceKey": format!("{kind}-{target_id}"),
            "channel": target_id
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn parses_total_token_usage_and_context_window() {
    let line = token_count_line("2026-06-15T01:00:00Z", 100, 25, 50, 20, 150, 258_400);
    let mut parsed = ParsedSession::default();

    parse_session_line(&line, &mut parsed);

    let usage = parsed.usage.unwrap();
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.cached_input_tokens, 25);
    assert_eq!(usage.output_tokens, 50);
    assert_eq!(usage.reasoning_output_tokens, 20);
    assert_eq!(usage.total_tokens, 150);
    assert_eq!(parsed.model_context_window, Some(258_400));
}

#[test]
fn repeated_token_count_keeps_cumulative_max_once() {
    let root = temp_root("duplicate");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    record_attribution_at(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        "2026-06-15T00:00:00Z",
    )
    .unwrap();
    write_session(
        &codex_home,
        "15",
        "rollout-duplicate",
        &[
            session_meta_line(
                "session-dup",
                PROVIDER_SUBSCRIPTION,
                "2026-06-15T01:00:00Z",
                Some("gpt-5.5"),
            ),
            token_count_line("2026-06-15T01:01:00Z", 60, 10, 40, 5, 100, 258_400),
            token_count_line("2026-06-15T01:02:00Z", 90, 20, 60, 10, 150, 258_400),
        ],
    );

    let response =
        usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();

    assert_eq!(
        window_total(&response, "subscriptions", "sub-a", "all"),
        150
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unchanged_session_uses_persistent_scan_cache_and_append_refreshes_it() {
    let root = temp_root("scan-cache");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    record_attribution_at(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        "2026-06-15T00:00:00Z",
    )
    .unwrap();
    let session_path = write_session(
        &codex_home,
        "15",
        "rollout-cache",
        &[
            session_meta_line(
                "session-cache",
                PROVIDER_SUBSCRIPTION,
                "2026-06-15T01:00:00Z",
                Some("gpt-5.5"),
            ),
            token_count_line("2026-06-15T01:01:00Z", 100, 0, 20, 5, 120, 258_400),
        ],
    );

    let first = usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();
    assert_eq!(window_total(&first, "subscriptions", "sub-a", "all"), 120);
    assert_eq!(token_event_count(&db_path, &session_path), 1);
    assert_eq!(
        scan_state_last_scanned_at(&db_path, &session_path),
        "2026-06-15T03:00:00Z"
    );

    let unchanged =
        usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T04:00:00Z").unwrap();
    assert_eq!(
        window_total(&unchanged, "subscriptions", "sub-a", "all"),
        120
    );
    assert_eq!(
        scan_state_last_scanned_at(&db_path, &session_path),
        "2026-06-15T03:00:00Z"
    );

    let mut lines = fs::read_to_string(&session_path).unwrap();
    lines.push_str(&token_count_line(
        "2026-06-15T04:01:00Z",
        200,
        0,
        40,
        10,
        240,
        258_400,
    ));
    lines.push('\n');
    fs::write(&session_path, lines).unwrap();

    let appended =
        usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T05:00:00Z").unwrap();
    assert_eq!(
        window_total(&appended, "subscriptions", "sub-a", "all"),
        240
    );
    assert_eq!(token_event_count(&db_path, &session_path), 2);
    assert_eq!(
        scan_state_last_scanned_at(&db_path, &session_path),
        "2026-06-15T05:00:00Z"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cached_session_windows_age_from_token_events_without_rescanning_file() {
    let root = temp_root("cached-windows");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    record_attribution_at(
        &db_path,
        OWNER_TYPE_API_PROFILE,
        "api-a",
        PROVIDER_API,
        "2026-06-15T00:00:00Z",
    )
    .unwrap();
    let session_path = write_session(
        &codex_home,
        "15",
        "rollout-window-cache",
        &[
            session_meta_line(
                "session-window-cache",
                PROVIDER_API,
                "2026-06-15T01:00:00Z",
                Some("gpt-5.5"),
            ),
            token_count_line("2026-06-15T01:01:00Z", 100, 0, 20, 5, 120, 258_400),
        ],
    );

    let first = usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();
    assert_eq!(window_total(&first, "api_profiles", "api-a", "today"), 120);

    let next_day =
        usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-16T03:00:00Z").unwrap();
    assert_eq!(window_total(&next_day, "api_profiles", "api-a", "today"), 0);
    assert_eq!(
        window_total(&next_day, "api_profiles", "api-a", "days_7"),
        120
    );
    assert_eq!(
        scan_state_last_scanned_at(&db_path, &session_path),
        "2026-06-15T03:00:00Z"
    );
    fs::remove_dir_all(root).unwrap();
}

fn cached_stats(
    db_path: &Path,
    codex_home: &Path,
    now: &str,
    cache: &mut Option<AggregateCache>,
) -> Value {
    usage_stats_get_for_scan_sources(
        db_path,
        &[sources::main_usage_scan_source(codex_home)],
        now,
        cache,
    )
    .unwrap()
}

#[test]
fn aggregation_cache_answers_an_unchanged_database_and_notices_new_data() {
    let root = temp_root("aggregate-cache");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    record_attribution_at(
        &db_path,
        OWNER_TYPE_API_PROFILE,
        "api-a",
        PROVIDER_API,
        "2026-06-15T00:00:00Z",
    )
    .unwrap();
    let session_path = write_session(
        &codex_home,
        "15",
        "rollout-aggregate-cache",
        &[
            session_meta_line(
                "session-aggregate-cache",
                PROVIDER_API,
                "2026-06-15T01:00:00Z",
                Some("gpt-5.5"),
            ),
            token_count_line("2026-06-15T01:01:00Z", 100, 0, 20, 5, 120, 258_400),
        ],
    );
    let mut cache = None;

    let first = cached_stats(&db_path, &codex_home, "2026-06-15T03:00:00Z", &mut cache);
    // An unchanged database half a minute later must be answered from the cache, not by scanning
    // every token event again. Emptying the table proves the second answer did not read it.
    Connection::open(&db_path)
        .unwrap()
        .execute("DELETE FROM session_token_events", [])
        .unwrap();
    let second = cached_stats(&db_path, &codex_home, "2026-06-15T03:00:30Z", &mut cache);
    assert_eq!(first, second);
    assert_eq!(window_total(&second, "api_profiles", "api-a", "today"), 120);

    // Appending to the session changes the database, so the cache must not be used.
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&session_path)
        .unwrap();
    use std::io::Write as _;
    writeln!(
        file,
        "{}",
        token_count_line("2026-06-15T03:01:00Z", 300, 0, 60, 15, 360, 258_400)
    )
    .unwrap();
    drop(file);
    let third = cached_stats(&db_path, &codex_home, "2026-06-15T03:02:00Z", &mut cache);
    assert_eq!(window_total(&third, "api_profiles", "api-a", "today"), 360);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn aggregation_cache_recomputes_when_an_event_leaves_a_window() {
    let root = temp_root("aggregate-cache-windows");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    record_attribution_at(
        &db_path,
        OWNER_TYPE_API_PROFILE,
        "api-a",
        PROVIDER_API,
        "2026-06-15T00:00:00Z",
    )
    .unwrap();
    write_session(
        &codex_home,
        "15",
        "rollout-aggregate-window",
        &[
            session_meta_line(
                "session-aggregate-window",
                PROVIDER_API,
                "2026-06-15T01:00:00Z",
                Some("gpt-5.5"),
            ),
            token_count_line("2026-06-15T01:01:00Z", 100, 0, 20, 5, 120, 258_400),
        ],
    );
    let mut cache = None;

    let inside = cached_stats(&db_path, &codex_home, "2026-06-22T01:00:30Z", &mut cache);
    assert_eq!(
        window_total(&inside, "api_profiles", "api-a", "days_7"),
        120
    );
    // Same day, unchanged database, one minute later: the only event is now older than seven days.
    let outside = cached_stats(&db_path, &codex_home, "2026-06-22T01:01:30Z", &mut cache);
    assert_eq!(window_total(&outside, "api_profiles", "api-a", "days_7"), 0);
    assert_eq!(
        window_total(&outside, "api_profiles", "api-a", "days_30"),
        120
    );

    // A new day moves the start of "today", which also invalidates the cache.
    let next_day = cached_stats(&db_path, &codex_home, "2026-06-23T01:01:30Z", &mut cache);
    assert_eq!(window_total(&next_day, "api_profiles", "api-a", "today"), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn skips_sessions_before_stats_started_at() {
    let root = temp_root("started-at");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    set_stats_started_at(&db_path, "2026-06-15T02:00:00Z");
    record_attribution_at(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        "2026-06-15T00:00:00Z",
    )
    .unwrap();
    write_session(
        &codex_home,
        "15",
        "rollout-old",
        &[
            session_meta_line(
                "session-old",
                PROVIDER_SUBSCRIPTION,
                "2026-06-15T01:00:00Z",
                Some("gpt-5.5"),
            ),
            token_count_line("2026-06-15T01:05:00Z", 100, 0, 20, 5, 120, 258_400),
        ],
    );

    let response =
        usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();

    assert!(response
        .get("subscriptions")
        .and_then(Value::as_object)
        .unwrap()
        .is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_attribution_is_not_assigned_to_any_card() {
    let root = temp_root("missing-attribution");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    set_stats_started_at(&db_path, "2026-06-15T00:00:00Z");
    write_session(
        &codex_home,
        "15",
        "rollout-no-owner",
        &[
            session_meta_line(
                "session-no-owner",
                PROVIDER_SUBSCRIPTION,
                "2026-06-15T01:00:00Z",
                Some("gpt-5.5"),
            ),
            token_count_line("2026-06-15T01:05:00Z", 100, 0, 20, 5, 120, 258_400),
        ],
    );

    let response =
        usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();

    assert!(response
        .get("subscriptions")
        .and_then(Value::as_object)
        .unwrap()
        .is_empty());
    assert!(!response
        .get("warnings")
        .and_then(Value::as_array)
        .unwrap()
        .is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn managed_instance_sessions_use_marker_attribution() {
    let root = temp_root("managed-instance");
    let db_path = root.join("usage.sqlite");
    let main_codex_home = root.join("codex");
    let instances_dir = root.join("codex-app-instances");
    let instance_root = instances_dir.join("api-cpa-plus");
    let instance_codex_home = instance_root.join("codex-home");
    set_stats_started_at(&db_path, "2026-06-15T00:00:00Z");
    write_instance_marker(&instance_root, "api", "cpa-plus");
    write_session(
        &instance_codex_home,
        "15",
        "rollout-instance",
        &[
            session_meta_line(
                "session-instance",
                PROVIDER_API,
                "2026-06-15T01:00:00Z",
                None,
            ),
            turn_context_line("2026-06-15T01:00:30Z", "gpt-5.5"),
            token_count_line("2026-06-15T01:05:00Z", 100, 0, 20, 5, 120, 258_400),
        ],
    );

    let mut sources = vec![main_usage_scan_source(&main_codex_home)];
    sources.extend(managed_instance_usage_scan_sources(&instances_dir).unwrap());
    let response =
        usage_stats_get_for_scan_sources(&db_path, &sources, "2026-06-15T03:00:00Z", &mut None)
            .unwrap();

    assert_eq!(
        window_total(&response, "api_profiles", "cpa-plus", "all"),
        120
    );
    assert!(response
        .get("warnings")
        .and_then(Value::as_array)
        .unwrap()
        .is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn today_usage_uses_token_count_delta_timestamp_not_session_start() {
    let root = temp_root("window-delta");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    set_stats_started_at(&db_path, "2026-06-14T00:00:00Z");
    record_attribution_at(
        &db_path,
        OWNER_TYPE_API_PROFILE,
        "api-a",
        PROVIDER_API,
        "2026-06-14T00:00:00Z",
    )
    .unwrap();
    write_session(
        &codex_home,
        "14",
        "rollout-cross-day",
        &[
            session_meta_line(
                "session-cross-day",
                PROVIDER_API,
                "2026-06-14T10:00:00Z",
                None,
            ),
            turn_context_line("2026-06-14T10:00:30Z", "gpt-5.5"),
            token_count_line("2026-06-14T12:00:00Z", 80, 0, 20, 5, 100, 258_400),
            token_count_line("2026-06-15T17:00:00Z", 240, 0, 60, 15, 300, 258_400),
        ],
    );

    let response =
        usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T18:00:00Z").unwrap();

    assert_eq!(window_total(&response, "api_profiles", "api-a", "all"), 300);
    assert_eq!(
        window_total(&response, "api_profiles", "api-a", "today"),
        200
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn aggregates_subscription_and_api_profile_owners_separately() {
    let root = temp_root("owners");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    record_attribution_at(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        "2026-06-15T00:00:00Z",
    )
    .unwrap();
    record_attribution_at(
        &db_path,
        OWNER_TYPE_API_PROFILE,
        "api-a",
        PROVIDER_API,
        "2026-06-15T00:00:00Z",
    )
    .unwrap();
    write_session(
        &codex_home,
        "15",
        "rollout-sub",
        &[
            session_meta_line(
                "session-sub",
                PROVIDER_SUBSCRIPTION,
                "2026-06-15T01:00:00Z",
                None,
            ),
            turn_context_line("2026-06-15T01:00:30Z", "gpt-5.5"),
            token_count_line("2026-06-15T01:05:00Z", 100, 0, 20, 5, 120, 258_400),
        ],
    );
    write_session(
        &codex_home,
        "15",
        "rollout-api",
        &[
            session_meta_line("session-api", PROVIDER_API, "2026-06-15T02:00:00Z", None),
            turn_context_line("2026-06-15T02:00:30Z", "gpt-5.4 mini"),
            token_count_line("2026-06-15T02:05:00Z", 200, 50, 40, 10, 240, 128_000),
        ],
    );

    let response =
        usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();

    assert_eq!(
        window_total(&response, "subscriptions", "sub-a", "all"),
        120
    );
    assert_eq!(window_total(&response, "api_profiles", "api-a", "all"), 240);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn aggregates_usage_by_model_inside_each_window() {
    let root = temp_root("by-model");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    record_attribution_at(
        &db_path,
        OWNER_TYPE_API_PROFILE,
        "api-a",
        PROVIDER_API,
        "2026-06-15T00:00:00Z",
    )
    .unwrap();
    write_session(
        &codex_home,
        "15",
        "rollout-model-a",
        &[
            session_meta_line(
                "session-model-a",
                PROVIDER_API,
                "2026-06-15T01:00:00Z",
                None,
            ),
            turn_context_line("2026-06-15T01:00:30Z", "gpt-5.5"),
            token_count_line("2026-06-15T01:05:00Z", 100, 0, 20, 5, 120, 258_400),
        ],
    );
    write_session(
        &codex_home,
        "15",
        "rollout-model-b",
        &[
            session_meta_line(
                "session-model-b",
                PROVIDER_API,
                "2026-06-15T02:00:00Z",
                None,
            ),
            turn_context_line("2026-06-15T02:00:30Z", "gpt-5.4 mini"),
            token_count_line("2026-06-15T02:05:00Z", 200, 50, 40, 10, 240, 128_000),
        ],
    );

    let response =
        usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();

    assert_eq!(window_total(&response, "api_profiles", "api-a", "all"), 360);
    assert_eq!(
        model_window(&response, "api_profiles", "api-a", "all", "gpt-5.5")
            .get("total_tokens")
            .and_then(Value::as_u64),
        Some(120)
    );
    assert_eq!(
        model_window(&response, "api_profiles", "api-a", "all", "gpt-5.4 mini")
            .get("total_tokens")
            .and_then(Value::as_u64),
        Some(240)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_scan_write_rolls_back_the_whole_refresh() {
    let root = temp_root("rollback");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    record_attribution_at(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        "2026-06-15T00:00:00Z",
    )
    .unwrap();
    let path_a = write_session(
        &codex_home,
        "15",
        "rollout-a",
        &[
            session_meta_line(
                "session-a",
                PROVIDER_SUBSCRIPTION,
                "2026-06-15T01:00:00Z",
                None,
            ),
            token_count_line("2026-06-15T01:01:00Z", 100, 0, 20, 5, 120, 258_400),
        ],
    );
    let mut cache = None;
    let first = cached_stats(&db_path, &codex_home, "2026-06-15T03:00:00Z", &mut cache);
    assert_eq!(window_total(&first, "subscriptions", "sub-a", "all"), 120);

    // Both files change; the scan state of the second one cannot be written.
    let mut file = fs::OpenOptions::new().append(true).open(&path_a).unwrap();
    use std::io::Write as _;
    writeln!(
        file,
        "{}",
        token_count_line("2026-06-15T02:01:00Z", 200, 0, 40, 10, 240, 258_400)
    )
    .unwrap();
    drop(file);
    let path_b = write_session(
        &codex_home,
        "15",
        "rollout-b",
        &[
            session_meta_line(
                "session-b",
                PROVIDER_SUBSCRIPTION,
                "2026-06-15T02:00:00Z",
                None,
            ),
            token_count_line("2026-06-15T02:05:00Z", 80, 0, 20, 0, 100, 258_400),
        ],
    );
    Connection::open(&db_path)
        .unwrap()
        .execute_batch(
            r#"
            CREATE TRIGGER fail_rollout_b BEFORE INSERT ON session_scan_state
            WHEN NEW.source_path LIKE '%rollout-b%'
            BEGIN SELECT RAISE(ABORT, 'forced failure'); END;
            "#,
        )
        .unwrap();
    let error = usage_stats_get_for_scan_sources(
        &db_path,
        &[sources::main_usage_scan_source(&codex_home)],
        "2026-06-15T03:01:00Z",
        &mut cache,
    )
    .unwrap_err();
    assert!(error.contains("forced failure"), "{error}");

    // Nothing of the failed refresh was committed, not even the writes for the first file.
    assert_eq!(token_event_count(&db_path, &path_a), 1);
    assert_eq!(
        scan_state_last_scanned_at(&db_path, &path_a),
        "2026-06-15T03:00:00Z"
    );
    assert_eq!(token_event_count(&db_path, &path_b), 0);

    // The next refresh redoes both files instead of reusing the summary cached before them.
    Connection::open(&db_path)
        .unwrap()
        .execute_batch("DROP TRIGGER fail_rollout_b")
        .unwrap();
    let recovered = cached_stats(&db_path, &codex_home, "2026-06-15T03:02:00Z", &mut cache);
    assert_eq!(
        window_total(&recovered, "subscriptions", "sub-a", "all"),
        340
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn opening_an_older_database_adds_the_missing_session_usage_columns() {
    let root = temp_root("schema-upgrade");
    fs::create_dir_all(&root).unwrap();
    let db_path = root.join("usage.sqlite");
    Connection::open(&db_path)
        .unwrap()
        .execute_batch(
            r#"
            CREATE TABLE session_usage (
                session_id TEXT PRIMARY KEY,
                source_path TEXT NOT NULL,
                owner_type TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                started_at TEXT NOT NULL,
                started_at_seconds INTEGER NOT NULL,
                updated_at TEXT NOT NULL,
                updated_at_seconds INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                reasoning_output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                model_context_window INTEGER,
                estimated_cost_usd REAL,
                priced INTEGER NOT NULL,
                last_scanned_at TEXT NOT NULL
            );
            "#,
        )
        .unwrap();

    open_usage_connection(&db_path, "2026-06-15T00:00:00Z").unwrap();
    // A second open finds every column present and changes nothing.
    let connection = open_usage_connection(&db_path, "2026-06-15T00:00:00Z").unwrap();

    let columns: Vec<String> = connection
        .prepare("PRAGMA table_info(session_usage)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut expected = vec!["pricing_context".to_string(), "unpriced_reason".to_string()];
    for prefix in ["today", "days_7", "days_30"] {
        for name in [
            "input_tokens",
            "cached_input_tokens",
            "output_tokens",
            "reasoning_output_tokens",
            "total_tokens",
        ] {
            expected.push(format!("{prefix}_{name}"));
        }
    }
    assert_eq!(columns[19..], expected[..]);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

fn relative_source_path(root: &Path, source_path: &str) -> String {
    Path::new(source_path)
        .strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

/// Everything a scan leaves in the database except file stamps (they depend on the clock) and
/// the unused `today_*`/`days_7_*`/`days_30_*` columns of `session_usage`.
fn database_snapshot(db_path: &Path, root: &Path) -> Value {
    let connection = Connection::open(db_path).unwrap();
    let mut sessions = connection
        .prepare(
            r#"
            SELECT session_id, source_path, owner_type, owner_id, provider, model,
                   started_at, started_at_seconds, updated_at, updated_at_seconds,
                   input_tokens, cached_input_tokens, output_tokens,
                   reasoning_output_tokens, total_tokens, model_context_window,
                   estimated_cost_usd, priced, pricing_context, unpriced_reason, last_scanned_at
            FROM session_usage ORDER BY session_id
            "#,
        )
        .unwrap();
    let sessions: Vec<Value> = sessions
        .query_map([], |row| {
            Ok(json!({
                "session_id": row.get::<_, String>(0)?,
                "source_path": relative_source_path(root, &row.get::<_, String>(1)?),
                "owner_type": row.get::<_, String>(2)?,
                "owner_id": row.get::<_, String>(3)?,
                "provider": row.get::<_, String>(4)?,
                "model": row.get::<_, String>(5)?,
                "started_at": row.get::<_, String>(6)?,
                "started_at_seconds": row.get::<_, i64>(7)?,
                "updated_at": row.get::<_, String>(8)?,
                "updated_at_seconds": row.get::<_, i64>(9)?,
                "tokens": [
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, i64>(14)?
                ],
                "model_context_window": row.get::<_, Option<i64>>(15)?,
                "estimated_cost_usd": row.get::<_, Option<f64>>(16)?,
                "priced": row.get::<_, i64>(17)?,
                "pricing_context": row.get::<_, Option<String>>(18)?,
                "unpriced_reason": row.get::<_, Option<String>>(19)?,
                "last_scanned_at": row.get::<_, String>(20)?
            }))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut events = connection
        .prepare(
            r#"
            SELECT source_path, event_index, timestamp_seconds, input_tokens,
                   cached_input_tokens, output_tokens, reasoning_output_tokens, total_tokens
            FROM session_token_events ORDER BY source_path, event_index
            "#,
        )
        .unwrap();
    let events: Vec<Value> = events
        .query_map([], |row| {
            Ok(json!([
                relative_source_path(root, &row.get::<_, String>(0)?),
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?
            ]))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut scan_states = connection
        .prepare(
            r#"
            SELECT source_path, file_size, scan_scope, session_id, outcome, last_scanned_at
            FROM session_scan_state ORDER BY source_path
            "#,
        )
        .unwrap();
    let scan_states: Vec<Value> = scan_states
        .query_map([], |row| {
            Ok(json!([
                relative_source_path(root, &row.get::<_, String>(0)?),
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?
            ]))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let pricing_version: String = connection
        .query_row(
            "SELECT value FROM meta WHERE key = 'pricing_updated_at'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    json!({
        "session_usage": sessions,
        "session_token_events": events,
        "session_scan_state": scan_states,
        "pricing_updated_at": pricing_version
    })
}

fn write_session_file(path: &Path, lines: &[String]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("{}\n", lines.join("\n"))).unwrap();
}

/// A fixture that walks every scan outcome: owners switching mid-way, a managed instance,
/// short and long context pricing, an unpriced model, a counter reset, a duplicate session id,
/// a session from before the statistics started, one without attribution, one without
/// session_meta, and events that fall outside the 7 and 30 day windows. "Today" events sit
/// within an hour before "now" (12:00Z) and all others at least 25 hours earlier, so the
/// result does not depend on the local UTC offset between -11 and +11 hours.
///
/// The recorded baseline in `testdata/scan_baseline.json` was produced by the scan as of
/// commit 347da53, before the scan writes moved into one transaction and before the
/// `session_usage` window columns stopped being written. It only changes when the summary is
/// meant to change.
#[test]
fn scan_fixture_summary_matches_recorded_baseline() {
    let root = temp_root("baseline");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions").join("2026");
    let instances_dir = root.join("codex-app-instances");
    let instance_root = instances_dir.join("api-cpa-plus");
    set_stats_started_at(&db_path, "2026-06-05T00:00:00Z");
    for (owner_type, owner_id, provider, started_at) in [
        (
            OWNER_TYPE_API_PROFILE,
            "api-early",
            PROVIDER_API,
            "2026-04-01T00:00:00Z",
        ),
        (
            OWNER_TYPE_SUBSCRIPTION,
            "sub-a",
            PROVIDER_SUBSCRIPTION,
            "2026-06-05T00:00:00Z",
        ),
        (
            OWNER_TYPE_API_PROFILE,
            "api-a",
            PROVIDER_API,
            "2026-06-10T00:00:00Z",
        ),
        (
            OWNER_TYPE_SUBSCRIPTION,
            "sub-b",
            PROVIDER_SUBSCRIPTION,
            "2026-06-14T12:00:00Z",
        ),
    ] {
        record_attribution_at(&db_path, owner_type, owner_id, provider, started_at).unwrap();
    }
    write_instance_marker(&instance_root, "api", "cpa-plus");

    // sub-a, short context gpt-5.5, one event eight days ago and one yesterday.
    write_session_file(
        &sessions.join("06").join("07").join("rollout-s1.jsonl"),
        &[
            session_meta_line(
                "s1",
                PROVIDER_SUBSCRIPTION,
                "2026-06-07T10:00:00Z",
                Some("gpt-5.5"),
            ),
            token_count_line("2026-06-07T10:05:00Z", 100, 20, 30, 5, 130, 258_400),
            token_count_line("2026-06-14T09:00:00Z", 400, 120, 90, 15, 490, 258_400),
        ],
    );
    // sub-b after the switch, model from turn_context, events today.
    let s2_path = sessions.join("06").join("15").join("rollout-s2.jsonl");
    write_session_file(
        &s2_path,
        &[
            session_meta_line("s2", PROVIDER_SUBSCRIPTION, "2026-06-15T11:00:00Z", None),
            turn_context_line("2026-06-15T11:00:10Z", "gpt-5.4 mini"),
            token_count_line("2026-06-15T11:05:00Z", 200, 50, 40, 10, 240, 128_000),
            token_count_line("2026-06-15T11:40:00Z", 500, 100, 80, 20, 580, 128_000),
        ],
    );
    // api-a, long context gpt-5.5, with a counter reset in the middle.
    write_session_file(
        &sessions.join("06").join("13").join("rollout-s3.jsonl"),
        &[
            session_meta_line("s3", PROVIDER_API, "2026-06-13T08:00:00Z", Some("gpt-5.5")),
            token_count_line("2026-06-13T08:10:00Z", 1_000, 200, 300, 50, 1_300, 300_000),
            token_count_line("2026-06-13T09:10:00Z", 400, 100, 100, 10, 500, 300_000),
            token_count_line("2026-06-14T10:00:00Z", 900, 300, 200, 30, 1_100, 300_000),
        ],
    );
    // api-a, a model without a price.
    write_session_file(
        &sessions.join("06").join("15").join("rollout-s4.jsonl"),
        &[
            session_meta_line(
                "s4",
                PROVIDER_API,
                "2026-06-15T11:10:00Z",
                Some("custom-model"),
            ),
            token_count_line("2026-06-15T11:20:00Z", 70, 0, 30, 0, 100, 64_000),
        ],
    );
    // A second file with s2's id, sorted after it: counted once, as a duplicate.
    write_session_file(
        &sessions
            .join("06")
            .join("15")
            .join("rollout-s2x-copy.jsonl"),
        &[
            session_meta_line("s2", PROVIDER_SUBSCRIPTION, "2026-06-15T11:00:00Z", None),
            token_count_line("2026-06-15T11:05:00Z", 999, 0, 1, 0, 1_000, 128_000),
        ],
    );
    // An archived session of sub-a.
    write_session_file(
        &codex_home
            .join("archived_sessions")
            .join("rollout-s5.jsonl"),
        &[
            session_meta_line(
                "s5",
                PROVIDER_SUBSCRIPTION,
                "2026-06-09T07:00:00Z",
                Some("gpt-5.5"),
            ),
            token_count_line("2026-06-09T07:30:00Z", 800, 600, 50, 10, 850, 258_400),
        ],
    );
    // Entirely before the statistics started.
    write_session_file(
        &sessions.join("06").join("01").join("rollout-s6.jsonl"),
        &[
            session_meta_line("s6", PROVIDER_API, "2026-06-01T08:00:00Z", Some("gpt-5.5")),
            token_count_line("2026-06-02T08:00:00Z", 100, 0, 20, 0, 120, 258_400),
        ],
    );
    // A provider nobody switched to.
    write_session_file(
        &sessions.join("06").join("14").join("rollout-s7.jsonl"),
        &[
            session_meta_line(
                "s7",
                "other-provider",
                "2026-06-14T08:00:00Z",
                Some("gpt-5.5"),
            ),
            token_count_line("2026-06-14T08:10:00Z", 100, 0, 20, 0, 120, 258_400),
        ],
    );
    // No session_meta line.
    write_session_file(
        &sessions.join("06").join("14").join("rollout-s8.jsonl"),
        &[token_count_line(
            "2026-06-14T08:10:00Z",
            100,
            0,
            20,
            0,
            120,
            258_400,
        )],
    );
    // Started before the statistics, still active after; one event older than 30 days.
    write_session_file(
        &sessions.join("05").join("01").join("rollout-s10.jsonl"),
        &[
            session_meta_line("s10", PROVIDER_API, "2026-05-01T08:00:00Z", None),
            turn_context_line("2026-05-01T08:00:10Z", "gpt-5.6"),
            token_count_line("2026-05-01T08:10:00Z", 1_000, 400, 100, 0, 1_100, 128_000),
            token_count_line(
                "2026-06-12T08:10:00Z",
                3_000,
                1_400,
                300,
                40,
                3_300,
                128_000,
            ),
        ],
    );
    // Managed instance: attributed by its marker.
    write_session_file(
        &instance_root
            .join("codex-home")
            .join("sessions")
            .join("2026")
            .join("06")
            .join("15")
            .join("rollout-s11.jsonl"),
        &[
            session_meta_line("s11", PROVIDER_API, "2026-06-15T11:15:00Z", None),
            turn_context_line("2026-06-15T11:15:10Z", "gpt-5.4-mini"),
            token_count_line("2026-06-15T11:30:00Z", 600, 200, 60, 6, 660, 128_000),
        ],
    );

    let sources = || {
        let mut sources = vec![main_usage_scan_source(&codex_home)];
        sources.extend(managed_instance_usage_scan_sources(&instances_dir).unwrap());
        sources
    };
    let mut cache = None;
    let mut run = |now: &str| {
        usage_stats_get_for_scan_sources(&db_path, &sources(), now, &mut cache).unwrap()
    };

    let first = run("2026-06-15T12:00:00Z");
    let mut file = fs::OpenOptions::new().append(true).open(&s2_path).unwrap();
    use std::io::Write as _;
    writeln!(
        file,
        "{}",
        token_count_line("2026-06-15T12:10:00Z", 700, 150, 100, 30, 830, 128_000)
    )
    .unwrap();
    drop(file);
    write_session_file(
        &sessions.join("06").join("15").join("rollout-s12.jsonl"),
        &[
            session_meta_line("s12", PROVIDER_SUBSCRIPTION, "2026-06-15T12:15:00Z", None),
            turn_context_line("2026-06-15T12:15:10Z", "gpt-5.6-luna"),
            token_count_line("2026-06-15T12:20:00Z", 300, 0, 50, 5, 350, 128_000),
        ],
    );
    let appended = run("2026-06-15T12:30:00Z");
    let unchanged = run("2026-06-15T12:30:30Z");
    let next_day = run("2026-06-16T12:00:00Z");
    // Neither of the last two runs writes, so this is also the state after the append.
    let actual = json!({
        "first": first,
        "appended": appended,
        "next_day": next_day,
        "database": database_snapshot(&db_path, &root)
    });
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(unchanged, appended);
    let expected: Value =
        serde_json::from_str(include_str!("testdata/scan_baseline.json")).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn cost_formula_uses_cached_input_and_output_prices() {
    let usage = TokenUsage {
        input_tokens: 1_000_000,
        cached_input_tokens: 250_000,
        output_tokens: 100_000,
        reasoning_output_tokens: 25_000,
        total_tokens: 1_100_000,
    };

    let estimate = estimate_cost("gpt-5.4-mini", &usage, Some(128_000));

    assert!(estimate.priced);
    let expected = 0.75 * 0.75 + 0.25 * 0.075 + 0.1 * 4.5;
    assert!((estimate.cost_usd.unwrap() - expected).abs() < 0.000_001);
}

#[test]
fn gpt_5_6_alias_uses_sol_pricing() {
    let usage = TokenUsage {
        input_tokens: 1_000_000,
        cached_input_tokens: 200_000,
        output_tokens: 100_000,
        reasoning_output_tokens: 0,
        total_tokens: 1_100_000,
    };

    let estimate = estimate_cost("gpt-5.6", &usage, Some(128_000));

    assert!(estimate.priced);
    let expected = 0.8 * 5.0 + 0.2 * 0.5 + 0.1 * 30.0;
    assert!((estimate.cost_usd.unwrap() - expected).abs() < 0.000_001);
}

#[test]
fn gpt_5_6_variants_use_their_own_prices() {
    let usage = TokenUsage {
        input_tokens: 1_000_000,
        cached_input_tokens: 0,
        output_tokens: 1_000_000,
        reasoning_output_tokens: 0,
        total_tokens: 2_000_000,
    };

    let terra = estimate_cost("gpt-5.6-terra", &usage, Some(128_000));
    let luna = estimate_cost("gpt-5.6-luna", &usage, Some(128_000));

    assert_eq!(terra.cost_usd, Some(17.5));
    assert_eq!(luna.cost_usd, Some(7.0));
}

#[test]
fn cumulative_input_above_threshold_still_uses_short_context_price() {
    let usage = TokenUsage {
        input_tokens: 753_341,
        cached_input_tokens: 661_376,
        output_tokens: 5_386,
        reasoning_output_tokens: 2_117,
        total_tokens: 758_727,
    };

    let estimate = estimate_cost("gpt-5.5", &usage, Some(258_400));

    assert!(estimate.priced);
    assert_eq!(
        estimate.pricing_context,
        Some(PRICING_CONTEXT_STANDARD_SHORT)
    );
    let expected = per_million_cost(91_965, 5.0)
        + per_million_cost(661_376, 0.5)
        + per_million_cost(5_386, 30.0);
    assert!((estimate.cost_usd.unwrap() - expected).abs() < 0.000_001);
}

#[test]
fn long_context_uses_standard_long_context_prices() {
    let usage = TokenUsage {
        input_tokens: 300_000,
        cached_input_tokens: 100_000,
        output_tokens: 10_000,
        reasoning_output_tokens: 1_000,
        total_tokens: 310_000,
    };

    let estimate = estimate_cost("gpt-5.5", &usage, Some(LONG_CONTEXT_THRESHOLD_TOKENS));

    assert!(estimate.priced);
    assert_eq!(
        estimate.pricing_context,
        Some(PRICING_CONTEXT_STANDARD_LONG)
    );
    let expected = per_million_cost(200_000, 10.0)
        + per_million_cost(100_000, 1.0)
        + per_million_cost(10_000, 45.0);
    assert!((estimate.cost_usd.unwrap() - expected).abs() < 0.000_001);
}

#[test]
fn missing_price_is_tokens_only() {
    let usage = TokenUsage {
        input_tokens: 100,
        cached_input_tokens: 0,
        output_tokens: 20,
        reasoning_output_tokens: 5,
        total_tokens: 120,
    };

    let estimate = estimate_cost("unknown-model", &usage, Some(128_000));

    assert!(!estimate.priced);
    assert!(estimate.cost_usd.is_none());
    assert_eq!(
        estimate.unpriced_reason,
        Some(UNPRICED_REASON_MISSING_MODEL_PRICE)
    );
}
