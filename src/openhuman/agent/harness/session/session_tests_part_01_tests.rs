use super::*;

/// Regression test for the `build_session_agent_inner` agent-id
/// threading bug.
///
/// Prior to the fix, `build_session_agent_inner` took an `agent_id:
/// &str` parameter but never threaded it into the `Agent::builder()`
/// chain. The builder's `.build()` then fell back to the legacy
/// `"main"` default, and every session built via
/// `Agent::from_config_for_agent` carried `agent_definition_name =
/// "main"` at runtime regardless of which id the caller asked for.
///
/// In the current codebase the user-facing path is `"orchestrator"`,
/// and the same builder is also used by several direct session agents.
/// A fallback to `"main"` silently misfiles transcripts on disk and
/// stamps the wrong agent metadata into them. Typed sub-agents are
/// unaffected because they're spawned through `subagent_runner` and
/// never touch the `from_config_for_agent` / builder fallback path.
///
/// This test pins the builder contract the fix relies on: calling
/// `.agent_definition_name(id)` on the builder chain produces an
/// `Agent` whose [`Agent::agent_definition_name`] accessor returns
/// that id verbatim. `"orchestrator"` covers the user-facing chat path;
/// the others are defensive coverage so a future top-level caller still
/// inherits the contract.
#[test]
fn agent_builder_threads_agent_definition_name_when_set() {
    for expected in ["integrations_agent", "orchestrator", "trigger_triage"] {
        let agent = build_minimal_agent_with_definition_name(Some(expected));
        assert_eq!(
            agent.agent_definition_name(),
            expected,
            "agent.agent_definition_name() should return the value passed to the builder"
        );
    }
}

/// Complementary to [`agent_builder_threads_agent_definition_name_when_set`]:
/// when a caller builds an `Agent` without ever calling
/// [`AgentBuilder::agent_definition_name`], the legacy `"main"`
/// fallback still applies. This pins the fallback contract that
/// direct builder users (tests, CLI harnesses) rely on, and
/// documents the exact misbehaviour the threading fix prevents —
/// `build_session_agent_inner` used to hit this fallback even when
/// a caller asked for a concrete agent id, because the
/// `.agent_definition_name` setter was missing from the builder chain.
#[test]
fn agent_builder_falls_back_to_main_when_definition_name_unset() {
    let agent = build_minimal_agent_with_definition_name(None);
    assert_eq!(
        agent.agent_definition_name(),
        "main",
        "AgentBuilder::build should default agent_definition_name to \"main\" when unset"
    );
}

#[test]
fn set_connected_integrations_marks_session_initialized_and_updates_hash() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    assert!(
        !agent.connected_integrations_initialized,
        "fresh builder-built agents should start with placeholder integration state"
    );

    agent.set_connected_integrations(vec![
        crate::openhuman::agent::context::prompt::ConnectedIntegration {
            toolkit: "gmail".into(),
            description: "Email".into(),
            tools: vec![],
            gated_tools: vec![],
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        },
    ]);

    assert!(agent.connected_integrations_initialized);
    assert_eq!(agent.connected_integrations().len(), 1);
    assert_eq!(agent.connected_integrations()[0].toolkit, "gmail");
    assert_eq!(
        agent.last_seen_integrations_hash,
        crate::openhuman::integrations::composio::connected_set_hash(
            agent.connected_integrations()
        )
    );
}

#[test]
fn refresh_delegation_tools_updates_schema_even_when_tool_arc_is_shared() {
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;

    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.set_connected_integrations(vec![
        crate::openhuman::agent::context::prompt::ConnectedIntegration {
            toolkit: "gmail".into(),
            description: "Email".into(),
            tools: vec![],
            gated_tools: vec![],
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        },
    ]);

    agent.refresh_delegation_tools();
    assert_eq!(
        integration_delegate_toolkit_enum(&agent),
        vec!["gmail".to_string()]
    );

    // Simulate an in-flight turn holding a shared Arc clone.
    let _shared_tools = agent.tools_arc();
    agent.set_connected_integrations(vec![
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
    ]);

    agent.refresh_delegation_tools();
    assert_eq!(
        integration_delegate_toolkit_enum(&agent),
        vec!["gmail".to_string(), "notion".to_string()]
    );
    // The schema moved; the executable instances must have moved with it.
    // Before #6145 the shared clone above blocked the instance reconcile and
    // only this assertion would have failed.
    super::assert_synthesized_delegates_are_executable(&agent);
}

