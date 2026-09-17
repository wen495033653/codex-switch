use super::*;
use std::{
    env,
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

    parse_session_line(&line, &mut parsed, None);

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
        usage_stats_get_for_scan_sources(&db_path, &sources, "2026-06-15T03:00:00Z").unwrap();

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
