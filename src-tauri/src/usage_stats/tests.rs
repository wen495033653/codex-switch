use super::*;
use super::{aggregate::usage_window_starts, db::*, model::*, pricing::*, records::*, sources::*};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    env, fs,
    io::Write as _,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const T0: &str = "2026-06-15T00:00:00Z";

fn temp_root(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!("codex-switch-usage-stats-{name}-{stamp}"))
}

fn write_rollout(codex_home: &Path, name: &str, lines: &[String]) -> PathBuf {
    let dir = codex_home
        .join("sessions")
        .join("2026")
        .join("06")
        .join("15");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("rollout-{name}.jsonl"));
    fs::write(
        &path,
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )
    .unwrap();
    path
}

fn append_lines(path: &Path, lines: &[String]) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}

fn session_meta_line(timestamp: &str, thread_id: &str, provider: &str) -> String {
    json!({
        "timestamp": timestamp,
        "type": "session_meta",
        "payload": { "id": thread_id, "model_provider": provider, "timestamp": timestamp }
    })
    .to_string()
}

fn turn_context_line(timestamp: &str, turn_id: &str, model: &str) -> String {
    json!({
        "timestamp": timestamp,
        "type": "turn_context",
        "payload": { "turn_id": turn_id, "model": model }
    })
    .to_string()
}

fn thread_settings_line(timestamp: &str, model: &str, provider: &str, tier: Value) -> String {
    json!({
        "timestamp": timestamp,
        "type": "event_msg",
        "payload": {
            "type": "thread_settings_applied",
            "thread_settings": {
                "model": model,
                "model_provider_id": provider,
                "service_tier": tier
            }
        }
    })
    .to_string()
}

fn record_line(
    timestamp: &str,
    response_id: &str,
    thread_id: &str,
    input: u64,
    cached: u64,
    output: u64,
) -> String {
    json!({
        "timestamp": timestamp,
        "type": "token_usage_record",
        "payload": {
            "thread_id": thread_id,
            "turn_id": "turn",
            "session_id": thread_id,
            "response_id": response_id,
            "usage": {
                "input_tokens": input,
                "cached_input_tokens": cached,
                "cache_write_input_tokens": 0,
                "output_tokens": output,
                "reasoning_output_tokens": output / 2,
                "total_tokens": input + output
            }
        }
    })
    .to_string()
}

fn subscription_session(timestamp: &str, thread_id: &str, model: &str) -> Vec<String> {
    vec![
        session_meta_line(timestamp, thread_id, PROVIDER_SUBSCRIPTION),
        turn_context_line(timestamp, "turn-1", model),
    ]
}

fn stats(db_path: &Path, codex_home: &Path, now: &str) -> Value {
    usage_stats_get_for_scan_sources(db_path, &[main_usage_scan_source(codex_home)], now).unwrap()
}

fn attribute(db_path: &Path, owner_type: &str, owner_id: &str, provider: &str, at: &str) {
    record_attribution_at(db_path, owner_type, owner_id, provider, at).unwrap();
}

fn window<'a>(response: &'a Value, owner_map: &str, owner_id: &str, name: &str) -> &'a Value {
    response
        .get(owner_map)
        .and_then(|map| map.get(owner_id))
        .and_then(|owner| owner.get(name))
        .unwrap_or_else(|| panic!("missing {owner_map}.{owner_id}.{name} in {response}"))
}

fn total_tokens(response: &Value, owner_map: &str, owner_id: &str, name: &str) -> u64 {
    window(response, owner_map, owner_id, name)["total_tokens"]
        .as_u64()
        .unwrap()
}

