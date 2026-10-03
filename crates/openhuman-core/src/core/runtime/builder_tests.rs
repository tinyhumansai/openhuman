use super::{DomainSet, ServiceSet};
use crate::core::all::DomainGroup;

#[test]
fn domain_set_presets_have_expected_flags() {
    // full() = every family on (byte-identical registration).
    let full = DomainSet::full();
    for group in [
        DomainGroup::Agent,
        DomainGroup::Memory,
        DomainGroup::Threads,
        DomainGroup::Config,
        DomainGroup::Security,
        DomainGroup::Flows,
        DomainGroup::Skills,
        DomainGroup::Mcp,
        DomainGroup::Channels,
        DomainGroup::Web3,
        DomainGroup::Voice,
        DomainGroup::Media,
        DomainGroup::Integrations,
        DomainGroup::Platform,
    ] {
        assert!(full.allows(group), "full() must allow {group:?}");
    }

    // harness() = exactly agent/memory/threads/config/security on; all gate
    // families AND platform off.
    let harness = DomainSet::harness();
    for on in [
        DomainGroup::Agent,
        DomainGroup::Memory,
        DomainGroup::Threads,
        DomainGroup::Config,
        DomainGroup::Security,
    ] {
        assert!(harness.allows(on), "harness() must allow {on:?}");
    }
    for off in [
        DomainGroup::Flows,
        DomainGroup::Skills,
        DomainGroup::Mcp,
        DomainGroup::Channels,
        DomainGroup::Web3,
        DomainGroup::Voice,
        DomainGroup::Media,
        DomainGroup::Platform,
    ] {
        assert!(!harness.allows(off), "harness() must NOT allow {off:?}");
    }

    // none() = every family off.
    let none = DomainSet::none();
    for group in [
        DomainGroup::Agent,
        DomainGroup::Memory,
        DomainGroup::Threads,
        DomainGroup::Config,
        DomainGroup::Security,
        DomainGroup::Flows,
        DomainGroup::Skills,
        DomainGroup::Mcp,
        DomainGroup::Channels,
        DomainGroup::Web3,
        DomainGroup::Voice,
        DomainGroup::Media,
        DomainGroup::Platform,
    ] {
        assert!(!none.allows(group), "none() must NOT allow {group:?}");
    }

    // Spot-check the field/group wiring is not transposed.
    assert!(DomainSet::harness().allows(DomainGroup::Memory));
    assert!(!DomainSet::harness().allows(DomainGroup::Web3));
}

#[test]
fn embedded_domain_set_enables_the_host_families() {
    let set = DomainSet::embedded();

    for on in [
        DomainGroup::Agent,
        DomainGroup::Memory,
        DomainGroup::Threads,
        DomainGroup::Config,
        DomainGroup::Security,
        DomainGroup::Platform,
    ] {
        assert!(set.allows(on), "embedded() must allow {on:?}");
    }

    for off in [
        DomainGroup::Mcp,
        DomainGroup::Web3,
        DomainGroup::Voice,
        DomainGroup::Media,
    ] {
        assert!(!set.allows(off), "embedded() must NOT allow {off:?}");
    }
}

#[test]
fn embedded_keeps_flows_on_for_workflow_boot_reconcile() {
    // Boot reconciliation keys off `ctx.domains().flows`.
    assert!(DomainSet::embedded().allows(DomainGroup::Flows));
}

#[test]
fn embedded_keeps_channels_on_for_web_chat() {
    // `channel.web_chat` is tagged DomainGroup::Channels and the TUI drives
    // chat turns through it. This is precisely why embedded() is not
    // built on harness(), which leaves channels off.
    assert!(DomainSet::embedded().allows(DomainGroup::Channels));
}

#[test]
fn embedded_retains_supporting_domains() {
    // Guards the most tempting future "simplification": deriving this
    // preset from harness(), which leaves the supporting Platform,
    // Channels, and Integrations families off.
    let harness = DomainSet::harness();
    let tui = DomainSet::embedded();

    assert!(!harness.allows(DomainGroup::Platform));
    assert!(tui.allows(DomainGroup::Platform));
    assert!(!harness.allows(DomainGroup::Channels));
    assert!(tui.allows(DomainGroup::Channels));
    assert!(!harness.allows(DomainGroup::Integrations));
    assert!(tui.allows(DomainGroup::Integrations));
}

#[test]
fn embedded_service_set_binds_no_transport() {
    // The whole point of the typed facade: the host talks to the core
    // in-process, so no port, no bearer handshake, no loopback listener.
    let services = ServiceSet::embedded();

    assert!(!services.rpc_http, "embedded() must not bind HTTP");
    assert!(!services.socketio, "embedded() must not mount Socket.IO");

    // But a long-lived operator session still wants background work.
    assert!(services.cron);
    assert!(services.login_gated);
    assert!(services.harness_init);
    assert!(services.memory_sync);
}

#[test]
fn boot_jobs_are_independent_from_runtime_service_flags() {
    let mut custom = ServiceSet::none();
    custom.rpc_http = true;
    custom.login_gated = true;
    custom.update_scheduler = true;
    assert!(!custom.harness_init);
    assert!(!custom.skill_catalog_refresh);
    assert!(!custom.mcp_boot);
    assert!(!custom.integrations);
    assert!(!custom.memory_sync);

    let desktop = ServiceSet::desktop();
    assert!(desktop.harness_init);
    assert!(desktop.skill_catalog_refresh);
    assert!(desktop.mcp_boot);
    assert!(desktop.integrations);
    assert!(desktop.memory_sync);

    // headless_api() runs no bootstrap jobs either.
    let headless = ServiceSet::headless_api();
    assert!(!headless.integrations);
    assert!(!headless.memory_sync);
}
