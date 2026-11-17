use super::*;

#[test]
fn iana_names_normalise_and_everything_else_is_refused() {
    assert_eq!(
        normalize_time_zone(" Asia/Kolkata ").as_deref(),
        Some("Asia/Kolkata")
    );
    assert_eq!(
        normalize_time_zone("asia/kolkata").as_deref(),
        Some("Asia/Kolkata")
    );
    assert_eq!(normalize_time_zone("UTC").as_deref(), Some("UTC"));
    for bad in [
        "",
        "  ",
        "IST",
        "+05:30",
        "Asia/Mumbai",
        "Mars/Olympus",
        "EST",
        "mst",
        "GMT",
        "EST5EDT",
    ] {
        assert_eq!(
            normalize_time_zone(bad),
            None,
            "{bad:?} is not an IANA zone"
        );
    }
}

#[test]
fn the_users_zone_wins_over_the_device() {
    let config = Config {
        user_timezone: Some("America/Sao_Paulo".into()),
        ..Config::default()
    };
    assert_eq!(config.time_zone(), "America/Sao_Paulo");
}

#[test]
fn no_or_an_invalid_user_zone_falls_back_to_the_device_then_utc() {
    let fallback = device_time_zone().unwrap_or_else(|| "UTC".to_string());
    for user in [None, Some("IST".to_string())] {
        let config = Config {
            user_timezone: user,
            ..Config::default()
        };
        assert_eq!(config.time_zone(), fallback);
    }
}
