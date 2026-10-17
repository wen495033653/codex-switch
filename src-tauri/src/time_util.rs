use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub(crate) fn parse_rfc3339_seconds(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value.trim(), &Rfc3339)
        .ok()
        .map(|time| time.unix_timestamp())
}

pub(crate) fn parse_rfc3339_millis(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value.trim(), &Rfc3339)
        .ok()
        .and_then(|time| i64::try_from(time.unix_timestamp_nanos() / 1_000_000).ok())
}

/// Seconds since the epoch as RFC 3339 (UTC); `None` outside the representable range.
pub(crate) fn format_unix_seconds(seconds: i64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|time| time.format(&Rfc3339).ok())
}

pub(crate) fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}
