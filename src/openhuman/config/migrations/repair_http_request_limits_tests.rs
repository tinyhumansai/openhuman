use super::*;
use crate::openhuman::config::Config;

#[test]
fn repairs_zero_timeout_and_size() {
    let mut config = Config::default();
    config.http_request.timeout_secs = 0;
    config.http_request.max_response_size = 0;

    let stats = run(&mut config).expect("migration should succeed");

    let defaults = HttpRequestConfig::default();
    assert!(stats.timeout_repaired);
    assert!(stats.max_response_size_repaired);
    assert_eq!(config.http_request.timeout_secs, defaults.timeout_secs);
    assert_eq!(
        config.http_request.max_response_size,
        defaults.max_response_size
    );
    // The whole point: no zeros survive.
    assert_ne!(config.http_request.timeout_secs, 0);
    assert_ne!(config.http_request.max_response_size, 0);
}

#[test]
fn repairs_only_the_zero_field() {
    let mut config = Config::default();
    config.http_request.timeout_secs = 0;
    config.http_request.max_response_size = 2_000_000;

    let stats = run(&mut config).expect("migration should succeed");

    assert!(stats.timeout_repaired);
    assert!(!stats.max_response_size_repaired);
    assert_ne!(config.http_request.timeout_secs, 0);
    assert_eq!(config.http_request.max_response_size, 2_000_000);
}

#[test]
fn leaves_nonzero_values_untouched() {
    let mut config = Config::default();
    config.http_request.timeout_secs = 45;
    config.http_request.max_response_size = 3_000_000;

    let stats = run(&mut config).expect("migration should succeed");

    assert!(!stats.timeout_repaired);
    assert!(!stats.max_response_size_repaired);
    assert_eq!(config.http_request.timeout_secs, 45);
    assert_eq!(config.http_request.max_response_size, 3_000_000);
}
