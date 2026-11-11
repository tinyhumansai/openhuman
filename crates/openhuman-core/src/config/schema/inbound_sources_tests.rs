use super::*;

fn parsed(toml_text: &str) -> crate::config::schema::InboundConfig {
    toml::from_str(toml_text).unwrap()
}

#[test]
fn a_source_needs_only_its_agent() {
    let inbound = parsed("[sources.\"x:mentions\"]\nagent_id = \"teeny-mentions\"\n");
    let source = inbound.source("x:mentions").unwrap();
    assert_eq!(source.agent_id, "teeny-mentions");
    assert_eq!(source.debounce_ms, DEFAULT_DEBOUNCE_MS);
    assert_eq!(source.max_wait_ms, DEFAULT_MAX_WAIT_MS);
    assert_eq!(source.max_batch, DEFAULT_MAX_BATCH);
    assert_eq!(source.result_ttl_days, DEFAULT_RESULT_TTL_DAYS);
    assert_eq!(source.prompt, None);
    assert_eq!(
        (5_000, 60_000, 25, 7),
        (
            DEFAULT_DEBOUNCE_MS,
            DEFAULT_MAX_WAIT_MS,
            DEFAULT_MAX_BATCH,
            DEFAULT_RESULT_TTL_DAYS
        )
    );
}

#[test]
fn every_setting_is_read() {
    let inbound = parsed(
        "[sources.\"x:mentions\"]\nagent_id = \"a\"\ndebounce_ms = 30000\nmax_wait_ms = 300000\n\
         max_batch = 20\nprompt = \"go {{events}}\"\nresult_ttl_days = 3\n",
    );
    let source = inbound.source("x:mentions").unwrap();
    assert_eq!(
        (source.debounce_ms, source.max_wait_ms, source.max_batch),
        (30_000, 300_000, 20)
    );
    assert_eq!(source.prompt.as_deref(), Some("go {{events}}"));
    assert_eq!(source.result_ttl_days, 3);
}

#[test]
fn an_unconfigured_source_is_refused_by_name() {
    let inbound = parsed("");
    let refused = inbound.source("x:mentions").unwrap_err();
    assert!(refused.contains("x:mentions"), "{refused}");
    assert!(refused.contains("not configured"), "{refused}");
}

#[test]
fn source_names_are_checked() {
    for good in ["x:mentions", "x_1:a.b", "a-b:c*", "rss:feed-2"] {
        assert!(check_source_name(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "mentions",
        "X:mentions",
        "x:",
        ":m",
        "x:m:n",
        "x y:m",
        "x:m/n",
        "x:é",
    ] {
        assert!(check_source_name(bad).is_err(), "{bad}");
    }
    for reserved in ["composio:gmail", "webhook:t1", "web:chat", "channel:slack"] {
        let refused = check_source_name(reserved).unwrap_err();
        assert!(refused.contains("reserved"), "{reserved}: {refused}");
    }
}

#[test]
fn an_unusable_source_is_refused_and_left_out() {
    let inbound = parsed(
        "[sources.\"x:mentions\"]\nagent_id = \"a\"\n\
         [sources.\"x:dms\"]\nagent_id = \" \"\n\
         [sources.\"x:zero\"]\nagent_id = \"a\"\nmax_batch = 0\n\
         [sources.\"composio:gmail\"]\nagent_id = \"a\"\n",
    );
    assert!(inbound.source("x:dms").unwrap_err().contains("agent_id"));
    assert!(inbound.source("x:zero").unwrap_err().contains("max_batch"));
    assert!(inbound
        .source("composio:gmail")
        .unwrap_err()
        .contains("reserved"));
    let usable: Vec<&str> = inbound.usable_sources().map(|(name, _)| name).collect();
    assert_eq!(usable, ["x:mentions"]);
    assert!(inbound.is_pinned("x:mentions"));
    assert!(!inbound.is_pinned("x:dms"));
}
