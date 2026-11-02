use super::{group_first_time_when_bus_ready, DomainSubscriberPlan};
use crate::config::test_env::EnvVarGuard;

// ---- domain-subscriber gating (#4796 DoD item 3) ----------------------------
// `register_domain_subscribers` registers on the process-global event bus behind
// a `Once`, so its gating is proven via the pure `DomainSubscriberPlan` the
// registrar consumes — no real subscribers, no bus mutation.

#[test]
fn domain_subscriber_plan_full_registers_every_gated_subscriber() {
    let plan = DomainSubscriberPlan::for_domains(crate::core::runtime::DomainSet::full());
    assert_eq!(
        plan,
        DomainSubscriberPlan {
            platform: true,
            integrations: true,
            security: true,
            desktop: true,
            skills: true,
            channels: true,
            flows: true,
            memory: true,
            threads: true,
            agent: true,
            hosted: true,
            mcp: true,
        },
        "full() must register every gated domain subscriber"
    );
}

#[test]
fn domain_subscriber_plan_none_registers_no_gated_subscriber() {
    let plan = DomainSubscriberPlan::for_domains(crate::core::runtime::DomainSet::none());
    assert_eq!(
        plan,
        DomainSubscriberPlan {
            platform: false,
            integrations: false,
            security: false,
            desktop: false,
            skills: false,
            channels: false,
            flows: false,
            memory: false,
            threads: false,
            agent: false,
            hosted: false,
            mcp: false,
        },
        "none() must register no gated domain subscriber (core infra still runs, ungated)"
    );
}

#[test]
fn domain_subscriber_plan_harness_gates_by_owning_group() {
    let plan = DomainSubscriberPlan::for_domains(crate::core::runtime::DomainSet::harness());
    // harness() = agent + memory + threads + config + security.
    assert!(plan.agent, "harness keeps agent subscribers");
    assert!(
        plan.memory,
        "harness keeps memory conversation-persistence + sync bridge"
    );
    // Skills, Desktop and Integrations own the subscribers omitted by harness.
    assert!(!plan.skills, "harness must skip the webhook subscriber");
    assert!(!plan.desktop, "harness must skip the notification bridge");
    assert!(
        !plan.integrations,
        "harness must skip composio and task-source subscribers"
    );
    assert!(
        plan.security,
        "harness retains the device-tunnel subscriber"
    );
    assert!(!plan.platform, "harness excludes the platform domain");
    assert!(
        !plan.channels,
        "harness must skip channel-inbound subscriber"
    );
    assert!(!plan.flows, "harness must skip flows trigger dispatch");
    assert!(
        !plan.hosted,
        "harness must skip hosted orchestration ingest"
    );
    assert!(!plan.mcp, "harness must skip mcp_registry bus init");
}

#[test]
fn domain_subscriber_registration_retries_after_bus_becomes_ready() {
    use crate::core::all::DomainGroup;
    use std::collections::HashSet;
    use std::sync::Mutex;

    let completed = Mutex::new(HashSet::new());

    assert!(!group_first_time_when_bus_ready(
        &completed,
        DomainGroup::Flows,
        false,
    ));
    assert!(
        completed.lock().expect("registry lock").is_empty(),
        "a deferred attempt must not mark the group complete"
    );

    assert!(group_first_time_when_bus_ready(
        &completed,
        DomainGroup::Flows,
        true,
    ));
    assert!(
        completed
            .lock()
            .expect("registry lock")
            .contains(&DomainGroup::Flows),
        "the ready retry must mark the group complete"
    );
}

#[test]
fn domain_subscriber_registration_is_idempotent_after_success() {
    use crate::core::all::DomainGroup;
    use std::collections::HashSet;
    use std::sync::Mutex;

    let completed = Mutex::new(HashSet::new());

    assert!(group_first_time_when_bus_ready(
        &completed,
        DomainGroup::Channels,
        true,
    ));
    assert!(!group_first_time_when_bus_ready(
        &completed,
        DomainGroup::Channels,
        true,
    ));
    assert_eq!(
        completed.lock().expect("registry lock").len(),
        1,
        "a completed group must be recorded exactly once"
    );
}

