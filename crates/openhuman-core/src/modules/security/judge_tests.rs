use super::*;
use crate::config::schema::{JudgeMode, PrivacyMode, SecurityModeConfig};

fn route() -> SecurityJudgeRoute {
    SecurityJudgeRoute::open_router("a-working-key")
}

/// A judge as the default settings build it, for other modules' tests.
pub(in crate::modules::security) fn a_judge() -> JudgeConfig {
    build(&Config::default(), route()).expect("on by default")
}

#[test]
fn a_route_turns_the_judge_on_by_default() {
    let judge = build(&Config::default(), route()).expect("on by default");

    assert_eq!(judge.credential.0, "a-working-key");
    assert_eq!(judge.endpoint, "https://openrouter.ai/api/v1/systemone");
    assert_eq!(judge.provider, JudgeProvider::OpenRouter);
}

#[test]
fn the_judge_never_covers_scheduled_or_remote_callers() {
    let judge = build(&Config::default(), route()).expect("on by default");

    assert!(!judge.origins.contains(&Origin::Cron));
    assert!(!judge.origins.contains(&Origin::Saas));
    assert!(judge.origins.contains(&Origin::Copilot));
}

#[test]
fn the_default_settings_are_inside_the_bounds_the_module_accepts() {
    // A config the module rejects fails its initialization, and enforced
    // security then refuses everything, so these must stay in range.
    let judge = build(&Config::default(), route()).expect("on by default");

    assert!(judge.threshold_millis <= 1000);
    assert!(judge.exfiltrate_millis <= 1000);
    assert!((1..=30_000).contains(&judge.timeout_ms));
    assert!((1..=10_000).contains(&judge.max_requests));
    assert!((1..=86_400_000).contains(&judge.budget_window_ms));
    assert!(judge.cache_entries <= 1024);
    assert!(judge.cache_ttl_ms <= 600_000);
    assert!(judge.origins.len() <= 6);
}

#[test]
fn the_off_switch_keeps_the_judge_out() {
    let mut config = Config::default();
    config.security.judge = JudgeMode::Off;

    assert!(build(&config, route()).is_none());
}

#[test]
fn security_off_keeps_the_judge_out() {
    let mut config = Config::default();
    config.security = SecurityModeConfig::off();

    assert!(build(&config, route()).is_none());
}

#[test]
fn local_only_privacy_keeps_call_details_on_the_machine() {
    let mut config = Config::default();
    config.privacy.mode = PrivacyMode::LocalOnly;

    assert!(build(&config, route()).is_none());
}

#[test]
fn an_unusable_key_is_left_out_rather_than_failing_the_module() {
    let config = Config::default();

    assert!(build(&config, SecurityJudgeRoute::open_router("abc")).is_none());
    assert!(build(&config, SecurityJudgeRoute::open_router("k".repeat(4097))).is_none());
}

#[test]
fn an_empty_endpoint_is_left_out_rather_than_failing_the_module() {
    let mut empty = route();
    empty.endpoint = String::new();

    assert!(build(&Config::default(), empty).is_none());
}

#[test]
fn the_tinyhumans_gateway_speaks_the_openrouter_dialect() {
    let judge = build(
        &Config::default(),
        SecurityJudgeRoute::tinyhumans("https://api.example.test/", "a-working-key"),
    )
    .expect("on by default");

    assert_eq!(
        judge.endpoint,
        "https://api.example.test/agent-integrations/openrouter/systemone"
    );
    assert_eq!(judge.provider, JudgeProvider::OpenRouter);
}

#[test]
fn openjev_keeps_its_own_dialect() {
    let judge = build(
        &Config::default(),
        SecurityJudgeRoute::open_jev("a-working-key"),
    )
    .expect("on by default");

    assert_eq!(judge.endpoint, "https://api.openjev.sh/v1/systemone");
    assert_eq!(judge.provider, JudgeProvider::OpenJev);
}

#[test]
fn only_the_hosts_own_route_turns_the_judge_on() {
    let hosted = Config {
        security_judge: Some(SecurityJudgeRoute::open_jev("a-hosts-key")),
        ..Config::default()
    };
    let judge = resolve(&hosted).expect("the host supplied a route");
    assert_eq!(judge.credential.0, "a-hosts-key");
    assert_eq!(judge.provider, JudgeProvider::OpenJev);
    // Without one it reads no stored credential: the judge stays off.
    assert!(resolve(&Config::default()).is_none());
}

fn with_endpoint(endpoint: &str) -> SecurityJudgeRoute {
    SecurityJudgeRoute {
        endpoint: endpoint.to_owned(),
        ..route()
    }
}

#[test]
fn an_endpoint_the_client_refuses_is_left_out_rather_than_failing_the_module() {
    let config = Config::default();

    for refused in [
        "not a url",
        "ftp://api.example.test/v1/systemone",
        "http://api.example.test/v1/systemone",
        "https://user:secret@api.example.test/v1/systemone",
        "https://api.example.test/v1/systemone?key=value",
        "https://api.example.test/v1/systemone#part",
    ] {
        assert!(
            build(&config, with_endpoint(refused)).is_none(),
            "{refused}"
        );
    }
}

#[test]
fn plain_http_is_fine_on_a_literal_loopback_address() {
    let config = Config::default();

    assert!(build(&config, with_endpoint("http://127.0.0.1:8080/systemone")).is_some());
    assert!(build(&config, with_endpoint("http://[::1]:8080/systemone")).is_some());
}

#[test]
fn a_blank_key_is_left_out_rather_than_failing_the_module() {
    assert!(build(&Config::default(), SecurityJudgeRoute::open_router("     ")).is_none());
}
