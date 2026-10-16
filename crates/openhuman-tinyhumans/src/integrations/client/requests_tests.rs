use super::*;

#[test]
fn control_plane_exempts_non_tool_namespace() {
    // A control-plane call re-homed onto an integration descriptor: anything
    // not under /agent-integrations/ is exempt.
    for path in [
        "/teams/me/usage",
        "/payments/stripe/currentPlan",
        "/auth/refresh",
    ] {
        assert!(
            backend_egress_descriptor(path).control_plane,
            "{path} must be treated as control-plane"
        );
    }
}

#[test]
fn control_plane_exempts_composio_management_and_pricing() {
    // Connection-management / catalog / OAuth / pricing carry no user content.
    for path in [
        "/agent-integrations/pricing",
        "/agent-integrations/composio/connections",
        // Per-connection delete rides the same `connections` head.
        "/agent-integrations/composio/connections/conn_123",
        "/agent-integrations/composio/authorize",
        "/agent-integrations/composio/tools",
        "/agent-integrations/composio/toolkits",
        // The query is stripped before classification.
        "/agent-integrations/composio/tools?toolkits=gmail",
    ] {
        assert!(
            backend_egress_descriptor(path).control_plane,
            "{path} must be exempt (control-plane)"
        );
    }
}

#[test]
fn control_plane_does_not_exempt_user_data_paths() {
    for path in [
        "/agent-integrations/composio/execute",
        "/agent-integrations/composio/triggers",
        "/agent-integrations/composio/triggers/available",
        "/agent-integrations/composio/github/repos",
        // An unknown composio sub-route defaults to blocked (fail-closed).
        "/agent-integrations/composio/some-future-write",
        "/agent-integrations/parallel/research",
        "/agent-integrations/tinyfish/fetch",
        "/agent-integrations/file-storage/files",
        "/agent-integrations/google-places/search",
        "/agent-integrations/openrouter/images",
    ] {
        assert!(
            !backend_egress_descriptor(path).control_plane,
            "{path} must NOT be exempt (ships user data)"
        );
    }
}

#[test]
fn local_only_keeps_control_plane_flowing_and_blocks_user_data() {
    use openhuman_embed::__host::config::PrivacyMode;
    use openhuman_embed::__host::egress::local_only_blocks;
    let pricing = backend_egress_descriptor("/agent-integrations/pricing");
    assert!(!local_only_blocks(PrivacyMode::LocalOnly, &pricing));
    let execute = backend_egress_descriptor("/agent-integrations/composio/execute");
    assert!(local_only_blocks(PrivacyMode::LocalOnly, &execute));
}
