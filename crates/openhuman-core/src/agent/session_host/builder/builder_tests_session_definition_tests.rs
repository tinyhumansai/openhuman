//! `agent_definition`: a direct builder caller can be its own catalogue.
//!
//! Every session turn is a hosted root invocation that resolves its agent id
//! against the host catalogue before it composes a message, and refuses the
//! turn when the id is not there. `from_config_with_definition` already
//! stamps a caller's definition on the session for that reason; a direct
//! builder caller had no equivalent, and fell back to the process registry.
//! These tests pin the route that closes that gap.

use crate::agent::harness::definition::{AgentDefinition, AgentDefinitionRegistry, ToolScope};
use crate::agent::tinyagents::host::OpenHumanDefinitionRegistry;
use std::sync::Arc;
use tinyagents_definition::DefinitionRegistry;

/// An id no built-in definition uses, so a hit can only have come from the
/// caller's own definition.
const SEAT: &str = "library-host--seat";

fn definition(id: &str, tools: Vec<&str>) -> Arc<AgentDefinition> {
    let mut def = AgentDefinitionRegistry::builtins_only()
        .get("orchestrator")
        .cloned()
        .expect("built-in orchestrator");
    def.id = id.to_string();
    def.display_name = Some("Library host seat".to_string());
    def.when_to_use = "a seat a library host built and owns".to_string();
    def.tools = ToolScope::Named(tools.into_iter().map(str::to_string).collect());
    def.disallowed_tools.clear();
    Arc::new(def)
}

/// A builder with everything a session needs and no identity yet.
fn bare() -> crate::agent::SessionHostBuilder {
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    crate::agent::SessionHostBuilder::new()
        .chat_model(model)
        .tools(Vec::new())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
}

fn seat(
    definition: Option<Arc<AgentDefinition>>,
) -> crate::agent::session_host::types::OpenHumanSessionHost {
    let mut builder = bare().agent_definition_name(SEAT);
    if let Some(definition) = definition {
        builder = builder.agent_definition(definition);
    }
    builder
        .build()
        .expect("a library host seat is a legal build")
}

/// The catalogue a turn is resolved against, assembled the way
/// `OpenHumanHostBundleFactory::build` assembles it.
fn catalogue(
    agent: &crate::agent::session_host::types::OpenHumanSessionHost,
) -> OpenHumanDefinitionRegistry {
    let base = agent
        .hosted_base
        .as_ref()
        .expect("a caller that brought its own definition has a hosted authority");
    let mut catalogue = OpenHumanDefinitionRegistry::new(Arc::clone(&base.definitions))
        .with_config(Arc::clone(&base.config));
    if let Some(definition) = base.session_definition.as_ref() {
        catalogue = catalogue.with_session_definition(Arc::clone(definition));
    }
    catalogue
}

#[tokio::test]
async fn a_definition_the_builder_was_given_resolves_for_the_turn() {
    let agent = seat(Some(definition(SEAT, vec!["desk_say", "publish_artifact"])));

    let resolved = catalogue(&agent)
        .resolve(SEAT)
        .await
        .expect("resolution never errors")
        .expect("the caller's own definition answers the lookup");

    assert_eq!(resolved.id, SEAT);
    assert_eq!(resolved.name, "Library host seat");
    assert!(
        resolved.is_valid(),
        "a hosted turn refuses an invalid definition: {:?}",
        resolved.diagnostics()
    );
}

#[tokio::test]
async fn the_definition_carries_the_belt_the_turn_is_allowed_to_call() {
    // The resolved definition's tool list *is* the turn's allow-list, so a
    // seat whose belt it does not name cannot call it. A definition that
    // names nothing denies everything rather than allowing everything.
    let agent = seat(Some(definition(SEAT, vec!["desk_say", "publish_artifact"])));

    let resolved = catalogue(&agent)
        .resolve(SEAT)
        .await
        .expect("resolution never errors")
        .expect("resolves");

    assert_eq!(resolved.tools, vec!["desk_say", "publish_artifact"]);
}

