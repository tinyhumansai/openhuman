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

#[test]
fn a_memory_binding_recalls_each_turn_only_when_asked() {
    let binding = MemoryBinding::new("writer").root("team:acme");
    assert_eq!(binding.recall_each_turn_timeout(), None);
    let binding = binding.recall_each_turn(std::time::Duration::from_millis(1_500));
    assert_eq!(
        binding.recall_each_turn_timeout(),
        Some(std::time::Duration::from_millis(1_500))
    );
    assert_eq!(binding.root_namespace(), Some("team:acme"));
}

#[test]
fn a_memory_binding_bounds_the_memory_block_only_when_asked() {
    let binding = MemoryBinding::new("writer");
    assert_eq!(binding.memory_block_deadline_timeout(), None);
    let binding = binding.memory_block_deadline(std::time::Duration::from_millis(300));
    assert_eq!(
        binding.memory_block_deadline_timeout(),
        Some(std::time::Duration::from_millis(300))
    );
}

#[test]
fn an_agent_spec_carries_tool_discovery_options_only_when_set() {
    assert_eq!(AgentSpec::new("alpha").into_parts().tool_discovery, None);
    let wanted = crate::ToolDiscoveryOptions {
        promote_found_tools: false,
        collapse_search_results: true,
        ..crate::ToolDiscoveryOptions::TODAY
    };
    let parts = AgentSpec::new("alpha").tool_discovery(wanted).into_parts();
    assert_eq!(parts.tool_discovery, Some(wanted));
}

#[test]
fn an_agent_spec_carries_context_overrides_only_when_set() {
    assert_eq!(
        AgentSpec::new("alpha").into_parts().context,
        crate::ContextOverrides::default()
    );
    let parts = AgentSpec::new("alpha")
        .keep_recent_tokens(12_000)
        .tool_result_max_age_turns(6)
        .tool_result_exempt_tools(["tool_search"])
        .replay_reasoning(crate::ReasoningReplay::CurrentTurn)
        .prune_retired_tools(true)
        .into_parts();
    assert_eq!(
        parts.context,
        crate::ContextOverrides {
            keep_recent_tokens: Some(12_000),
            tool_result_max_age_turns: Some(6),
            tool_result_exempt_tools: Some(vec!["tool_search".into()]),
            replay_reasoning: Some(crate::ReasoningReplay::CurrentTurn),
            prune_retired_tools: Some(true),
        }
    );
}