/// Regression for #3044: repeated mid-session connects while the `tools`
/// Arc stays shared (a detached sub-agent's cloned `ParentExecutionContext`
/// outliving its turn) must not accumulate duplicate synthesised `ToolSpec`s.
///
/// Before the fix, a failed `tools` reconcile rolled `synthesized_tool_names`
/// back to the *old* mask. On the next refresh the spec `retain` used that
/// stale mask and failed to drop the intervening refresh's specs, so the
/// synthesised delegate spec piled up once per connect.
#[test]
fn refresh_delegation_tools_no_duplicate_specs_across_shared_arc_connects() {
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

    let delegate_spec_count = |agent: &Agent| -> usize {
        agent
            .tool_specs()
            .iter()
            .filter(|s| s.name == "delegate_to_integrations_agent")
            .count()
    };

    // Turn 1: gmail connects.
    agent.set_connected_integrations(vec![conn("gmail", "Email")]);
    agent.refresh_delegation_tools();

    // Hold a shared clone across every subsequent refresh so `Arc::get_mut`
    // always fails — exactly what happens during an in-flight turn.
    let _shared_tools = agent.tools_arc();

    // Turn 2: notion connects mid-session.
    agent.set_connected_integrations(vec![conn("gmail", "Email"), conn("notion", "Docs")]);
    agent.refresh_delegation_tools();

    // Turn 3: slack connects mid-session — this is where the old code
    // produced a duplicate `delegate_to_integrations_agent` spec.
    agent.set_connected_integrations(vec![
        conn("gmail", "Email"),
        conn("notion", "Docs"),
        conn("slack", "Chat"),
    ]);
    agent.refresh_delegation_tools();

    assert_eq!(
        delegate_spec_count(&agent),
        1,
        "exactly one synthesised delegate spec must remain after repeated shared-Arc connects"
    );
    assert_eq!(
        integration_delegate_toolkit_enum(&agent),
        vec![
            "gmail".to_string(),
            "notion".to_string(),
            "slack".to_string()
        ]
    );
    super::assert_synthesized_delegates_are_executable(&agent);
}

#[tokio::test]
async fn composio_listener_drains_integrations_changed_events() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    // Use an isolated bus, NOT the global singleton: other tests (e.g.
    // `events_tests` and any composio-listener publisher) emit
    // `ComposioIntegrationsChanged` on the global bus in parallel, which would
    // leak into this receiver and make the second drain observe a foreign
    // event — racing the "drained after one pass" assertion. Injecting a
    // locally-owned channel keeps this test deterministic.
    let isolated = crate::core::bus_testing::isolated_bus().await;
    agent.set_composio_integrations_rx_for_test(isolated.receiver());
    isolated.publish(DomainEvent::ComposioIntegrationsChanged {
        toolkits: vec!["gmail".into()],
    });
    assert!(agent.drain_composio_integrations_changed_events());
    assert!(
        !agent.drain_composio_integrations_changed_events(),
        "event queue should be drained after one pass"
    );
}

#[tokio::test]
async fn skill_listener_drains_workflows_changed_events() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    // Use an isolated bus, NOT the global singleton: other tests publish
    // `WorkflowsChanged` on the global bus in parallel — `skill_listener_
    // treats_lag_as_signal` floods 256 of them and
    // `create_workflow_inner_emits_workflows_changed` emits one — so a foreign
    // event could land between the two drains below and flip the second drain
    // to `true`, failing the "drained after one pass" assertion. Injecting a
    // locally-owned channel isolates this test from those publishers.
    let isolated = crate::core::bus_testing::isolated_bus().await;
    agent.set_skill_events_rx_for_test(isolated.receiver());
    isolated.publish(DomainEvent::WorkflowsChanged {
        reason: "install".into(),
    });
    assert!(
        agent.drain_skill_events(),
        "a WorkflowsChanged event should be observed"
    );
    assert!(
        !agent.drain_skill_events(),
        "event queue should be drained after one pass"
    );
}

