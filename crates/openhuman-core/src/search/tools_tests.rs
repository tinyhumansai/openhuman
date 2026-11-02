use super::*;
use crate::config::{ProviderRoute, SearchProviderSettings};

fn signed_out_byok_config() -> Config {
    let mut config = Config::default();
    config.search.providers = [
        ("brave".to_string(), SearchProviderSettings::direct()),
        ("tavily".to_string(), SearchProviderSettings::direct()),
    ]
    .into_iter()
    .collect();
    config.search.brave.api_key = Some("b".into());
    config.search.tavily.api_key = Some("t".into());
    config
}

#[test]
fn classified_errors_become_actionable_messages() {
    let balance = user_facing_error("tinysearch.insufficient_balance: 402 from backend");
    assert!(balance.contains("balance is too low"));
    assert!(user_facing_error("tinysearch.rate_limited: slow down").contains("rate limited"));
    assert_eq!(
        user_facing_error("tinysearch.provider_unavailable: all down"),
        SEARCH_EXHAUSTED_MESSAGE
    );
    assert_eq!(
        user_facing_error(
            "search ExecuteTool failed: tinysearch.invalid_arguments: urls must not be empty"
        ),
        "The search request was rejected: urls must not be empty"
    );
    assert_eq!(user_facing_error("boom"), "Web search failed: boom");
}

#[tokio::test]
async fn role_tools_are_built_for_usable_byok_providers() {
    let config = signed_out_byok_config();
    let tools = build_search_tools(&config).await;
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"web_search_tool"), "{names:?}");
    assert!(names.contains(&"web_contents_tool"), "{names:?}");
    // Nothing usable can answer without a session or a Gemini key.
    assert!(!names.contains(&"web_answer_tool"), "{names:?}");
    let search = tools
        .iter()
        .find(|t| t.name() == "web_search_tool")
        .unwrap();
    assert_eq!(search.category(), ToolCategory::Workflow);
    assert!(search.supports_markdown());
    assert!(search.parameters_schema()["properties"]["query"].is_object());
}

#[tokio::test]
async fn no_tools_when_search_is_off_or_nothing_is_usable() {
    let mut config = signed_out_byok_config();
    config.search.enabled = Some(false);
    assert!(build_search_tools(&config).await.is_empty());

    let mut managed_only = Config::default();
    managed_only.search.providers.insert(
        "exa".into(),
        SearchProviderSettings {
            enabled: true,
            route: ProviderRoute::Backend,
        },
    );
    // No backend credential in a default test config.
    assert!(build_search_tools(&managed_only).await.is_empty());
}

#[test]
fn recorded_tools_keep_their_declaration() {
    let spec = ToolSpec {
        name: "web_answer_tool".into(),
        description: "Grounded answers".into(),
        parameters: serde_json::json!({"type": "object"}),
    };
    let tool = TinySearchTool::recorded(spec.clone());
    assert_eq!(tool.name(), "web_answer_tool");
    assert_eq!(tool.spec(), &spec);
    assert_eq!(tool.exposure(), ToolExposure::Direct);
}

#[tokio::test]
async fn unavailable_privacy_module_refuses_external_search_tool_dispatch() {
    let _privacy =
        crate::security::live_policy::test_privacy_scope(crate::config::PrivacyMode::LocalOnly);

    let message = local_only_search_block("web_search_tool")
        .await
        .expect("search requests must be blocked in LocalOnly mode");
    assert!(message.contains(tinytools::status::POLICY_BLOCKED_MARKER));
    assert!(message.contains("tinysearch"));
}

#[tokio::test]
async fn standard_privacy_mode_refuses_when_the_module_cannot_answer() {
    let _privacy =
        crate::security::live_policy::test_privacy_scope(crate::config::PrivacyMode::Standard);

    assert!(local_only_search_block("web_search_tool").await.is_some());
}

// ---------------------------------------------------------------------------
// #6991: a credential alone makes the managed route look reachable, so a
// deployment that is offline, firewalled, out of balance or holding a dead key
// offers these tools on every turn and fails every call. The old message ended
// "unavailable right now", which reads as "retry later": in the DeepSWE-10 run
// the agent called search three times on one task, then guessed, and the guess
// was what the hidden test rejected.
// ---------------------------------------------------------------------------

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: "search".into(),
        parameters: serde_json::json!({"type": "object"}),
    }
}

/// A config whose managed providers are selected but whose direct keys are
/// absent, which is the shape that offers the tools and cannot serve them.
fn nothing_usable_config() -> Config {
    let mut config = Config::default();
    config.search.providers = [("brave".to_string(), SearchProviderSettings::direct())]
        .into_iter()
        .collect();
    config
}

#[test]
fn the_unavailable_verdict_tells_the_model_to_stop_rather_than_wait() {
    let message = user_facing_error("tinysearch.provider_unavailable: all down");

    assert!(
        message.contains("Do not call it again"),
        "the model must be told to stop, got: {message}"
    );
    assert!(
        !message.contains("right now"),
        "nothing the model can do will change the answer, so it must not read as a retry hint: {message}"
    );
}

#[tokio::test]
async fn a_tool_starts_with_nothing_recorded() {
    let config = nothing_usable_config();
    let tool = TinySearchTool::recorded(spec("web_search_tool"));

    assert!(!tool.is_exhausted_for(tool.provider_signature(&config).await));
}

