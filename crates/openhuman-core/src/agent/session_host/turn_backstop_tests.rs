use super::*;
use tinyagents_harness::turn_deadline::{parse_backstop, TurnDeadline};

#[test]
fn parse_backstop_defaults_and_disables() {
    assert_eq!(parse_backstop(None), Some(Duration::from_secs(900)));
    assert_eq!(
        parse_backstop(Some("garbage")),
        Some(Duration::from_secs(900))
    );
    assert_eq!(
        parse_backstop(Some(" 120 ")),
        Some(Duration::from_secs(120))
    );
    assert_eq!(parse_backstop(Some("0")), None);
}

/// The web backstop defaults to the harness turn ceiling, not the harness's
/// 900s, and keeps the env override and the `0` opt-out.
#[test]
fn web_backstop_defaults_to_an_hour_and_keeps_the_override() {
    let hour = Some(Duration::from_secs(DEFAULT_WEB_TURN_BACKSTOP_SECS));
    assert_eq!(DEFAULT_WEB_TURN_BACKSTOP_SECS, 3_600);
    assert_eq!(parse_web_backstop(None), hour);
    assert_eq!(parse_web_backstop(Some("garbage")), hour);
    assert_eq!(
        parse_web_backstop(Some(" 120 ")),
        Some(Duration::from_secs(120))
    );
    assert_eq!(parse_web_backstop(Some("0")), None);
    let deadline = TurnDeadline::starting_now(hour.unwrap());
    assert_eq!(deadline.wind_down_after(), Duration::from_secs(3_480));
    assert_eq!(deadline.hard_stop_after(), Duration::from_secs(3_540));
}