#[tokio::test]
async fn skill_listener_treats_lag_as_signal() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    // Isolated bus (see `skill_listener_drains_workflows_changed_events` for
    // why the global singleton races). Flood well past the 64-slot bounded
    // channel so the receiver lags. The `Lagged` arm must still report a
    // signal (returns true) so a refresh isn't silently dropped under load.
    let isolated = crate::core::bus_testing::isolated_bus().await;
    agent.set_skill_events_rx_for_test(isolated.receiver());
    for _ in 0..256 {
        // Overruns the receiver's buffer on purpose: `try_recv` must then
        // report `Lagged`, which the drain treats as "something changed".
        isolated.publish(DomainEvent::WorkflowsChanged {
            reason: "install".into(),
        });
    }
    assert!(
        agent.drain_skill_events(),
        "a lagged listener must be treated as a signal"
    );
}

#[tokio::test]
async fn skill_listener_closed_channel_nulls_rx_and_is_not_a_signal() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    // A receiver whose sender has been dropped → `try_recv` yields `Closed`.
    let isolated = crate::core::bus_testing::isolated_bus().await;
    agent.set_skill_events_rx_for_test(isolated.receiver());
    // Dropping the bus drops its connection, which eventually closes the signal
    // stream. "Eventually" is the difference from a raw channel: the dispatch
    // task has to notice the transport is gone and exit before the broadcast
    // sender is released, so this polls rather than asserting immediately.
    drop(isolated);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while agent.has_skill_events_rx() {
        assert!(
            !agent.drain_skill_events(),
            "a closed channel is never a signal"
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "a closed receiver should be dropped so the next drain re-arms"
        );
        tokio::task::yield_now().await;
    }
}