#[test]
fn domain_subscriber_registration_readiness_helper_is_idempotent() {
    use crate::core::all::DomainGroup;
    use std::collections::HashSet;
    use std::sync::Mutex;

    let completed = Mutex::new(HashSet::new());
    assert!(group_first_time_when_bus_ready(
        &completed,
        DomainGroup::Media,
        true
    ));
    assert!(!group_first_time_when_bus_ready(
        &completed,
        DomainGroup::Media,
        true
    ));
}
/// #5027 — the tool-execution timeout must be seeded on the always-on core boot
/// path (`register_domain_subscribers`), NOT inside
/// `channels::runtime::startup::start_channels`, which is skipped for
/// channel-less / web-chat-only cores (and when `OPENHUMAN_DISABLE_CHANNEL_LISTENERS`
/// is set). A minimal `DomainSet::none()` must still seed, because the seed is
/// DomainSet-independent.
///
/// The seed sits just *before* the ungated `INFRA: Once` block, so it re-runs on
/// every `register_domain_subscribers` call (each `bootstrap_core_runtime`),
/// re-applying the freshly reloaded config on an in-process restart — a seed gated
/// by the process-global `Once` would only fire on the first boot. `TEST_ENV_LOCK`
/// (via `EnvVarGuard`) serializes with `OPENHUMAN_TOOL_TIMEOUT_SECS` cleared so the
/// operator env override cannot mask the config-derived value. Runs under a tokio
/// runtime like the real boot paths — the INFRA block calls `subscribe_global`,
/// which `tokio::spawn`s when the global bus is already initialized by another test
/// in the binary.
#[tokio::test]
async fn tool_timeout_seeds_on_channelless_core_boot() {
    // Clear the operator override behind a panic-safe RAII guard: if any assertion
    // below panics, `Drop` still restores the previous value, so sibling tests that
    // share `TEST_ENV_LOCK` never inherit the cleared var.
    let _env = EnvVarGuard::locked_unset_many_async(&["OPENHUMAN_TOOL_TIMEOUT_SECS"]).await;

    // Distinctive, in-range (1..=3600) value so the assertion can only pass on a
    // real seed, never on the default. Channel-less: `channels_config` stays empty,
    // which is exactly the config for which `start_channels` is skipped.
    let mut config = crate::config::Config::default();
    config.agent.agent_timeout_secs = 1234;
    assert!(
        config.channels_config.active_channel.is_none(),
        "test premise: channel-less config, so start_channels would be skipped"
    );

    let tmp = tempfile::tempdir().expect("tempdir");
    // Minimal DomainSet — INFRA (and thus the timeout seed) is DomainSet-independent,
    // so even `none()` must seed. `embedded_core = true` skips the standalone
    // process-exit shutdown subscriber.
    super::register_domain_subscribers(
        tmp.path().to_path_buf(),
        config,
        true,
        crate::core::runtime::DomainSet::none(),
        &crate::core::runtime::DispatchCtx::ambient(),
        true,
    );

    assert_eq!(
        crate::tools::timeout::tool_execution_timeout_secs(),
        1234,
        "channel-less core boot must seed the tool-execution timeout from [agent].agent_timeout_secs"
    );
}

