use super::*;
use crate::agent::build::{check_domains_narrow, check_tool_groups_narrow};
use crate::AgentError;
use openhuman_core::tools::toolpacks::GroupMode;

#[test]
fn model_without_a_route_pins_the_model_on_an_inherited_provider() {
    let spec = AgentSpec::new("alpha").model("gpt-5");
    let parts = spec.into_parts();
    let provider = parts.provider.expect("provider set");
    assert_eq!(provider.model_id(), Some("gpt-5"));
    assert!(!provider.is_routed());
}

#[test]
fn debug_omits_the_provider_and_closure() {
    let spec = AgentSpec::new("alpha")
        .provider(Provider::openai_compatible(
            "https://x.example",
            "sk-secret",
        ))
        .config(|_| {});
    let debug = format!("{spec:?}");
    assert!(debug.contains("alpha"));
    assert!(!debug.contains("sk-secret"));
}

#[test]
fn domains_may_only_narrow_the_runtime() {
    let runtime = DomainSet::embedded();
    assert!(check_domains_narrow(DomainSet::kernel(), runtime).is_ok());
    let err = check_domains_narrow(DomainSet::full(), runtime).expect_err("widens");
    assert!(matches!(err, AgentError::WidensRuntime(_)), "{err:?}");
}

#[test]
fn tool_groups_may_only_narrow_the_runtime() {
    let runtime = ToolGroups::default(); // every group withheld
    assert!(check_tool_groups_narrow(&ToolGroups::none(), &runtime).is_ok());
    assert!(check_tool_groups_narrow(&ToolGroups::default(), &runtime).is_ok());
    let Some(id) = ToolGroups::ids().next() else {
        return; // slim build with no packs: nothing to widen
    };
    let widened = ToolGroups::none().with(id, GroupMode::Advertised);
    let err = check_tool_groups_narrow(&widened, &runtime).expect_err("widens");
    assert!(matches!(err, AgentError::WidensRuntime(_)), "{err:?}");
}

#[test]
fn composio_credential_is_carried_to_the_build_step_and_kept_out_of_debug() {
    use openhuman_core::config::ComposioHostCredential;
    let spec = AgentSpec::new("alpha")
        .composio(ComposioHostCredential::direct("ck_secret_alpha").entity_id("tenant-a"));
    let debug = format!("{spec:?}");
    assert!(!debug.contains("ck_secret_alpha"), "{debug}");
    let parts = spec.into_parts();
    let credential = parts.composio.expect("composio credential");
    assert_eq!(credential.api_key(), "ck_secret_alpha");
    assert_eq!(credential.entity(), "tenant-a");
}
