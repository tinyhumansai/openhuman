use super::provider_role_for;
use super::{resolve_dispatcher_kind, DispatcherKind};

#[test]
fn legacy_orchestrator_fallback_defaults_to_chat() {
    assert_eq!(provider_role_for("orchestrator", Some("chat-v1")), "chat");
    assert_eq!(provider_role_for("orchestrator", None), "chat");
    // A legacy heavy default_model tier still falls through to chat.
    assert_eq!(
        provider_role_for("orchestrator", Some("reasoning-v1")),
        "chat"
    );
}

#[test]
fn explicit_hints_route_to_workload() {
    assert_eq!(
        provider_role_for("orchestrator", Some("hint:agentic")),
        "agentic"
    );
    assert_eq!(
        provider_role_for("orchestrator", Some("hint:reasoning")),
        "reasoning"
    );
    assert_eq!(
        provider_role_for("orchestrator", Some("hint:coding")),
        "coding"
    );
    // The cloud tick: orchestrator agent_id + the subconscious hint.
    assert_eq!(
        provider_role_for("orchestrator", Some("hint:subconscious")),
        "subconscious"
    );
}

#[test]
fn subconscious_agent_id_routes_to_subconscious_without_hint() {
    // The event-driven long-lived session builds with agent_id="subconscious"
    // and no hint — it must still resolve the subconscious workload (Codex P2).
    assert_eq!(provider_role_for("subconscious", None), "subconscious");
    assert_eq!(
        provider_role_for("subconscious", Some("chat-v1")),
        "subconscious"
    );
    assert_eq!(provider_role_for(" subconscious ", None), "subconscious");
}

#[test]
fn auto_prefers_native_when_supported_never_pformat() {
    assert_eq!(
        resolve_dispatcher_kind("auto", true, "chat"),
        DispatcherKind::Native
    );
    // Text-only provider defaults to JSON-in-tag, NOT P-Format.
    assert_eq!(
        resolve_dispatcher_kind("auto", false, "chat"),
        DispatcherKind::Xml
    );
    // An unrecognized value behaves like "auto".
    assert_eq!(
        resolve_dispatcher_kind("bogus", false, "chat"),
        DispatcherKind::Xml
    );
}

#[test]
fn explicit_choices_are_honoured_including_opt_in_pformat() {
    assert_eq!(
        resolve_dispatcher_kind("native", false, "chat"),
        DispatcherKind::Native
    );
    assert_eq!(
        resolve_dispatcher_kind("xml", true, "chat"),
        DispatcherKind::Xml
    );
    // P-Format is only ever selected when explicitly requested.
    assert_eq!(
        resolve_dispatcher_kind("pformat", true, "chat"),
        DispatcherKind::PFormat
    );
}

#[test]
fn integrations_agent_falls_off_native_to_json_in_tag() {
    // Native would ship JSON tool specs and blow the provider grammar-rule
    // ceiling on large Composio toolkits → force JSON-in-tag.
    assert_eq!(
        resolve_dispatcher_kind("auto", true, "integrations_agent"),
        DispatcherKind::Xml
    );
    assert_eq!(
        resolve_dispatcher_kind("native", true, "integrations_agent"),
        DispatcherKind::Xml
    );
    // An explicit non-native choice is left untouched for that agent.
    assert_eq!(
        resolve_dispatcher_kind("pformat", true, "integrations_agent"),
        DispatcherKind::PFormat
    );
}
