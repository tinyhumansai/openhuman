//! Tests for local-only egress enforcement (privacy epic S7, #4441).
//!
//! The pure [`local_only_blocks`] / [`is_control_plane`] truth tables need no
//! process-global state. The two side-effecting wrappers ([`enforce_egress`],
//! [`local_only_tool_block`]) read the live policy, so those tests install a
//! LocalOnly / Standard policy under [`TEST_ENV_LOCK`] (shared with the other
//! `live_policy`-touching tests so installs never race) and restore Standard on
//! the way out.

use super::*;
use crate::openhuman::config::PrivacyMode;
use crate::openhuman::security::egress::{DataKind, EgressDescriptor, EgressReason};

// ── Pure decision: local_only_blocks truth table ──────────────────────────

fn external_composio() -> EgressDescriptor {
    EgressDescriptor::composio("GITHUB_CREATE_ISSUE")
}

#[test]
fn local_only_blocks_external_transfer() {
    // LocalOnly + external user-data transfer → blocked.
    assert!(local_only_blocks(
        PrivacyMode::LocalOnly,
        &external_composio()
    ));
}

#[test]
fn local_only_blocks_each_external_surface() {
    // AC7: the block applies across every egress surface's user-data descriptor.
    let surfaces = [
        EgressDescriptor::inference("openai", "gpt-4o", true),
        EgressDescriptor::composio("SLACK_SEND_MESSAGE"),
        EgressDescriptor::embedding("cloud", "text-embedding-3-small"),
        EgressDescriptor::network_fetch("api.example.com"),
        EgressDescriptor::integration("/agent-integrations/parallel/research"),
    ];
    for desc in &surfaces {
        assert!(
            local_only_blocks(PrivacyMode::LocalOnly, desc),
            "surface {:?}/{} must block under LocalOnly",
            desc.reason,
            desc.service
        );
        assert!(
            !local_only_blocks(PrivacyMode::Standard, desc),
            "surface {:?}/{} must be allowed under Standard",
            desc.reason,
            desc.service
        );
    }
}

#[test]
fn standard_mode_allows_everything() {
    // Standard / Sensitive never block here.
    assert!(!local_only_blocks(
        PrivacyMode::Standard,
        &external_composio()
    ));
    assert!(!local_only_blocks(
        PrivacyMode::Sensitive,
        &external_composio()
    ));
}

#[test]
fn local_only_allows_local_runtime() {
    // A non-external transfer (local runtime — Ollama/LM Studio/etc.) is never
    // blocked: nothing leaves the device.
    let local = EgressDescriptor::inference("ollama", "llama3", false);
    assert!(!local.is_external);
    assert!(!local_only_blocks(PrivacyMode::LocalOnly, &local));
}

#[test]
fn local_only_allows_control_plane_integration() {
    // LocalOnly + an exempt control-plane integration path → allowed.
    let pricing = EgressDescriptor::integration("/agent-integrations/pricing");
    assert!(!local_only_blocks(PrivacyMode::LocalOnly, &pricing));
}

#[test]
fn local_only_blocks_user_data_integration() {
    // LocalOnly + a user-data integration path → blocked.
    let execute = EgressDescriptor::integration("/agent-integrations/composio/execute");
    assert!(local_only_blocks(PrivacyMode::LocalOnly, &execute));
}

// ── Pure decision: is_control_plane boundary ──────────────────────────────

#[test]
fn control_plane_exempts_non_tool_namespace() {
    // A control-plane call re-homed onto an integration descriptor (defensive
    // path (1)): anything not under /agent-integrations/ is exempt.
    for path in [
        "/teams/me/usage",
        "/payments/stripe/currentPlan",
        "/auth/refresh",
    ] {
        assert!(
            is_control_plane(&EgressDescriptor::integration(path)),
            "{path} must be treated as control-plane"
        );
    }
}