/// Exercises real SKILL.md discovery from disk, so it is meaningful only with
/// the `skills` domain compiled in — the disabled facade's
/// `load_workflow_metadata` always returns an empty catalog by design.
#[test]
#[cfg(feature = "skills")]
fn refresh_workflows_picks_up_skill_installed_on_disk() {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    use crate::openhuman::skills::ops_types::{SKILL_MD, TRUST_MARKER};

    // Isolated, trusted workspace with one project-scope skill on disk.
    let ws = tempfile::TempDir::new().expect("temp workspace");
    let wsp = ws.path().to_path_buf();
    std::fs::create_dir_all(wsp.join(".openhuman")).unwrap();
    std::fs::write(wsp.join(".openhuman").join(TRUST_MARKER), "").unwrap();
    let skill_dir = wsp
        .join(".openhuman")
        .join("skills")
        .join("zz-refresh-test");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join(SKILL_MD),
        "---\nname: zz-refresh-test\ndescription: a refresh test skill\n---\n# body\n",
    )
    .unwrap();

    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();
    let provider = Arc::new(MockProvider {
        responses: Mutex::new(vec![]),
    });
    let mut agent = Agent::builder()
        .chat_model(provider)
        .tools(vec![Box::new(MockTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(wsp.clone())
        .build()
        .expect("agent build should succeed");

    // Starts with no skills; refresh discovers the on-disk one and parks it
    // for announcement.
    assert!(agent.test_workflow_ids().is_empty());
    assert!(
        agent.refresh_workflows("test"),
        "installing a skill on disk should change the set"
    );
    assert!(
        agent
            .test_workflow_ids()
            .iter()
            .any(|id| id == "zz-refresh-test"),
        "the new skill should be discoverable"
    );
    assert!(
        agent
            .test_pending_skill_announcement()
            .iter()
            .any(|id| id == "zz-refresh-test"),
        "the new skill should be parked for announcement"
    );
    // Idempotent: no new install -> no change.
    assert!(
        !agent.refresh_workflows("test"),
        "no install since last refresh -> no change"
    );
}

/// See [`refresh_workflows_picks_up_skill_installed_on_disk`] — same
/// disk-discovery dependency, so same `skills` gate.
#[test]
#[cfg(feature = "skills")]
fn refresh_workflows_retracts_skill_removed_from_disk() {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    use crate::openhuman::skills::ops_types::{SKILL_MD, TRUST_MARKER};

    let ws = tempfile::TempDir::new().expect("temp workspace");
    let wsp = ws.path().to_path_buf();
    std::fs::create_dir_all(wsp.join(".openhuman")).unwrap();
    std::fs::write(wsp.join(".openhuman").join(TRUST_MARKER), "").unwrap();

    // Write a skill to disk.
    let skill_dir = wsp
        .join(".openhuman")
        .join("skills")
        .join("zz-retract-test");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join(SKILL_MD),
        "---\nname: zz-retract-test\ndescription: a retraction test skill\n---\n# body\n",
    )
    .unwrap();

    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();
    let provider = Arc::new(MockProvider {
        responses: Mutex::new(vec![]),
    });
    let mut agent = Agent::builder()
        .chat_model(provider)
        .tools(vec![Box::new(MockTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(wsp.clone())
        .build()
        .expect("agent build should succeed");

    // First refresh: picks up the installed skill.
    assert!(agent.refresh_workflows("test-install"));
    assert!(
        agent
            .test_workflow_ids()
            .iter()
            .any(|id| id == "zz-retract-test"),
        "skill should be in catalogue after first refresh"
    );
    assert!(
        agent
            .test_pending_skill_announcement()
            .iter()
            .any(|id| id == "zz-retract-test"),
        "skill should be parked for announcement"
    );
    // Now remove the skill from disk.
    std::fs::remove_dir_all(&skill_dir).unwrap();

    // Second refresh: detects the removal, parks the retraction.
    assert!(
        agent.refresh_workflows("test-remove"),
        "removing a skill should change the set"
    );
    assert!(
        !agent
            .test_workflow_ids()
            .iter()
            .any(|id| id == "zz-retract-test"),
        "skill should be gone from catalogue after removal"
    );
    assert!(
        agent
            .test_pending_skill_retraction()
            .iter()
            .any(|id| id == "zz-retract-test"),
        "removed skill should be parked for retraction"
    );
    // Retraction should have cleared it from announced_skills; re-install will
    // be announced fresh (not silently re-added). Verify by re-adding the skill
    // and confirming it gets announced again.
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join(SKILL_MD),
        "---\nname: zz-retract-test\ndescription: a retraction test skill\n---\n# body\n",
    )
    .unwrap();
    assert!(agent.refresh_workflows("test-reinstall"));
    assert!(
        agent
            .test_pending_skill_announcement()
            .iter()
            .any(|id| id == "zz-retract-test"),
        "re-installed skill should be announced again after retraction cleared it from announced set"
    );
    // Re-install must also cancel the still-pending retraction so the user turn
    // never carries a contradictory "installed" + "retracted" pair for the same
    // skill.
    assert!(
        !agent
            .test_pending_skill_retraction()
            .iter()
            .any(|id| id == "zz-retract-test"),
        "re-install should cancel the pending retraction for the same skill"
    );
}

#[tokio::test]
async fn turn_without_tools_returns_text() {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Arc::new(MockProvider {
        responses: Mutex::new(vec![crate::openhuman::inference::provider::ChatResponse {
            text: Some("hello".into()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        }]),
    });

    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    let mut agent = Agent::builder()
        .chat_model(provider)
        .tools(vec![Box::new(MockTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .build()
        .unwrap();

    let response = agent.turn("hi").await.unwrap();
    assert_eq!(response, "hello");
}

/// The public [`Agent::last_turn_usage`] accessor peeks the per-turn
/// token/cost totals **without draining** them, so a downstream crate
/// embedding OpenHuman as a library (e.g. the OpenCompany hosting platform's
/// cost-metering hook) can read usage after a turn while the existing
/// web-channel `take_last_turn_usage_totals` drain path still works.
#[tokio::test]
async fn last_turn_usage_is_public_and_non_draining() {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Arc::new(MockProvider {
        responses: Mutex::new(vec![crate::openhuman::inference::provider::ChatResponse {
            text: Some("hello".into()),
            tool_calls: vec![],
            usage: Some(crate::openhuman::inference::provider::UsageInfo {
                input_tokens: 123,
                output_tokens: 45,
                context_window: 8000,
                charged_amount_usd: 0.01,
                ..Default::default()
            }),
            reasoning_content: None,
        }]),
    });

    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    let mut agent = Agent::builder()
        .chat_model(provider)
        .tools(vec![Box::new(MockTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .build()
        .unwrap();

    // No turn has run yet — nothing to report.
    assert!(agent.last_turn_usage().is_none());

    let response = agent.turn("hi").await.unwrap();
    assert_eq!(response, "hello");

    // The accessor now yields totals, and the return type's fields are all
    // publicly readable (this closure would not compile if they were not).
    let peeked: crate::openhuman::agent::harness::LastTurnUsage = {
        let usage = agent
            .last_turn_usage()
            .expect("usage should be populated after a turn");
        crate::openhuman::agent::harness::LastTurnUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            cost_usd: usage.cost_usd,
            context_window: usage.context_window,
            subagents: usage.subagents.clone(),
        }
    };

    // Peeking must not consume: a second read returns the same snapshot.
    assert_eq!(agent.last_turn_usage(), Some(&peeked));

    // The internal web-channel drain still sees the very same value, proving
    // the borrow accessor left it untouched.
    let drained = agent
        .take_last_turn_usage_totals()
        .expect("drain should still yield the totals the borrow peeked");
    assert_eq!(drained, peeked);

    // After the drain the peek accessor reports nothing, as expected.
    assert!(agent.last_turn_usage().is_none());
}

/// Regression for #6145: a delegate tool that appears for the **first time**
/// while the `tools` Arc is shared must be dispatchable, not just advertised.
///
/// This is the exact field scenario from the issue — an agent whose surface
/// carried no `delegate_to_integrations_agent` at all, a mid-session Composio
/// connect while a clone was held, and a refresh that logged
/// `added=["delegate_to_integrations_agent"] tools_reconciled=false`. The spec
/// was reconciled, the instance was not, and the policy snapshot — built from
/// the instances — had no entry for it, so the fail-closed visibility filter
/// hid the delegate: silently missing until a unique-owner refresh happened.
#[test]
fn newly_synthesized_delegate_is_executable_while_tool_arc_is_shared() {
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;

    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));

    // Turn 1 with nothing connected: no integration delegate exists yet.
    agent.refresh_delegation_tools();
    let advertised_before = agent
        .tool_specs()
        .iter()
        .any(|spec| spec.name == "delegate_to_integrations_agent");
    let executable_before = agent
        .synthesized_tools_arc()
        .iter()
        .any(|t| t.name() == "delegate_to_integrations_agent");
    assert_eq!(
        advertised_before, executable_before,
        "schema and instances must agree even before anything is connected"
    );

    // A detached sub-agent's cloned ParentExecutionContext holds a clone for
    // as long as it runs. `Arc::get_mut` would fail from here on — which is
    // what used to break the instance reconcile.
    let _shared_tools = agent.tools_arc();

    agent.set_connected_integrations(vec![
        crate::openhuman::agent::context::prompt::ConnectedIntegration {
            toolkit: "gmail".into(),
            description: "Email".into(),
            tools: vec![],
            gated_tools: vec![],
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        },
    ]);
    agent.refresh_delegation_tools();

    assert!(
        agent
            .tool_specs()
            .iter()
            .any(|spec| spec.name == "delegate_to_integrations_agent"),
        "the mid-session connect must publish the delegate spec"
    );
    assert!(
        agent
            .synthesized_tools_arc()
            .iter()
            .any(|t| t.name() == "delegate_to_integrations_agent"),
        "the delegate must also exist as an executable instance — a spec with \
         nothing registered to run it is #6145"
    );
    super::assert_synthesized_delegates_are_executable(&agent);
}
