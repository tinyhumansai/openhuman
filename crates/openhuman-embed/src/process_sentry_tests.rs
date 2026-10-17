use super::*;

fn event(message: &str, tags: &[(&str, &str)]) -> Event<'static> {
    let mut event = Event {
        message: Some(message.to_string()),
        ..Default::default()
    };
    for (key, value) in tags {
        event.tags.insert((*key).to_string(), (*value).to_string());
    }
    event
}

fn no_user() -> Option<String> {
    None
}

fn fixed_user() -> Option<String> {
    Some("user-123".into())
}

#[test]
fn the_chain_is_the_union_of_both_hosts_with_unique_names() {
    // 19 CLI predicates + 1 shell-only (localhost dev fetch); the shell's other
    // 11 were already in the CLI chain. `is_transient_backend_api_failure` and
    // its three siblings were one `||` group in both hosts.
    assert_eq!(NOISE_FILTERS.len(), 21);
    let mut names: Vec<_> = NOISE_FILTERS.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        NOISE_FILTERS.len(),
        "filter names must be unique"
    );
}

#[test]
fn each_known_noise_class_is_dropped() {
    let cases = [
        (
            "localhost-dev-fetch",
            event(
                "Failed to request http://localhost:1420/index.html: refused",
                &[],
            ),
        ),
        (
            "transient-provider-http",
            event(
                "provider failed",
                &[
                    ("domain", "llm_provider"),
                    ("failure", "non_2xx"),
                    ("status", "503"),
                ],
            ),
        ),
        ("fs-limitation", event("write failed (os error 665)", &[])),
        (
            "max-iterations",
            event(
                &format!(
                    "{}: 50",
                    tinyagents_harness::agent_error::MAX_ITERATIONS_ERROR_PREFIX
                ),
                &[],
            ),
        ),
        (
            "channel-message-404",
            event(
                "DELETE /channels/c1/messages/m1 -> 404",
                &[
                    ("domain", "backend_api"),
                    ("failure", "non_2xx"),
                    ("status", "404"),
                    ("method", "DELETE"),
                ],
            ),
        ),
        (
            "session-expired",
            event(
                "Session expired. Please log in again.",
                &[("domain", "rpc")],
            ),
        ),
    ];
    for (expected, event) in cases {
        let name = known_noise(&event);
        assert!(name.is_some(), "{expected}: not classified as noise");
        assert!(
            before_send(event, no_user).is_none(),
            "{expected}: before_send kept it (classified as {name:?})"
        );
    }
}

#[test]
fn a_real_error_is_kept_scrubbed_and_attributed() {
    let mut event = event("upload failed: api_key=sk-abc123", &[("domain", "memory")]);
    event.server_name = Some("operators-laptop".into());
    assert_eq!(known_noise(&event), None);

    let kept = before_send(event, fixed_user).expect("a real error reaches Sentry");
    assert_eq!(kept.server_name, None, "hostname stripped");
    assert_eq!(
        kept.message.as_deref(),
        Some("upload failed: api_key=[REDACTED]")
    );
    assert_eq!(kept.user.and_then(|u| u.id).as_deref(), Some("user-123"));
}

#[test]
fn a_scope_bound_user_is_not_overwritten() {
    let mut event = event("boom", &[]);
    event.user = Some(sentry::User {
        id: Some("scope-user".into()),
        ..Default::default()
    });
    let kept = before_send(event, fixed_user).expect("kept");
    assert_eq!(kept.user.and_then(|u| u.id).as_deref(), Some("scope-user"));
}

#[test]
fn a_user_without_an_id_gets_the_fallback_and_keeps_its_fields() {
    let mut event = event("boom", &[]);
    event.user = Some(sentry::User {
        username: Some("operator".into()),
        ..Default::default()
    });
    let user = before_send(event, fixed_user).expect("kept").user.unwrap();
    assert_eq!(user.id.as_deref(), Some("user-123"));
    assert_eq!(user.username.as_deref(), Some("operator"));
}

