use super::*;

fn integration(toolkit: &str) -> ConnectedIntegration {
    ConnectedIntegration {
        toolkit: toolkit.into(),
        description: String::new(),
        tools: vec![],
        gated_tools: vec![],
        connected: true,
        connections: Vec::new(),
        non_active_status: None,
    }
}

fn toolkits(list: &[ConnectedIntegration]) -> Vec<String> {
    list.iter().map(|i| i.toolkit.clone()).collect()
}

#[test]
fn authoritative_result_is_taken_verbatim_even_when_empty() {
    // The backend confirming "zero connections" is truth — do NOT paper over
    // it with a stale cache, or the agent would advertise integrations the
    // user actually disconnected.
    let out = connected_with_fallback(
        Some(FetchConnectedIntegrationsStatus::Authoritative(vec![])),
        Some(vec![integration("gmail")]),
    );
    assert!(out.is_empty());

    let out = connected_with_fallback(
        Some(FetchConnectedIntegrationsStatus::Authoritative(vec![
            integration("gmail"),
            integration("slack"),
        ])),
        None,
    );
    assert_eq!(toolkits(&out), vec!["gmail", "slack"]);
}

#[test]
fn unavailable_falls_back_to_cached_snapshot() {
    // A transient backend failure must not drop the delegation surface.
    let out = connected_with_fallback(
        Some(FetchConnectedIntegrationsStatus::Unavailable),
        Some(vec![integration("gmail")]),
    );
    assert_eq!(toolkits(&out), vec!["gmail"]);
}

#[test]
fn timeout_falls_back_to_cached_snapshot() {
    // `None` models the dispatch timeout elapsing before the fetch returned.
    let out = connected_with_fallback(None, Some(vec![integration("notion")]));
    assert_eq!(toolkits(&out), vec!["notion"]);
}

#[test]
fn unavailable_without_cache_is_empty() {
    // No authoritative truth and no cache → conservative empty set (same
    // default as before, but only when we genuinely have nothing better).
    assert!(
        connected_with_fallback(Some(FetchConnectedIntegrationsStatus::Unavailable), None)
            .is_empty()
    );
    assert!(connected_with_fallback(None, None).is_empty());
}
