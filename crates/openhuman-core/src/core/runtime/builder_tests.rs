use super::{CoreBootMode, CoreBuilder, DomainSet, ServiceSet};
use crate::core::all::DomainGroup;
use crate::core::types::HostKind;

#[test]
fn core_builder_defaults_to_normal_boot_mode() {
    assert_eq!(
        CoreBuilder::new(HostKind::Library).boot_mode,
        CoreBootMode::Normal
    );
}

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
    assert!(services.memory_sync);
}

#[test]
fn boot_jobs_are_independent_from_runtime_service_flags() {
    let mut custom = ServiceSet::none();
    custom.rpc_http = true;
    custom.login_gated = true;
    custom.update_scheduler = true;
    assert!(!custom.skill_catalog_refresh);
    assert!(!custom.mcp_boot);
    assert!(!custom.integrations);
    assert!(!custom.memory_sync);

    let desktop = ServiceSet::desktop();
    assert!(desktop.skill_catalog_refresh);
    assert!(desktop.mcp_boot);
    assert!(desktop.integrations);
    assert!(desktop.memory_sync);

    // headless_api() runs no bootstrap jobs either.
    let headless = ServiceSet::headless_api();
    assert!(!headless.integrations);
    assert!(!headless.memory_sync);
}

/// Inspection boot refuses to start without a caller-supplied config and a
/// fixed bearer, before it touches any process state.
#[tokio::test]
async fn inspection_boot_rejects_missing_config_and_env_token() {
    let refusal = |result: anyhow::Result<super::CoreRuntime>| match result {
        Ok(_) => panic!("inspection boot must refuse"),
        Err(error) => error.to_string(),
    };
    let inspection = || CoreBuilder::new(HostKind::Cli).boot_mode(CoreBootMode::Inspection);

    let error = refusal(inspection().build().await);
    assert!(error.contains("caller-supplied config"), "{error}");

    let error = refusal(
        inspection()
            .config(crate::config::Config::default())
            .build()
            .await,
    );
    assert!(error.contains("fixed RPC bearer"), "{error}");
}

/// Every flag of every public `ServiceSet` preset, pinned. The host presets on
/// `openhuman_embed::RuntimeBuilder` (desktop, cli, tui, library) and the rpc
/// host builders start from these, so a silent flip here changes what a host
/// runs.
#[test]
fn service_set_presets_pin_every_flag() {
    let all_on = ServiceSet {
        rpc_http: true,
        socketio: true,
        cron: true,
        channels: true,
        login_gated: true,
        update_scheduler: true,
        memory_queue: true,
        skill_catalog_refresh: true,
        mcp_boot: true,
        integrations: true,
        memory_sync: true,
    };
    let all_off = ServiceSet {
        rpc_http: false,
        socketio: false,
        cron: false,
        channels: false,
        login_gated: false,
        update_scheduler: false,
        memory_queue: false,
        skill_catalog_refresh: false,
        mcp_boot: false,
        integrations: false,
        memory_sync: false,
    };
    assert_eq!(ServiceSet::desktop(), all_on);
    assert_eq!(ServiceSet::none(), all_off);
    assert_eq!(
        ServiceSet::headless_api(),
        ServiceSet {
            rpc_http: true,
            ..all_off
        }
    );
    assert_eq!(
        ServiceSet::embedded(),
        ServiceSet {
            cron: true,
            login_gated: true,
            memory_queue: true,
            skill_catalog_refresh: true,
            memory_sync: true,
            ..all_off
        }
    );
    assert_eq!(
        ServiceSet::secret_preflight(),
        ServiceSet {
            rpc_http: true,
            ..all_off
        }
    );
}