#[test]
fn breadcrumbs_tags_extra_and_request_are_scrubbed() {
    use sentry::protocol::{Breadcrumb, Request, Value};
    let secret = "api_key=sk-abc123";
    let mut event = event("boom", &[("detail", secret)]);
    let mut crumb = Breadcrumb {
        message: Some(secret.into()),
        ..Default::default()
    };
    crumb
        .data
        .insert("nested".into(), serde_json::json!({ "list": [secret] }));
    event.breadcrumbs.values.push(crumb);
    event
        .extra
        .insert("body".into(), Value::String(secret.into()));
    let mut request = Request {
        query_string: Some(secret.into()),
        data: Some(secret.into()),
        cookies: Some("session=abc".into()),
        ..Default::default()
    };
    request
        .headers
        .insert("Authorization".into(), "Bearer abc123xyz".into());
    event.request = Some(request);

    let kept = before_send(event, no_user).expect("kept");
    let redacted = "api_key=[REDACTED]";
    assert_eq!(kept.tags["detail"], redacted);
    let crumb = &kept.breadcrumbs.values[0];
    assert_eq!(crumb.message.as_deref(), Some(redacted));
    assert_eq!(crumb.data["nested"]["list"][0], redacted);
    assert_eq!(kept.extra["body"], redacted);
    let request = kept.request.expect("request kept");
    assert_eq!(request.query_string.as_deref(), Some(redacted));
    assert_eq!(request.data.as_deref(), Some(redacted));
    assert_eq!(request.cookies, None);
    assert_eq!(request.headers["Authorization"], "Bearer [REDACTED]");
}

#[test]
fn exception_values_are_scrubbed() {
    let mut event = event("boom", &[]);
    event.exception.values.push(sentry::protocol::Exception {
        value: Some("Authorization: Bearer abc123xyz".into()),
        ..Default::default()
    });
    let kept = before_send(event, no_user).expect("kept");
    assert_eq!(
        kept.exception.values[0].value.as_deref(),
        Some("Authorization: Bearer [REDACTED]")
    );
}

#[test]
fn the_localhost_rule_is_anchored_on_the_dev_proxy_prefix() {
    assert!(message_is_localhost_dev_fetch_noise(
        "Failed to request http://127.0.0.1:1420/x: err"
    ));
    assert!(!message_is_localhost_dev_fetch_noise(
        "Failed to request https://api.example.com/x: err"
    ));
}

#[test]
fn release_tags_carry_a_short_sha_when_built_with_one() {
    assert_eq!(release_tag("1.2.3", None), "openhuman@1.2.3");
    assert_eq!(release_tag("1.2.3", Some("  ")), "openhuman@1.2.3");
    assert_eq!(
        release_tag("1.2.3", Some("0123456789abcdef")),
        "openhuman@1.2.3+0123456789ab"
    );
}

#[test]
fn the_first_non_blank_candidate_wins() {
    assert_eq!(
        first_non_blank([None, Some("  ".into()), Some("https://k@o/1".into())]),
        Some("https://k@o/1".into())
    );
    assert_eq!(first_non_blank([None, Some(String::new())]), None);
}

#[test]
fn client_options_wire_the_chain_and_the_transport() {
    let options = client_options(SentryConfig::new(
        Some("not a dsn".into()),
        release_tag("1.0.0", None),
        "test".into(),
    ));
    assert_eq!(options.traces_sample_rate, 0.1);
    assert!(options.dsn.is_none(), "an unparsable DSN sends nothing");
    assert_eq!(options.release.as_deref(), Some("openhuman@1.0.0"));
    assert_eq!(options.environment.as_deref(), Some("test"));
    assert!(!options.send_default_pii);
    assert!(options.transport.is_some());
    let before_send = options.before_send.expect("chain installed");
    assert!(before_send(event("Failed to request http://localhost:1/x: e", &[])).is_none());
}

#[test]
fn environment_prefers_app_env_lowercased() {
    assert_eq!(resolve_environment(Some(" Staging ".into())), "staging");
}

#[test]
fn environment_falls_back_on_blank_or_missing() {
    let expected = if cfg!(debug_assertions) {
        "development"
    } else {
        "production"
    };
    assert_eq!(resolve_environment(None), expected);
    assert_eq!(resolve_environment(Some("   ".into())), expected);
}

#[test]
fn core_dsn_precedence_is_runtime_then_baked_and_skips_blanks() {
    let some = |v: &str| Some(v.to_string());
    assert_eq!(
        select_core_dsn(some("rc"), some("rl"), Some("bc"), Some("bl")).as_deref(),
        Some("rc")
    );
    assert_eq!(
        select_core_dsn(some("  "), some("rl"), Some("bc"), Some("bl")).as_deref(),
        Some("rl")
    );
    assert_eq!(
        select_core_dsn(None, None, Some(" bc "), Some("bl")).as_deref(),
        Some("bc")
    );
    assert_eq!(
        select_core_dsn(None, None, Some(""), Some("bl")).as_deref(),
        Some("bl")
    );
    assert_eq!(select_core_dsn(None, None, None, Some("  ")), None);
}