#[tokio::test]
async fn the_sessions_own_definition_outranks_a_registry_entry_sharing_its_id() {
    // A library host may reuse an id the process registry also knows. Its own
    // definition must win, or the host silently runs someone else's agent.
    let _ = AgentDefinitionRegistry::init_global_builtins();
    // Named by the definition, which is the only spelling the build accepts:
    // a session stamped with some other id could never resolve to it.
    let agent = bare()
        .agent_definition(definition("orchestrator", vec!["desk_say"]))
        .build()
        .expect("a seat may reuse a built-in id");

    let resolved = catalogue(&agent)
        .resolve("orchestrator")
        .await
        .expect("resolution never errors")
        .expect("resolves");

    assert_eq!(resolved.name, "Library host seat");
    assert_eq!(resolved.tools, vec!["desk_say"]);
}

#[tokio::test]
async fn without_a_definition_the_same_id_is_still_unknown() {
    // The counterpart, and the reason this route had to exist: nothing else
    // the builder takes can put an id in the catalogue.
    let agent = seat(None);
    // Not an early return on absence: a builder that stopped creating a
    // hosted authority at all would satisfy that, which is the regression
    // this test is here to catch. Under `cfg(test)` the builder always has a
    // catalogue to fall back on, so requiring one is safe.
    let base = agent
        .hosted_base
        .as_ref()
        .expect("a session always has a hosted authority under cfg(test)");
    assert!(base.session_definition.is_none());

    let catalogue = OpenHumanDefinitionRegistry::new(Arc::clone(&base.definitions))
        .with_config(Arc::clone(&base.config));

    assert!(
        catalogue
            .resolve(SEAT)
            .await
            .expect("resolution never errors")
            .is_none(),
        "the id must not resolve without the caller's definition"
    );
}

#[tokio::test]
async fn the_session_is_stamped_with_the_definitions_own_id() {
    // A caller that supplies a definition should not also have to name it:
    // the two must agree or the definition is unreachable, so the name is
    // derived rather than left to be got right twice.
    let agent = bare()
        .agent_definition(definition(SEAT, vec!["desk_say"]))
        .build()
        .expect("a definition alone is enough identity");

    assert_eq!(agent.agent_definition_name(), SEAT);
    assert!(
        catalogue(&agent)
            .resolve(SEAT)
            .await
            .expect("resolution never errors")
            .is_some(),
        "the derived id must be the one the definition answers for"
    );
}

#[test]
fn a_name_the_definition_contradicts_fails_the_build() {
    // Silently preferring either one would leave a session that resolves to
    // nothing, and every turn refused for want of a definition.
    // `OpenHumanSessionHost` is not `Debug`, so the `Ok` arm cannot be
    // unwrapped into a panic message.
    let Err(error) = bare()
        .agent_definition_name("some-other-id")
        .agent_definition(definition(SEAT, vec!["desk_say"]))
        .build()
    else {
        panic!("a name the definition contradicts must fail the build");
    };

    let message = error.to_string();
    assert!(message.contains("some-other-id"), "{message}");
    assert!(message.contains(SEAT), "{message}");
}

#[tokio::test]
async fn a_padded_name_is_stored_as_the_definitions_own_id() {
    // ` foo ` and `foo` agree after trimming, so the consistency check admits
    // the pair. Storing the caller's spelling would then resolve through
    // `OpenHumanDefinitionRegistry` (which trims) and miss through
    // `AgentDefinitionRegistry::get` (which does not), so the id is
    // normalized to the definition's own.
    let agent = bare()
        .agent_definition_name(format!("  {SEAT}  "))
        .agent_definition(definition(SEAT, vec!["desk_say"]))
        .build()
        .expect("a name that differs only by padding is not a contradiction");

    assert_eq!(agent.agent_definition_name(), SEAT);
    assert!(catalogue(&agent)
        .resolve(agent.agent_definition_name())
        .await
        .expect("resolution never errors")
        .is_some());
}
