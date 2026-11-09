use super::*;
use crate::config::schema::components::ComponentsConfig;

fn off(name: &str) -> ComponentSet {
    let mut c = ComponentsConfig::default();
    assert!(c.set(name, false));
    ComponentSet::from_config(&c)
}

#[test]
fn capability_of_a_disabled_component_shows_disabled() {
    let cap = super::super::lookup("voice.live_agent").expect("live voice capability");
    assert_ne!(cap.status, CapabilityStatus::Disabled);
    let marked = apply_components(cap, &off("voice"));
    assert_eq!(marked.status, CapabilityStatus::Disabled);
    assert_eq!(marked.id, cap.id);
    assert_eq!(apply_components(cap, &off("wallet")).status, cap.status);
    assert_eq!(
        apply_components(cap, &ComponentSet::all()).status,
        cap.status
    );
}

#[test]
fn domain_mapping_names_real_components() {
    for cap in super::super::all_capabilities() {
        if let Some(name) = component_for_domain(cap.domain) {
            assert!(
                crate::core::runtime::COMPONENT_NAMES.contains(&name),
                "{name}"
            );
        }
    }
}

#[test]
fn every_mapped_domain_shows_disabled_when_its_component_is_off() {
    for cap in super::super::all_capabilities() {
        if let Some(name) = component_for_domain(cap.domain) {
            let marked = apply_components(*cap, &off(name));
            assert_eq!(marked.status, CapabilityStatus::Disabled, "{}", cap.id);
        }
    }
}

#[test]
fn sandbox_capability_shows_disabled_when_sandbox_component_is_off() {
    let cap = super::super::lookup("security.sandbox_backends").expect("sandbox capability");
    assert_ne!(cap.status, CapabilityStatus::Disabled);
    assert_eq!(
        apply_components(cap, &off("sandbox")).status,
        CapabilityStatus::Disabled
    );
    assert_eq!(apply_components(cap, &off("voice")).status, cap.status);
}

#[test]
fn inbound_queue_capability_shows_disabled_when_the_eventqueues_component_is_off() {
    let cap = super::super::lookup("automation.inbound_event_queues").expect("inbound queues");
    assert_ne!(cap.status, CapabilityStatus::Disabled);
    assert_eq!(
        apply_components(cap, &off("eventqueues")).status,
        CapabilityStatus::Disabled
    );
    let tasks = super::super::lookup("automation.task_sources").expect("task sources");
    assert_eq!(
        apply_components(tasks, &off("eventqueues")).status,
        tasks.status
    );
}

#[test]
fn scheduled_job_capabilities_show_disabled_when_the_tinyflows_component_is_off() {
    for id in [
        "automation.view_cron_jobs",
        "automation.set_job_intervals",
        "automation.view_execution_history",
        "automation.morning_briefing",
    ] {
        let cap = super::super::lookup(id).expect(id);
        assert_ne!(cap.status, CapabilityStatus::Disabled, "{id}");
        assert_eq!(
            apply_components(cap, &off("tinyflows")).status,
            CapabilityStatus::Disabled,
            "{id}"
        );
        assert_eq!(
            apply_components(cap, &off("flows")).status,
            cap.status,
            "{id}"
        );
    }
}

#[tokio::test]
async fn an_explicit_context_decides_which_capabilities_are_disabled() {
    use crate::core::runtime::context::CoreContext;
    use crate::core::runtime::DispatchCtx;

    let mut voice_off = ComponentSet::all();
    voice_off.voice = false;
    let off = DispatchCtx::new(CoreContext::for_test_with_components(voice_off));
    let on = DispatchCtx::new(CoreContext::for_test_with_components(ComponentSet::all()));

    let ask = |ctx: DispatchCtx| async move {
        tokio::task::yield_now().await;
        lookup_capability(&ctx, "voice.live_agent")
            .expect("live voice capability")
            .value
            .status
    };
    let (disabled, enabled) = tokio::join!(ask(off), ask(on));
    assert_eq!(disabled, CapabilityStatus::Disabled);
    assert_ne!(enabled, CapabilityStatus::Disabled);
}
