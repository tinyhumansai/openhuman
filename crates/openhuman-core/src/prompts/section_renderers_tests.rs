use super::*;

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("valid RFC 3339 timestamp")
        .with_timezone(&Utc)
}

#[test]
fn datetime_line_formats_a_supplied_utc_instant_in_utc_and_iana_zones() {
    let instant = utc("2024-01-01T12:34:56Z");

    assert_eq!(
        current_datetime_line_at(instant, Some("UTC")),
        "Current Date & Time: 2024-01-01 12:34:56 UTC (UTC, UTC+00:00), Monday"
    );
    assert_eq!(
        current_datetime_line_at(instant, Some("Pacific/Chatham")),
        "Current Date & Time: 2024-01-02 02:19:56 Pacific/Chatham (+1345, UTC+13:45), Tuesday"
    );
    assert_eq!(
        current_datetime_line_at(utc("2024-07-01T12:34:56Z"), Some("Pacific/Chatham")),
        "Current Date & Time: 2024-07-02 01:19:56 Pacific/Chatham (+1245, UTC+12:45), Tuesday"
    );
}

#[test]
fn newest_prompt_clock_wins_and_non_lifo_drop_restores_the_remaining_provider() {
    let first = register_prompt_clock(|| utc("2024-01-01T12:00:00Z"));
    let second = register_prompt_clock(|| utc("2024-02-02T13:00:00Z"));
    let third = register_prompt_clock(|| utc("2024-03-03T14:00:00Z"));

    assert_eq!(
        current_datetime_line(Some("UTC")),
        current_datetime_line_at(utc("2024-03-03T14:00:00Z"), Some("UTC"))
    );

    drop(second);
    assert_eq!(
        current_datetime_line(Some("UTC")),
        current_datetime_line_at(utc("2024-03-03T14:00:00Z"), Some("UTC"))
    );
    drop(third);
    assert_eq!(
        current_datetime_line(Some("UTC")),
        current_datetime_line_at(utc("2024-01-01T12:00:00Z"), Some("UTC"))
    );
    drop(first);

    let before = Utc::now();
    let actual = current_datetime_line(Some("UTC"));
    let after = Utc::now();
    let stamped = actual
        .strip_prefix("Current Date & Time: ")
        .and_then(|rest| rest.get(..19))
        .and_then(|value| chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").ok())
        .expect("default clock emits a parseable UTC timestamp")
        .and_utc();
    assert!(
        (before.timestamp()..=after.timestamp()).contains(&stamped.timestamp()),
        "dropping the final registration must restore the live system clock"
    );
}

#[test]
fn prompt_clock_registration_is_removed_when_scope_unwinds() {
    let prior = register_prompt_clock(|| utc("2024-01-01T12:00:00Z"));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _temporary = register_prompt_clock(|| utc("2024-02-02T13:00:00Z"));
        assert_eq!(
            current_datetime_line(Some("UTC")),
            current_datetime_line_at(utc("2024-02-02T13:00:00Z"), Some("UTC"))
        );
        panic!("exercise prompt-clock guard unwinding");
    }));

    assert!(result.is_err());
    assert_eq!(
        current_datetime_line(Some("UTC")),
        current_datetime_line_at(utc("2024-01-01T12:00:00Z"), Some("UTC"))
    );
    drop(prior);
}