fn record_count(db_path: &Path) -> i64 {
    Connection::open(db_path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM usage_records", [], |row| row.get(0))
        .unwrap()
}

fn usage(input: u64, cached: u64, cache_write: u64, output: u64) -> TokenUsage {
    TokenUsage {
        input_tokens: input,
        cached_input_tokens: cached,
        cache_write_input_tokens: cache_write,
        output_tokens: output,
        reasoning_output_tokens: 0,
        total_tokens: input + output,
    }
}

fn assert_cost(cost: &EstimatedCost, expected_usd: f64, label: &str) {
    assert_eq!(cost.price_label, label);
    let actual = cost.cost_usd.expect("priced");
    assert!(
        (actual - expected_usd).abs() < 1e-9,
        "expected {expected_usd}, got {actual}"
    );
}

#[test]
fn records_carry_the_settings_in_effect_and_leave_an_unfinished_line() {
    let root = temp_root("reader");
    let codex_home = root.join("codex");
    let path = write_rollout(
        &codex_home,
        "reader",
        &[
            session_meta_line("2026-06-15T01:00:00Z", "thread-a", "openai"),
            thread_settings_line(
                "2026-06-15T01:00:01Z",
                "gpt-6-astra",
                "openai",
                json!("priority"),
            ),
            // A compaction request is recorded before its turn's turn_context.
            record_line(
                "2026-06-15T01:00:02Z",
                "resp-compact",
                "thread-a",
                300,
                0,
                10,
            ),
            turn_context_line("2026-06-15T01:00:03Z", "turn-2", "gpt-5.6-sol"),
            record_line("2026-06-15T01:00:04Z", "resp-1", "thread-a", 100, 40, 20),
        ],
    );
    let unfinished = record_line("2026-06-15T01:00:05Z", "resp-2", "thread-a", 50, 0, 5);
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&unfinished.as_bytes()[..30])
        .unwrap();

    let first = read_new_records(&path, None).unwrap();
    let summary = first
        .records
        .iter()
        .map(|record| {
            (
                record.response_id.as_str(),
                record.model.as_str(),
                record.provider.as_str(),
                record.service_tier.as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        summary,
        vec![
            ("resp-compact", "gpt-6-astra", "openai", "priority"),
            ("resp-1", "gpt-5.6-sol", "openai", "priority"),
        ]
    );
    assert_eq!(
        first.records[1].usage,
        TokenUsage {
            input_tokens: 100,
            cached_input_tokens: 40,
            cache_write_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 10,
            total_tokens: 120,
        }
    );
    assert!(first.skipped.is_empty());
    let complete_len = fs::metadata(&path).unwrap().len() - 30;
    assert_eq!(first.cursor.offset, complete_len);

    // The rest of the line arrives, then the tier goes back to the default (null).
    append_lines(
        &path,
        &[
            unfinished[30..].to_string(),
            thread_settings_line("2026-06-15T01:00:06Z", "gpt-5.6-sol", "openai", Value::Null),
            record_line("2026-06-15T01:00:07Z", "resp-3", "thread-a", 10, 0, 1),
        ],
    );
    let second = read_new_records(&path, Some(&first.cursor)).unwrap();
    assert!(!second.restarted);
    let tiers = second
        .records
        .iter()
        .map(|record| (record.response_id.as_str(), record.service_tier.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(tiers, vec![("resp-2", "priority"), ("resp-3", "")]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unreadable_record_line_is_reported_and_reading_continues() {
    let root = temp_root("bad-line");
    let codex_home = root.join("codex");
    let path = write_rollout(
        &codex_home,
        "bad",
        &[
            session_meta_line("2026-06-15T01:00:00Z", "thread-a", "openai"),
            json!({ "timestamp": "2026-06-15T01:00:01Z", "type": "token_usage_record", "payload": {} })
                .to_string(),
            record_line("2026-06-15T01:00:02Z", "resp-1", "thread-a", 10, 0, 1),
        ],
    );
    let read = read_new_records(&path, None).unwrap();
    assert_eq!(read.records.len(), 1);
    assert_eq!(read.skipped.len(), 1);
    assert!(
        read.skipped[0].reason.contains("response_id"),
        "{:?}",
        read.skipped
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn appended_records_are_read_once_and_a_rewritten_file_is_not_counted_twice() {
    let root = temp_root("incremental");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    attribute(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        T0,
    );
    let mut lines = subscription_session("2026-06-15T01:00:00Z", "thread-a", "gpt-5.6-sol");
    lines.push(record_line(
        "2026-06-15T01:01:00Z",
        "resp-1",
        "thread-a",
        100,
        0,
        20,
    ));
    let path = write_rollout(&codex_home, "a", &lines);

    let first = stats(&db_path, &codex_home, "2026-06-15T02:00:00Z");
    assert_eq!(total_tokens(&first, "subscriptions", "sub-a", "all"), 120);

    append_lines(
        &path,
        &[record_line(
            "2026-06-15T01:02:00Z",
            "resp-2",
            "thread-a",
            200,
            0,
            40,
        )],
    );
    let second = stats(&db_path, &codex_home, "2026-06-15T02:01:00Z");
    assert_eq!(total_tokens(&second, "subscriptions", "sub-a", "all"), 360);

    // Rewritten in place (the provider line changes length): read again from the start, and
    // the records already stored are not added a second time.
    let rewritten = fs::read_to_string(&path)
        .unwrap()
        .replace("\"openai\"", "\"openai-renamed\"");
    fs::write(&path, rewritten).unwrap();
    append_lines(
        &path,
        &[record_line(
            "2026-06-15T01:03:00Z",
            "resp-3",
            "thread-a",
            1,
            0,
            1,
        )],
    );
    let third = stats(&db_path, &codex_home, "2026-06-15T02:02:00Z");
    assert_eq!(total_tokens(&third, "subscriptions", "sub-a", "all"), 360);
    assert_eq!(record_count(&db_path), 3);
    // resp-3 came after the rename, so no attribution matches its provider.
    assert_eq!(
        third["warnings"],
        json!(["1 个 session 缺少 Codex Switch 归属记录，未计入卡片"])
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_response_copied_into_another_rollout_counts_once() {
    let root = temp_root("dedupe");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    attribute(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        T0,
    );
    let mut parent = subscription_session("2026-06-15T01:00:00Z", "thread-parent", "gpt-5.6-sol");
    parent.push(record_line(
        "2026-06-15T01:01:00Z",
        "resp-shared",
        "thread-parent",
        100,
        0,
        10,
    ));
    write_rollout(&codex_home, "parent", &parent);
    // A fork carries the parent's history, then its own responses.
    let mut fork = parent.clone();
    fork.push(record_line(
        "2026-06-15T01:05:00Z",
        "resp-fork",
        "thread-fork",
        50,
        0,
        5,
    ));
    write_rollout(&codex_home, "parent_fork", &fork);

    let response = stats(&db_path, &codex_home, "2026-06-15T02:00:00Z");
    assert_eq!(
        total_tokens(&response, "subscriptions", "sub-a", "all"),
        165
    );
    assert_eq!(
        window(&response, "subscriptions", "sub-a", "all")["session_count"],
        json!(2)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn nothing_at_or_before_the_start_is_counted_and_untouched_old_files_stay_unopened() {
    let root = temp_root("counted-after");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    attribute(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        T0,
    );
    let mut lines = subscription_session("2026-06-14T23:00:00Z", "thread-a", "gpt-5.6-sol");
    lines.push(record_line(
        "2026-06-14T23:30:00Z",
        "resp-before",
        "thread-a",
        100,
        0,
        10,
    ));
    lines.push(record_line(T0, "resp-at-start", "thread-a", 100, 0, 10));
    lines.push(record_line(
        "2026-06-15T00:00:01Z",
        "resp-after",
        "thread-a",
        7,
        0,
        3,
    ));
    write_rollout(&codex_home, "active", &lines);
    let old = write_rollout(
        &codex_home,
        "old",
        &[record_line(
            "2026-06-14T10:00:00Z",
            "resp-old",
            "thread-old",
            5,
            0,
            5,
        )],
    );
    let before_start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_781_000_000);
    fs::OpenOptions::new()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(before_start)
        .unwrap();

    let response = stats(&db_path, &codex_home, "2026-06-15T02:00:00Z");
    assert_eq!(total_tokens(&response, "subscriptions", "sub-a", "all"), 10);
    let cursors: i64 = Connection::open(&db_path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM rollout_cursors WHERE source_path = ?1",
            [old.to_string_lossy().to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(cursors, 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn each_response_goes_to_the_owner_selected_when_it_ran() {
    let root = temp_root("attribution");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    attribute(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        T0,
    );
    attribute(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-b",
        PROVIDER_SUBSCRIPTION,
        "2026-06-15T01:30:00Z",
    );
    attribute(&db_path, OWNER_TYPE_API_PROFILE, "api-x", PROVIDER_API, T0);
    // One session spans the account switch; the second switches to API mode midway.
    let mut switching = subscription_session("2026-06-15T01:00:00Z", "thread-a", "gpt-5.6-sol");
    switching.push(record_line(
        "2026-06-15T01:10:00Z",
        "resp-a",
        "thread-a",
        100,
        0,
        0,
    ));
    switching.push(record_line(
        "2026-06-15T01:40:00Z",
        "resp-b",
        "thread-a",
        30,
        0,
        0,
    ));
    write_rollout(&codex_home, "switching", &switching);
    let mut api = subscription_session("2026-06-15T01:00:00Z", "thread-api", "gpt-5.5");
    api.push(thread_settings_line(
        "2026-06-15T01:05:00Z",
        "gpt-5.5",
        PROVIDER_API,
        json!("default"),
    ));
    api.push(record_line(
        "2026-06-15T01:06:00Z",
        "resp-api",
        "thread-api",
        7,
        0,
        0,
    ));
    write_rollout(&codex_home, "api", &api);

    let response = stats(&db_path, &codex_home, "2026-06-15T02:00:00Z");
    assert_eq!(
        total_tokens(&response, "subscriptions", "sub-a", "all"),
        100
    );
    assert_eq!(total_tokens(&response, "subscriptions", "sub-b", "all"), 30);
    assert_eq!(total_tokens(&response, "api_profiles", "api-x", "all"), 7);
    assert_eq!(response["warnings"], json!([]));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn managed_instance_records_use_the_instance_owner() {
    let root = temp_root("instance");
    let db_path = root.join("usage.sqlite");
    let instances = root.join("instances");
    let instance_root = instances.join("account-sub-z");
    fs::create_dir_all(&instance_root).unwrap();
    fs::write(
        instance_root.join(CODEX_APP_INSTANCE_MARKER_FILE),
        json!({ "managedBy": "codex-switch", "kind": "account", "targetId": "sub-z" }).to_string(),
    )
    .unwrap();
    let codex_home = instance_root.join("codex-home");
    let mut lines = subscription_session("2026-06-15T01:00:00Z", "thread-i", "gpt-5.6-sol");
    lines.push(record_line(
        "2026-06-15T01:01:00Z",
        "resp-i",
        "thread-i",
        40,
        0,
        2,
    ));
    write_rollout(&codex_home, "instance", &lines);

    // No attribution rows at all: the marker alone decides the owner.
    open_usage_connection(&db_path, T0).unwrap();
    let sources = managed_instance_usage_scan_sources(&instances).unwrap();
    let response =
        usage_stats_get_for_scan_sources(&db_path, &sources, "2026-06-15T02:00:00Z").unwrap();
    assert_eq!(total_tokens(&response, "subscriptions", "sub-z", "all"), 42);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn windows_are_exact_to_the_second_around_their_starts() {
    let root = temp_root("windows");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    let start = "2026-05-01T00:00:00Z";
    attribute(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        start,
    );
    let now = "2026-06-15T10:37:21Z";
    let now_seconds = parse_rfc3339_seconds(now).unwrap();
    let starts = usage_window_starts(now_seconds);
    // Records one second before, at, and after every window start, plus some in whole hours.
    let mut seconds = Vec::new();
    for window_start in [starts.today, starts.days_7, starts.days_30] {
        seconds.extend([
            window_start - 1,
            window_start,
            window_start + 1,
            window_start + 7_200,
        ]);
    }
    seconds.push(now_seconds - 5);
    let mut lines = subscription_session("2026-05-10T00:00:00Z", "thread-a", "gpt-5.6-sol");
    for (index, second) in seconds.iter().enumerate() {
        let timestamp = time::OffsetDateTime::from_unix_timestamp(*second)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        lines.push(record_line(
            &timestamp,
            &format!("resp-{index}"),
            "thread-a",
            1 << index,
            0,
            0,
        ));
    }
    write_rollout(&codex_home, "windows", &lines);

    let response = stats(&db_path, &codex_home, now);
    let expected = |from: i64| -> u64 {
        seconds
            .iter()
            .enumerate()
            .filter(|(_, second)| **second >= from)
            .map(|(index, _)| 1u64 << index)
            .sum()
    };
    assert_eq!(
        total_tokens(&response, "subscriptions", "sub-a", "today"),
        expected(starts.today)
    );
    assert_eq!(
        total_tokens(&response, "subscriptions", "sub-a", "days_7"),
        expected(starts.days_7)
    );
    assert_eq!(
        total_tokens(&response, "subscriptions", "sub-a", "days_30"),
        expected(starts.days_30)
    );
    assert_eq!(
        total_tokens(&response, "subscriptions", "sub-a", "all"),
        expected(i64::MIN)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cards_keep_session_count_last_use_and_per_model_totals() {
    let root = temp_root("shape");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    attribute(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        T0,
    );
    let mut first = subscription_session("2026-06-15T01:00:00Z", "thread-a", "gpt-5.6-sol");
    first.push(record_line(
        "2026-06-15T01:01:00Z",
        "resp-1",
        "thread-a",
        100,
        60,
        10,
    ));
    first.push(turn_context_line(
        "2026-06-15T01:02:00Z",
        "turn-2",
        "gpt-6-astra",
    ));
    first.push(record_line(
        "2026-06-15T01:03:00Z",
        "resp-2",
        "thread-a",
        50,
        0,
        5,
    ));
    write_rollout(&codex_home, "first", &first);
    let mut second = subscription_session("2026-06-15T01:10:00Z", "thread-b", "gpt-5.6-sol");
    second.push(record_line(
        "2026-06-15T01:11:00Z",
        "resp-3",
        "thread-b",
        10,
        0,
        1,
    ));
    write_rollout(&codex_home, "second", &second);

    let response = stats(&db_path, &codex_home, "2026-06-15T02:00:00Z");
    let all = window(&response, "subscriptions", "sub-a", "all");
    assert_eq!(all["session_count"], json!(2));
    assert_eq!(all["last_used"], json!("2026-06-15T01:11:00Z"));
    assert_eq!(all["input_tokens"], json!(160));
    assert_eq!(all["cached_input_tokens"], json!(60));
    assert_eq!(all["output_tokens"], json!(16));
    assert_eq!(
        all["pricing_contexts"],
        json!({ "standard_short_context": 3 })
    );
    let sol = &all["by_model"]["gpt-5.6-sol"];
    assert_eq!(sol["total_tokens"], json!(121));
    assert_eq!(sol["session_count"], json!(2));
    // (40 × $4 + 60 × $0.40 + 10 × $20 + 10 × $4 + 1 × $20) / 1M
    let sol_cost = sol["estimated_cost_usd"].as_f64().unwrap();
    assert!((sol_cost - 0.000_444).abs() < 1e-12, "{sol_cost}");
    assert_eq!(all["by_model"]["gpt-6-astra"]["total_tokens"], json!(55));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_write_rolls_back_the_whole_refresh() {
    let root = temp_root("rollback");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    attribute(
        &db_path,
        OWNER_TYPE_SUBSCRIPTION,
        "sub-a",
        PROVIDER_SUBSCRIPTION,
        T0,
    );
    let mut a = subscription_session("2026-06-15T01:00:00Z", "thread-a", "gpt-5.6-sol");
    a.push(record_line(
        "2026-06-15T01:01:00Z",
        "resp-a",
        "thread-a",
        100,
        0,
        20,
    ));
    write_rollout(&codex_home, "a", &a);
    let mut b = subscription_session("2026-06-15T01:00:00Z", "thread-b", "gpt-5.6-sol");
    b.push(record_line(
        "2026-06-15T01:02:00Z",
        "resp-b",
        "thread-b",
        80,
        0,
        20,
    ));
    write_rollout(&codex_home, "b", &b);
    Connection::open(&db_path)
        .unwrap()
        .execute_batch(
            r#"
            CREATE TRIGGER fail_rollout_b BEFORE INSERT ON rollout_cursors
            WHEN NEW.source_path LIKE '%rollout-b%'
            BEGIN SELECT RAISE(ABORT, 'forced failure'); END;
            "#,
        )
        .unwrap();
    let error = usage_stats_get_for_scan_sources(
        &db_path,
        &[main_usage_scan_source(&codex_home)],
        "2026-06-15T02:00:00Z",
    )
    .unwrap_err();
    assert!(error.contains("forced failure"), "{error}");
    assert_eq!(record_count(&db_path), 0);

    Connection::open(&db_path)
        .unwrap()
        .execute_batch("DROP TRIGGER fail_rollout_b")
        .unwrap();
    let recovered = stats(&db_path, &codex_home, "2026-06-15T02:01:00Z");
    assert_eq!(
        total_tokens(&recovered, "subscriptions", "sub-a", "all"),
        220
    );
    fs::remove_dir_all(root).unwrap();
}

fn create_legacy_database(db_path: &Path) {
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    Connection::open(db_path)
        .unwrap()
        .execute_batch(
            r#"
            CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO meta VALUES ('stats_started_at', '2026-05-01T00:00:00Z');
            CREATE TABLE attribution (
                id INTEGER PRIMARY KEY AUTOINCREMENT, owner_type TEXT NOT NULL,
                owner_id TEXT NOT NULL, provider TEXT NOT NULL, started_at TEXT NOT NULL,
                started_at_seconds INTEGER NOT NULL
            );
            INSERT INTO attribution(owner_type, owner_id, provider, started_at, started_at_seconds)
            VALUES ('subscription', 'sub-a', 'openai', '2026-05-01T00:00:00Z', 1777593600);
            CREATE TABLE session_usage (
                session_id TEXT PRIMARY KEY, source_path TEXT NOT NULL, owner_type TEXT NOT NULL,
                owner_id TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
                started_at TEXT NOT NULL, started_at_seconds INTEGER NOT NULL,
                updated_at TEXT NOT NULL, updated_at_seconds INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL, cached_input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL, reasoning_output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL, model_context_window INTEGER,
                estimated_cost_usd REAL, priced INTEGER NOT NULL, pricing_context TEXT,
                unpriced_reason TEXT, last_scanned_at TEXT NOT NULL
            );
            -- An old session (no events any more) and a recent one with events.
            INSERT INTO session_usage VALUES
                ('s-old', '/old.jsonl', 'subscription', 'sub-a', 'openai', 'gpt-5.5',
                 '2026-05-02T00:00:00Z', 1777680000, '2026-05-02T01:00:00Z', 1777683600,
                 1000000, 0, 100000, 0, 1100000, 258400, 8.0, 1, 'standard_short_context', NULL,
                 '2026-06-14T12:00:00Z'),
                ('s-new', '/new.jsonl', 'subscription', 'sub-a', 'openai', 'gpt-6-astra',
                 '2026-06-10T00:00:00Z', 1781049600, '2026-06-14T11:00:00Z', 1781434800,
                 3000, 1000, 300, 0, 3300, 486400, NULL, 0, NULL, 'missing_model_price',
                 '2026-06-14T12:00:00Z');
            CREATE TABLE session_token_events (
                source_path TEXT NOT NULL, event_index INTEGER NOT NULL,
                timestamp_seconds INTEGER NOT NULL, input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                reasoning_output_tokens INTEGER NOT NULL, total_tokens INTEGER NOT NULL,
                PRIMARY KEY(source_path, event_index)
            ) WITHOUT ROWID;
            -- 2026-06-10T00:00Z and 2026-06-14T11:00Z; an event of an unindexed file is ignored.
            INSERT INTO session_token_events VALUES
                ('/new.jsonl', 0, 1781049600, 1000, 0, 100, 0, 1100),
                ('/new.jsonl', 1, 1781434800, 2000, 1000, 200, 0, 2200),
                ('/unindexed.jsonl', 0, 1781434800, 9, 0, 9, 0, 18);
            CREATE TABLE session_scan_state (
                source_path TEXT PRIMARY KEY, modified_nanos INTEGER NOT NULL,
                file_size INTEGER NOT NULL, scan_scope TEXT NOT NULL, session_id TEXT NOT NULL,
                outcome TEXT NOT NULL, last_scanned_at TEXT NOT NULL
            );
            INSERT INTO session_scan_state VALUES
                ('/new.jsonl', 0, 0, '', 's-new', 'indexed', '2026-06-14T12:00:00Z'),
                ('/old.jsonl', 0, 0, '', 's-old', 'indexed', '2026-06-01T00:00:00Z');
            "#,
        )
        .unwrap();
}

#[test]
fn per_session_statistics_are_carried_over_once() {
    let root = temp_root("migration");
    let db_path = root.join("usage.sqlite");
    let codex_home = root.join("codex");
    create_legacy_database(&db_path);
    // A response the old statistics already covered (before their last scan) and a new one.
    let mut lines = subscription_session("2026-06-14T10:00:00Z", "s-new", "gpt-6-astra");
    lines.push(record_line(
        "2026-06-14T11:00:00Z",
        "resp-covered",
        "s-new",
        2000,
        1000,
        200,
    ));
    lines.push(record_line(
        "2026-06-14T13:00:00Z",
        "resp-new",
        "s-new",
        500,
        0,
        50,
    ));
    write_rollout(&codex_home, "new", &lines);

    let now = "2026-06-14T14:00:00Z";
    let first = stats(&db_path, &codex_home, now);
    let connection = Connection::open(&db_path).unwrap();
    let counted_after: String = connection
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [META_RECORDS_COUNTED_AFTER],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(counted_after, "2026-06-14T12:00:00Z");
    // All: both old sessions plus the new response.
    assert_eq!(
        total_tokens(&first, "subscriptions", "sub-a", "all"),
        1_100_000 + 3300 + 550
    );
    // 30 days: the old events of the indexed session plus the new response.
    assert_eq!(
        total_tokens(&first, "subscriptions", "sub-a", "days_30"),
        1100 + 2200 + 550
    );
    let all = window(&first, "subscriptions", "sub-a", "all");
    assert_eq!(all["session_count"], json!(2));
    // gpt-6-astra now has a price, so nothing is unpriced any more.
    assert_eq!(all["priced"], json!(true));
    // Standard short-context prices of the current table: gpt-5.5 1M × $5 + 0.1M × $30, and
    // gpt-6-astra (2000 × $10 + 1000 × $1 + 300 × $50) + the new response (500 × $10 + 50 × $50).
    let expected = 5.0 + 3.0 + (20_000.0 + 1_000.0 + 15_000.0 + 5_000.0 + 2_500.0) / 1e6;
    let cost = all["estimated_cost_usd"].as_f64().unwrap();
    assert!((cost - expected).abs() < 1e-9, "{cost} vs {expected}");
    // The old tables are left as they are.
    let old_rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM session_usage", [], |row| row.get(0))
        .unwrap();
    assert_eq!(old_rows, 2);
    drop(connection);

    // Opening again does not carry anything over a second time.
    let again = stats(&db_path, &codex_home, "2026-06-14T14:01:00Z");
    assert_eq!(
        total_tokens(&again, "subscriptions", "sub-a", "all"),
        1_100_000 + 3300 + 550
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn request_cost_uses_the_tier_and_this_requests_context_length() {
    let short = usage(LONG_CONTEXT_THRESHOLD_TOKENS, 72_000, 0, 1_000);
    // 200K × $4 + 72K × $0.40 + 1K × $20
    assert_cost(
        &estimate_request_cost("gpt-5.6-sol", &short, ServiceTier::Standard),
        0.8 + 0.0288 + 0.02,
        PRICE_LABEL_STANDARD_SHORT,
    );
    let long = usage(LONG_CONTEXT_THRESHOLD_TOKENS + 1, 0, 0, 0);
    assert_cost(
        &estimate_request_cost("gpt-5.6-sol", &long, ServiceTier::Standard),
        272_001.0 * 8.0 / 1e6,
        PRICE_LABEL_STANDARD_LONG,
    );
    assert_cost(
        &estimate_request_cost("gpt-6-astra", &usage(1_000_000, 0, 0, 0), ServiceTier::Fast),
        40.0,
        PRICE_LABEL_FAST_LONG,
    );
    assert_cost(
        &estimate_request_cost("gpt-6-astra", &usage(100, 0, 0, 10), ServiceTier::Fast),
        (100.0 * 20.0 + 10.0 * 100.0) / 1e6,
        PRICE_LABEL_FAST_SHORT,
    );
    // Cache writes are part of the input and priced on their own.
    assert_cost(
        &estimate_request_cost(
            "gpt-6-sol",
            &usage(1_000, 200, 300, 0),
            ServiceTier::Standard,
        ),
        (500.0 * 2.0 + 200.0 * 0.2 + 300.0 * 2.5) / 1e6,
        PRICE_LABEL_STANDARD_SHORT,
    );
    // gpt-5.4-mini has one price whatever the length.
    assert_cost(
        &estimate_request_cost("gpt-5.4-mini", &long, ServiceTier::Standard),
        272_001.0 * 0.75 / 1e6,
        PRICE_LABEL_STANDARD_SHORT,
    );
}

#[test]
fn missing_prices_leave_the_request_unpriced_with_the_reason() {
    let long = usage(LONG_CONTEXT_THRESHOLD_TOKENS + 1, 0, 0, 0);
    let cases = [
        (
            "gpt-5.5",
            long.clone(),
            ServiceTier::Fast,
            UNPRICED_MISSING_TIER_PRICE,
        ),
        (
            "gpt-5.6-sol",
            usage(10, 0, 0, 0),
            ServiceTier::from_setting("flex"),
            UNPRICED_MISSING_TIER_PRICE,
        ),
        (
            "codex-auto-review",
            usage(10, 0, 0, 0),
            ServiceTier::Standard,
            UNPRICED_MISSING_MODEL_PRICE,
        ),
        (
            "gpt-5.5",
            usage(10, 0, 5, 0),
            ServiceTier::Standard,
            UNPRICED_MISSING_CACHE_WRITE_PRICE,
        ),
    ];
    for (model, usage, tier, reason) in cases {
        let cost = estimate_request_cost(model, &usage, tier);
        assert_eq!(
            cost,
            EstimatedCost {
                cost_usd: None,
                price_label: reason
            },
            "{model}"
        );
    }
    assert_eq!(ServiceTier::from_setting("priority"), ServiceTier::Fast);
    assert_eq!(ServiceTier::from_setting("fast"), ServiceTier::Fast);
    assert_eq!(ServiceTier::from_setting("default"), ServiceTier::Standard);
    assert_eq!(ServiceTier::from_setting(""), ServiceTier::Standard);
}
