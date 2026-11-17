use super::*;

fn off(name: &str) -> ComponentSet {
    let mut c = ComponentsConfig::default();
    assert!(c.set(name, false));
    ComponentSet::from_config(&c)
}

#[test]
fn all_on_changes_nothing() {
    let all = ComponentSet::all();
    assert_eq!(all.narrow_domains(DomainSet::full()), DomainSet::full());
    assert_eq!(
        all.narrow_services(ServiceSet::desktop()),
        ServiceSet::desktop()
    );
    assert_eq!(
        all.narrow_tool_groups(ToolGroups::advertised()),
        ToolGroups::advertised()
    );
}

#[test]
fn each_flag_removes_its_domain_members() {
    let full = DomainSet::full();
    assert!(!off("flows").narrow_domains(full).flows);
    assert!(!off("skills").narrow_domains(full).skills);
    assert!(!off("mcp").narrow_domains(full).mcp);
    assert!(!off("channels").narrow_domains(full).channels);
    assert!(!off("wallet").narrow_domains(full).web3);
    assert!(!off("voice").narrow_domains(full).voice);
    assert!(!off("memory").narrow_domains(full).memory);
    assert!(!off("connectors").narrow_domains(full).integrations);
    // Disabling one family leaves the others alone.
    let d = off("flows").narrow_domains(full);
    assert!(d.skills && d.mcp && d.agent);
}

#[test]
fn each_flag_removes_its_service_members() {
    let d = ServiceSet::desktop();
    assert!(!off("channels").narrow_services(d).channels);
    let m = off("memory").narrow_services(d);
    assert!(!m.memory_queue && !m.memory_sync);
    assert!(!off("skills").narrow_services(d).skill_catalog_refresh);
    assert!(!off("mcp").narrow_services(d).mcp_boot);
    assert!(!off("connectors").narrow_services(d).integrations);
    assert!(off("channels").narrow_services(d).cron);
}

#[test]
fn each_flag_turns_its_tool_groups_off() {
    let adv = ToolGroups::advertised();
    for (name, group) in [
        ("flows", "workflows"),
        ("documents", "documents"),
        ("mcp", "mcp"),
        ("connectors", "composio"),
        ("wallet", "web3"),
        ("skills", "skills"),
    ] {
        let g = off(name).narrow_tool_groups(adv.clone());
        assert_eq!(g.mode(group), GroupMode::Off, "{name} -> {group}");
    }
    let g = off("flows").narrow_tool_groups(adv);
    assert_eq!(g.mode("documents"), GroupMode::Advertised);
}

#[test]
fn config_cannot_widen_a_host_preset() {
    let all = ComponentSet::all();
    for preset in [
        DomainSet::harness(),
        DomainSet::kernel(),
        DomainSet::none(),
        DomainSet::embedded(),
    ] {
        assert_eq!(
            all.narrow_domains(preset),
            preset,
            "all-on must be identity"
        );
        assert_eq!(all.narrow_domains(preset).intersect(&preset), preset);
    }
    // kernel has no flows/mcp/...; components being on must not add them.
    let k = all.narrow_domains(DomainSet::kernel());
    assert!(!k.flows && !k.mcp && !k.memory && !k.skills);
    // A tool-group host preset of `none` stays Off.
    let g = all.narrow_tool_groups(ToolGroups::none());
    assert_eq!(g.mode("workflows"), GroupMode::Off);
    // And headless services stay off.
    assert_eq!(all.narrow_services(ServiceSet::none()), ServiceSet::none());
    assert_eq!(
        all.narrow_services(ServiceSet::headless_api()),
        ServiceSet::headless_api()
    );
}

#[test]
fn narrowing_never_raises_a_withheld_group() {
    let g = off("flows").narrow_tool_groups(ToolGroups::packed());
    assert_eq!(g.mode("workflows"), GroupMode::Off);
    assert_eq!(g.mode("documents"), GroupMode::Withheld);
}

#[test]
fn disabled_component_answers_a_typed_error() {
    let err = off("hosting").require("hosting").unwrap_err();
    assert_eq!(err.message, "COMPONENT_DISABLED:hosting");
    assert!(err.expected_user_state);
    assert_eq!(err.data.as_ref().unwrap()["component"], "hosting");
    let decoded = StructuredRpcError::decode(&err.encode()).unwrap();
    assert_eq!(decoded, err);
    assert!(ComponentSet::all().require("hosting").is_ok());
    assert!(ComponentSet::all().enabled("not-a-component"));
}

#[test]
fn group_to_component_mapping_names_real_components() {
    for g in DomainGroup::ALL {
        if let Some(n) = ComponentSet::component_for_group(*g) {
            assert!(COMPONENT_NAMES.contains(&n), "{n}");
        }
    }
}

#[test]
fn channels_on_never_widens_a_host_that_left_channels_out() {
    let mut host = DomainSet::full();
    host.channels = false;
    let mut services = ServiceSet::desktop();
    services.channels = false;
    let on = ComponentSet::all();
    assert!(!on.narrow_domains(host).channels);
    assert!(!on.narrow_services(services).channels);
}

#[test]
fn channels_off_names_the_typed_error_and_owns_the_channels_group() {
    let c = off("channels");
    let err = c.require("channels").unwrap_err();
    assert_eq!(err.message, "COMPONENT_DISABLED:channels");
    assert_eq!(
        ComponentSet::component_for_group(DomainGroup::Channels),
        Some("channels")
    );
}

#[test]
fn require_config_refuses_with_the_typed_error_only_when_the_flag_is_off() {
    let mut config = crate::config::Config::default();
    assert!(require_config(&config, "computer").is_ok());
    config.components.computer = false;
    let err = require_config(&config, "computer").unwrap_err();
    assert!(err.contains("COMPONENT_DISABLED:computer"), "{err}");
    assert!(require_config(&config, "search").is_ok());
}

#[test]
fn the_tinyflows_flag_stops_the_cron_service_and_nothing_else() {
    let desktop = ServiceSet::desktop();
    assert!(desktop.cron, "the desktop preset runs the scheduler");
    let narrowed = off("tinyflows").narrow_services(desktop);
    assert!(!narrowed.cron);
    assert_eq!(narrowed.channels, desktop.channels);
    // `flows` is a different switch: it leaves the scheduler running.
    assert!(off("flows").narrow_services(desktop).cron);
    assert_eq!(
        off("tinyflows").require("tinyflows").unwrap_err().message,
        "COMPONENT_DISABLED:tinyflows"
    );
}
