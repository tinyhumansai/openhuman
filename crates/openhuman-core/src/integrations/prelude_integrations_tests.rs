use super::*;
use std::collections::HashSet;

fn item(toolkit: &str, connected: bool) -> crate::integrations::ConnectedIntegration {
    crate::integrations::ConnectedIntegration {
        toolkit: toolkit.into(),
        description: String::new(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected,
        connections: Vec::new(),
        non_active_status: None,
    }
}

#[test]
fn only_connected_toolkits_seed_the_announced_set() {
    let items = vec![
        item("gmail", true),
        item("notion", false),
        item("slack", false),
    ];
    let seeded = connected_toolkit_slugs(&items);
    assert_eq!(seeded, HashSet::from(["gmail".to_string()]));
}

#[test]
fn allowlisted_but_unconnected_toolkits_are_never_announced() {
    let mut announced = HashSet::new();
    let mut pending = Vec::new();
    // The backend lists every allowlisted toolkit; only two are connected.
    let mut current: Vec<_> = (0..130)
        .map(|i| item(&format!("toolkit_{i:03}"), false))
        .collect();
    current.push(item("gmail", true));
    current.push(item("github", true));

    merge_integration_announcements(&mut announced, &mut pending, &current);

    assert_eq!(pending, vec!["github".to_string(), "gmail".to_string()]);
    assert_eq!(
        announced,
        HashSet::from(["gmail".to_string(), "github".to_string()])
    );
}

#[test]
fn a_new_connection_is_announced_once_and_a_revoke_is_dropped() {
    let mut announced = HashSet::from(["gmail".to_string()]);
    let mut pending = Vec::new();

    // Notion becomes connected; gmail is revoked (still listed, not connected).
    let current = vec![
        item("gmail", false),
        item("notion", true),
        item("slack", false),
    ];
    merge_integration_announcements(&mut announced, &mut pending, &current);
    assert_eq!(pending, vec!["notion".to_string()]);
    assert_eq!(announced, HashSet::from(["notion".to_string()]));

    // The same snapshot again queues nothing new.
    merge_integration_announcements(&mut announced, &mut pending, &current);
    assert_eq!(pending, vec!["notion".to_string()]);
}

#[test]
fn a_pending_announcement_for_a_revoked_toolkit_is_withdrawn() {
    let mut announced = HashSet::from(["notion".to_string()]);
    let mut pending = vec!["notion".to_string()];
    merge_integration_announcements(&mut announced, &mut pending, &[item("notion", false)]);
    assert!(pending.is_empty());
    assert!(announced.is_empty());
}

#[test]
fn a_toolkit_connected_between_a_stale_and_an_authoritative_hydration_is_announced() {
    let mut state = super::super::OpenHumanTurnPreludeMutable::default();

    // Turn 1: the backend is unreachable, the stale snapshot has gmail only.
    apply_cold_hydration(
        &mut state,
        vec![item("gmail", true), item("notion", false)],
        false,
        HashSet::from(["mcp_a".to_string()]),
    );
    assert!(!state.connected_integrations_initialized);
    assert_eq!(
        state.announced_integrations,
        HashSet::from(["gmail".into()])
    );
    assert!(state.pending_integration_announcement.is_empty());

    // Turn 2: the live fetch succeeds and notion is now connected, as is a
    // new MCP server. Both are news to the model and must be queued.
    apply_cold_hydration(
        &mut state,
        vec![item("gmail", true), item("notion", true)],
        true,
        HashSet::from(["mcp_a".to_string(), "mcp_b".to_string()]),
    );
    assert!(state.connected_integrations_initialized);
    assert!(state.connected_integrations_authoritative);
    assert_eq!(
        state.pending_integration_announcement,
        vec!["notion".to_string()]
    );
    assert_eq!(state.pending_mcp_announcement, vec!["mcp_b".to_string()]);
    assert_eq!(state.connected_integrations.len(), 2);
}

#[test]
fn the_first_hydration_seeds_without_announcing() {
    let mut state = super::super::OpenHumanTurnPreludeMutable::default();
    apply_cold_hydration(
        &mut state,
        vec![item("gmail", true)],
        true,
        HashSet::from(["mcp_a".to_string()]),
    );
    assert_eq!(
        state.announced_integrations,
        HashSet::from(["gmail".into()])
    );
    assert!(state.pending_integration_announcement.is_empty());
    assert!(state.pending_mcp_announcement.is_empty());
}