#[test]
fn proactive_message_reaches_web_on_channelless_core_boot() {
    const CHILD: &str = "OPENHUMAN_PROACTIVE_SUBSCRIBER_TEST_CHILD";
    const TEST_NAME: &str =
        "core::runtime::subscribers::tests::proactive_message_reaches_web_on_channelless_core_boot";

    if std::env::var(CHILD).as_deref() == Ok("1") {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                crate::core::bus::init().await.expect("bus init");
                let mut web_events = crate::web_chat::subscribe_web_channel_events();
                let config = crate::config::Config::default();
                assert!(config.channels_config.active_channel.is_none());

                super::register_domain_subscribers(
                    std::env::temp_dir(),
                    config,
                    true,
                    crate::core::runtime::DomainSet::none(),
                    &crate::core::runtime::DispatchCtx::ambient(),
                );

                crate::core::bus::BUS.publish(
                    crate::core::events::DomainEvent::ProactiveMessageRequested {
                        source: "cron:test".into(),
                        message: "Your morning briefing is ready.".into(),
                        job_name: Some("morning-briefing".into()),
                        tenant: None,
                    },
                );

                let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    loop {
                        let event = web_events
                            .recv()
                            .await
                            .expect("web event stream should remain open");
                        if matches!(
                            event.wire_event(),
                            crate::web_chat::WebChannelEvent::ProactiveMessage {
                                full_response, ..
                            } if full_response.as_deref() == Some("Your morning briefing is ready.")
                        ) {
                            break event;
                        }
                    }
                })
                .await
                .expect("proactive message should reach the web event stream");
                assert_eq!(event.event_name(), "proactive_message");
            });
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .output()
        .expect("run isolated proactive subscriber boot test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "isolated proactive subscriber boot test failed:\n{stdout}\n{stderr}"
    );
}

#[cfg(feature = "modules")]
#[test]
fn proactive_message_is_scoped_to_originating_saas_profile() {
    const CHILD: &str = "OPENHUMAN_PROACTIVE_SUBSCRIBER_TEST_CHILD";
    const TEST_NAME: &str = "core::runtime::subscribers::tests::proactive_message_is_scoped_to_originating_saas_profile";

    if std::env::var(CHILD).as_deref() == Ok("1") {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                crate::core::runtime::mode::lock_mode(crate::core::runtime::Mode::Saas)
                    .expect("lock isolated test process in SaaS mode");
                crate::core::bus::init().await.expect("bus init");
                let mut web_events = crate::web_chat::subscribe_web_channel_events();
                let config = crate::config::Config::default();
                assert!(config.channels_config.active_channel.is_none());

                let root = tempfile::tempdir().expect("profile root");
                let id = crate::profiles::ProfileId::parse("proactive-test").unwrap();
                let profile_host = std::sync::Arc::new(crate::profiles::ProfileHost::new(
                    crate::core::runtime::SaasConfig::new(root.path()),
                    crate::core::runtime::CoreContext::for_test(
                        crate::core::runtime::DomainSet::full(),
                        None,
                    ),
                ));
                profile_host
                    .provision(&id)
                    .await
                    .expect("provision profile");
                let profile = profile_host.open(&id).await.expect("open profile");
                crate::profiles::host::install(profile_host);
                let ctx = crate::core::runtime::DispatchCtx::new(std::sync::Arc::clone(
                    profile.context(),
                ));
                let tenant = ctx.event_tenant();
                assert_eq!(
                    tenant.as_ref().and_then(|tenant| tenant.profile.as_deref()),
                    Some("proactive-test")
                );

                super::register_domain_subscribers(
                    std::env::temp_dir(),
                    config,
                    true,
                    crate::core::runtime::DomainSet::none(),
                    &ctx,
                );

                crate::core::bus::BUS.publish(
                    crate::core::events::DomainEvent::ProactiveMessageRequested {
                        source: "cron:test".into(),
                        message: "Your morning briefing is ready.".into(),
                        job_name: Some("morning-briefing".into()),
                        tenant,
                    },
                );

                let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    loop {
                        let event = web_events
                            .recv()
                            .await
                            .expect("web event stream should remain open");
                        if matches!(
                            event.wire_event(),
                            crate::web_chat::WebChannelEvent::ProactiveMessage {
                                full_response, ..
                            } if full_response.as_deref() == Some("Your morning briefing is ready.")
                        ) {
                            break event;
                        }
                    }
                })
                .await
                .expect("proactive message should reach the web event stream");
                assert_eq!(event.event_name(), "proactive_message");
                assert!(event.belongs_to_profile("proactive-test"));
                assert!(!event.belongs_to_profile("another-profile"));
            });
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .output()
        .expect("run isolated proactive subscriber boot test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "isolated proactive subscriber boot test failed:\n{stdout}\n{stderr}"
    );
}