#[test]
fn control_plane_exempts_composio_management_and_pricing() {
    // Connection-management / catalog / OAuth / pricing carry no user content —
    // the read-only allow-list the Connections UI needs under LocalOnly.
    for path in [
        "/agent-integrations/pricing",
        "/agent-integrations/composio/connections",
        // Per-connection delete rides the same `connections` head → exempt.
        "/agent-integrations/composio/connections/conn_123",
        "/agent-integrations/composio/authorize",
        "/agent-integrations/composio/tools",
        "/agent-integrations/composio/toolkits",
    ] {
        assert!(
            is_control_plane(&EgressDescriptor::integration(path)),
            "{path} must be exempt (control-plane)"
        );
    }
}

#[test]
fn control_plane_does_not_exempt_user_data_paths() {
    // composio/execute ships tool arguments; composio/triggers[/available]
    // POST user-supplied slug/connectionId/triggerConfig (user-data writes);
    // github/repos reveals user-adjacent data; the non-composio tool namespaces
    // ship queries / content — none are exempt (fail-closed).
    for path in [
        "/agent-integrations/composio/execute",
        // Trigger writes: create_trigger / enable_trigger POST here. The
        // descriptor carries no HTTP method, so the same-path reads block too.
        "/agent-integrations/composio/triggers",
        "/agent-integrations/composio/triggers/available",
        "/agent-integrations/composio/github/repos",
        // An unknown composio sub-route defaults to blocked (fail-closed).
        "/agent-integrations/composio/some-future-write",
        "/agent-integrations/parallel/research",
        "/agent-integrations/tinyfish/fetch",
        "/agent-integrations/twilio/call",
        "/agent-integrations/file-storage/files",
        "/agent-integrations/google-places/search",
    ] {
        assert!(
            !is_control_plane(&EgressDescriptor::integration(path)),
            "{path} must NOT be exempt (ships user data)"
        );
    }
}

#[test]
fn control_plane_only_applies_to_integration_reason() {
    // A network fetch whose host happens to spell a backend path is still a
    // user-data transfer, never control-plane.
    let net = EgressDescriptor::network_fetch("agent-integrations");
    assert_eq!(net.reason, EgressReason::NetworkFetch);
    assert!(!is_control_plane(&net));
}

// ── Side-effecting wrappers (read the live policy) ────────────────────────
//
// These use the thread-scoped `test_privacy_scope` override rather than
// installing into the process-global policy, so they never race sibling tests
// that read `current_privacy_mode` on other threads (see `live_policy`).

use crate::openhuman::security::live_policy::test_privacy_scope;

#[test]
fn enforce_egress_blocks_under_local_only_and_allows_otherwise() {
    {
        let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
        // A user-data transfer is refused with a clean, service-naming message.
        let err = enforce_egress(&external_composio()).expect_err("must block under LocalOnly");
        let msg = err.to_string();
        assert!(msg.contains("Local-only privacy mode is active"), "{msg}");
        assert!(
            msg.contains("GITHUB_CREATE_ISSUE"),
            "names the service: {msg}"
        );
        // A control-plane integration round-trip still flows.
        enforce_egress(&EgressDescriptor::integration(
            "/agent-integrations/composio/connections",
        ))
        .expect("control-plane must be allowed under LocalOnly");
    }

    // Standard mode permits the same user-data transfer.
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    enforce_egress(&external_composio()).expect("Standard mode must allow");
}

#[test]
fn local_only_tool_block_marks_message_and_clears_when_allowed() {
    let mut desc = EgressDescriptor::network_fetch("api.example.com");
    desc = desc.with_data_kind(DataKind::ToolArguments);

    {
        let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
        let msg = local_only_tool_block(&desc).expect("network fetch blocked under LocalOnly");
        assert!(
            msg.starts_with(crate::openhuman::security::POLICY_BLOCKED_MARKER),
            "tool block must carry the policy-blocked marker: {msg}"
        );
        assert!(msg.contains("api.example.com"), "names the host: {msg}");
    }

    // Standard mode → no block.
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    assert!(local_only_tool_block(&desc).is_none());
}
