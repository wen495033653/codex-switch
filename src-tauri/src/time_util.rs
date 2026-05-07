use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub(crate) fn parse_rfc3339_seconds(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value.trim(), &Rfc3339)
        .ok()
        .map(|time| time.unix_timestamp())
}

pub(crate) fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}
