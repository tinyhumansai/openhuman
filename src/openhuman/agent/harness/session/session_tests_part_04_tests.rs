use super::*;

/// The superseded synthesised instances are released once the readers holding
/// them go away — the issue's "clean up the old one once its refcount drops".
#[test]
fn superseded_synthesized_instances_are_released_when_readers_drop() {
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;

    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));

    let conn =
        |slug: &str, desc: &str| crate::openhuman::agent::context::prompt::ConnectedIntegration {
            toolkit: slug.into(),
            description: desc.into(),
            tools: vec![],
            gated_tools: vec![],
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        };

    agent.set_connected_integrations(vec![conn("gmail", "Email")]);
    agent.refresh_delegation_tools();

    // A turn takes its snapshot of the synthesised set and keeps it.
    let in_flight = agent.synthesized_tools_arc();
    assert_eq!(Arc::strong_count(&in_flight), 2, "agent + in-flight turn");

    // The connection set changes underneath it.
    agent.set_connected_integrations(vec![conn("gmail", "Email"), conn("notion", "Docs")]);
    agent.refresh_delegation_tools();

    // The reader still sees a coherent set, and it is the *previous* one.
    assert_eq!(
        Arc::strong_count(&in_flight),
        1,
        "the agent has moved on to a fresh Arc; only the in-flight turn holds the old one"
    );
    assert!(
        !Arc::ptr_eq(&in_flight, &agent.synthesized_tools_arc()),
        "a refresh must publish a new Arc rather than mutating the shared one"
    );

    // When the turn ends, the superseded instances are freed.
    drop(in_flight);
    assert_eq!(
        Arc::strong_count(&agent.synthesized_tools_arc()),
        2,
        "only the agent's own Arc and this test's clone remain"
    );
}

/// The tool block is part of the KV-cache prefix, so a refresh that changes
/// nothing must change no bytes.
///
/// Every prefix cache renders the tool catalogue ahead of the conversation, so
/// the `tools` array sits *in front of* the frozen system prompt rather than
/// after it. A rebuild that reorders an unchanged set — or that rebuilds at all
/// when the connection set is the same — therefore costs the entire cached
/// prefix on that turn, system prompt included, while every functional test
/// still passes because the same capabilities are advertised.
///
/// Two properties keep that from happening and both are easy to lose:
/// `refresh_delegation_tools` appends the synthesised specs in a deterministic
/// sequence (derived from `definition.subagents`, a `Vec`) rather than from the
/// `HashSet` it also maintains, and `connected_set_hash` sorts before hashing so
/// a backend that returns the same toolkits in a new order never reaches here at
/// all. This asserts the observable consequence rather than either mechanism, so
/// it survives a refactor of either.
#[test]
fn an_unchanged_integration_set_leaves_the_tool_block_byte_stable() {
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;

    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));

    let integrations = || {
        vec![
            crate::openhuman::agent::context::prompt::ConnectedIntegration {
                toolkit: "gmail".into(),
                description: "Email".into(),
                tools: vec![],
                gated_tools: vec![],
                connected: true,
                connections: Vec::new(),
                non_active_status: None,
            },
            crate::openhuman::agent::context::prompt::ConnectedIntegration {
                toolkit: "notion".into(),
                description: "Docs".into(),
                tools: vec![],
                gated_tools: vec![],
                connected: true,
                connections: Vec::new(),
                non_active_status: None,
            },
        ]
    };

    // Exactly what a provider request carries: name, description and schema,
    // in order. Description is part of the wire bytes too — a stale delegate
    // description surviving a reconcile is the same class of bug as a stale
    // schema and this fixture should catch either.
    let wire_block = |agent: &Agent| -> Vec<(String, String, String)> {
        agent
            .tool_specs()
            .iter()
            .map(|spec| {
                (
                    spec.name.clone(),
                    spec.description.clone(),
                    spec.parameters.to_string(),
                )
            })
            .collect()
    };

    agent.set_connected_integrations(integrations());
    agent.refresh_delegation_tools();
    let first = wire_block(&agent);
    assert!(
        first
            .iter()
            .any(|(name, _, _)| name.starts_with("delegate_")),
        "the fixture must actually synthesise delegates, or this pins nothing"
    );

    // A second reconcile over the same connection set — the turn-boundary path
    // when the Composio cache answers with what it answered last turn.
    agent.refresh_delegation_tools();
    assert_eq!(
        wire_block(&agent),
        first,
        "re-synthesising an unchanged integration set rewrote the tool block; \
         the tool catalogue precedes the conversation in every provider's cached \
         prefix, so this costs the frozen system prompt too"
    );

    // The same set in the opposite order is the same capability surface. The
    // hash is order-insensitive, so a reordered backend response must not even
    // reach a rebuild — and if it does, it must still land on the same bytes.
    let mut reordered = integrations();
    reordered.reverse();
    assert_eq!(
        crate::openhuman::integrations::composio::connected_set_hash(&reordered),
        crate::openhuman::integrations::composio::connected_set_hash(&integrations()),
        "connected_set_hash must not see a reordering as a change"
    );
    agent.set_connected_integrations(reordered);
    agent.refresh_delegation_tools();
    assert_eq!(
        wire_block(&agent),
        first,
        "the same toolkits in a different order produced a different tool block"
    );
}

/// A resumed prefix with no leading system message must not erase this
/// turn's freshly rendered one.
///
/// `bound_cached_transcript_messages` keeps a leading system message only
/// "when present" — a transcript that was never seeded with one (or was
/// truncated ahead of it) hands `absorb_resumed_transcript_prefix` a `cached`
/// prefix with no system entry at all. Unconditionally dropping the current
/// system message(s) in that case would leave `history` with none, which is
/// strictly worse than the stale-but-present one this turn already built.
#[tokio::test]
async fn a_resumed_prefix_without_a_system_message_keeps_the_current_one() {
    use crate::openhuman::agent::messages::{ChatMessage, ConversationMessage};

    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let canned = crate::openhuman::agent::harness::session::transcript::SessionTranscript {
        meta: fake_transcript_meta("thr_resume_no_system"),
        messages: vec![
            ChatMessage::user("first question"),
            ChatMessage::assistant("first answer"),
        ],
    };
    let (mut agent, _handle) = agent_with_fake_locator(workspace.path(), Some(canned));
    agent.try_load_session_transcript();

    agent.history = vec![
        ConversationMessage::Chat(ChatMessage::system("freshly rendered prompt")),
        ConversationMessage::Chat(ChatMessage::user("second question")),
    ];

    agent.absorb_resumed_transcript_prefix();

    let rendered: Vec<(&str, &str)> = agent
        .history
        .iter()
        .map(|entry| match entry {
            ConversationMessage::Chat(chat) => (chat.role.as_str(), chat.content.as_str()),
            other => panic!("expected only Chat entries, got {other:?}"),
        })
        .collect();
    assert_eq!(
        rendered,
        vec![
            ("user", "first question"),
            ("assistant", "first answer"),
            // The cached prefix supplied no system message, so this turn's
            // freshly rendered one must survive rather than being dropped
            // unconditionally.
            ("system", "freshly rendered prompt"),
            ("user", "second question"),
        ],
        "the current system prompt must be kept when the cached prefix has none"
    );
}