#[tokio::test]
async fn the_refusal_answers_only_for_the_configuration_that_failed() {
    // The contract this module documents is that every call re-reads the live
    // config, so a provider or login change is honoured without rebuilding the
    // session. A refusal that outlived a config change would break it: the user
    // adds a key and search stays dead until the thread is abandoned.
    let dead = nothing_usable_config();
    let tool = TinySearchTool::recorded(spec("web_search_tool"));
    let dead_signature = tool.provider_signature(&dead).await;

    tool.mark_exhausted_for(dead_signature);
    assert!(tool.is_exhausted_for(dead_signature));

    let mut fixed = dead.clone();
    fixed.search.brave.api_key = Some("added-after-the-failure".into());

    assert_ne!(
        tool.provider_signature(&fixed).await,
        dead_signature,
        "adding a provider key must change what the tool sees"
    );
    assert!(
        !tool.is_exhausted_for(tool.provider_signature(&fixed).await),
        "adding a key must clear the refusal"
    );
}

#[tokio::test]
async fn one_tools_dead_providers_do_not_answer_for_another() {
    // `build_search_tools` can offer provider-specific tools beside the role
    // tools, and the roles draw on different provider orders, so a shared
    // verdict would let one provider's outage disable unrelated ones.
    let config = nothing_usable_config();
    let failed = TinySearchTool::recorded(spec("web_search_tool"));
    let other = TinySearchTool::recorded(spec("web_answer_tool"));

    failed.mark_exhausted_for(failed.provider_signature(&config).await);

    assert!(!other.is_exhausted_for(other.provider_signature(&config).await));
}

#[tokio::test]
async fn a_refused_call_does_not_reach_the_module() {
    // The second call under the same configuration is answered from the
    // recorded verdict, so a dead search costs one module round trip per
    // configuration rather than one per call.
    let config = nothing_usable_config();
    let tool = TinySearchTool::new(std::sync::Arc::new(config.clone()), spec("web_search_tool"));
    tool.mark_exhausted_for(tool.provider_signature(&config).await);

    let result = tool
        .execute(serde_json::json!({"query": "anything"}))
        .await
        .expect("a refusal is a reported failure, not a tool error");

    assert!(result.is_error);
    assert_eq!(
        result.error_kind,
        Some(tinytools::ToolErrorKind::Failed),
        "the failure is permanent for this configuration, so it must not be tagged retryable"
    );
    assert!(
        result.text().contains("Do not call it again"),
        "{:?}",
        result.text()
    );
}

#[test]
fn a_rejected_provider_key_names_the_provider_and_the_fix() {
    let message = user_facing_error(
        "search ExecuteTool failed: tinysearch.provider_unauthorized: tavily rejected the \
         configured API key (HTTP 401)",
    );
    assert_eq!(
        message,
        "Web search is unavailable: tavily rejected the configured API key (HTTP 401). Update \
         the key under Connections → Search, or remove it to use managed search."
    );
    assert!(!tinysearch_bus::errors::exhausts_providers(
        "tinysearch.provider_unauthorized: tavily rejected the configured API key (HTTP 401)"
    ));
}

#[test]
fn a_rejected_backend_credential_asks_for_the_right_fix() {
    assert!(
        user_facing_error("tinysearch.backend_unauthorized: rejected")
            .contains("sign-in has expired")
    );
    assert!(backend_unauthorized_message(false).contains("Sign in again"));
    let key = backend_unauthorized_message(true);
    assert!(key.contains("rejected the configured API key"), "{key}");
    assert!(!key.contains("Sign in again"), "{key}");
    assert!(!tinysearch_bus::errors::exhausts_providers(
        "tinysearch.backend_unauthorized: rejected"
    ));
}

/// Drain `rx` for a `SessionExpired` whose source is `source`.
fn session_expired_from(
    rx: &mut tinybus::EventReceiver<crate::core::events::DomainEvent>,
    source: &str,
) -> bool {
    loop {
        match rx.try_recv() {
            Ok(crate::core::events::DomainEvent::SessionExpired { source: seen, .. })
                if seen == source =>
            {
                return true;
            }
            Ok(_) | Err(tinybus::TryRecvError::Lagged(_)) => continue,
            Err(_) => return false,
        }
    }
}

/// A user on their own inference key never reached a backend call that
/// noticed a dead session, so search kept failing on it with no sign-in
/// prompt. A managed-search 401 on the session now starts re-auth itself.
#[tokio::test]
async fn a_rejected_session_starts_re_authentication() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS
        .get()
        .expect("event bus initialized")
        .receiver();
    on_backend_unauthorized(
        "test_marker_session_search",
        false,
        Some(crate::credentials::backend_credential::credential_fingerprint("test-session-token")),
    );
    assert!(session_expired_from(
        &mut rx,
        "search.test_marker_session_search"
    ));
}

/// A rejected TinyHumans API key has no session to expire, so it must never
/// sign the user out.
#[tokio::test]
async fn a_rejected_api_key_does_not_sign_the_user_out() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS
        .get()
        .expect("event bus initialized")
        .receiver();
    on_backend_unauthorized("test_marker_api_key_search", true, None);
    assert!(!session_expired_from(
        &mut rx,
        "search.test_marker_api_key_search"
    ));
}

#[tokio::test]
async fn components_search_off_registers_no_search_tools() {
    let mut config = signed_out_byok_config();
    assert!(!build_search_tools(&config).await.is_empty());
    config.components.search = false;
    assert!(build_search_tools(&config).await.is_empty());
    assert!(crate::modules::search::configured_tool_specs(&config)
        .await
        .is_empty());
}
