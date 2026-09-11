use super::*;

/// The workspace an MCP supervisor event is attributed to.
///
/// One process supervises every workspace it has opened, so these events name
/// theirs; nothing in these cases depends on which one it is (#5931).
fn mcp_workspace() -> std::path::PathBuf {
    std::path::PathBuf::from("/tmp/openhuman-ws")
}

#[test]
fn all_variants_have_correct_domain() {
    let cases: Vec<(DomainEvent, &str)> = vec![
        // Agent
        (
            DomainEvent::AgentTurnStarted {
                session_id: "s".into(),
                channel: "c".into(),
            },
            "agent",
        ),
        (
            DomainEvent::AgentTurnCompleted {
                session_id: "s".into(),
                text_chars: 0,
                iterations: 0,
            },
            "agent",
        ),
        (
            DomainEvent::AgentError {
                session_id: "s".into(),
                message: "e".into(),
                recoverable: false,
            },
            "agent",
        ),
        (
            DomainEvent::SubagentSpawned {
                parent_session: "s".into(),
                agent_id: "researcher".into(),
                mode: "typed".into(),
                task_id: "task-1".into(),
                prompt_chars: 42,
            },
            "agent",
        ),
        (
            DomainEvent::SubagentCompleted {
                parent_session: "s".into(),
                task_id: "task-1".into(),
                agent_id: "researcher".into(),
                elapsed_ms: 123,
                output_chars: 100,
                iterations: 2,
            },
            "agent",
        ),
        (
            DomainEvent::SubagentFailed {
                parent_session: "s".into(),
                task_id: "task-1".into(),
                agent_id: "researcher".into(),
                error: "boom".into(),
            },
            "agent",
        ),
        // Run Queue
        (
            DomainEvent::RunQueueMessageQueued {
                thread_id: "t".into(),
                mode: "steer".into(),
                queue_depth: 1,
            },
            "agent",
        ),
        (
            DomainEvent::RunQueueFollowupDispatched {
                thread_id: "t".into(),
                followup_count: 1,
            },
            "agent",
        ),
        (
            DomainEvent::RunQueueInterrupted {
                thread_id: "t".into(),
                cancelled_request_id: "req-1".into(),
            },
            "agent",
        ),
        // Memory
        (
            DomainEvent::MemoryStored {
                key: "k".into(),
                category: "c".into(),
                namespace: "n".into(),
            },
            "memory",
        ),
        (
            DomainEvent::MemoryRecalled {
                query: "q".into(),
                hit_count: 0,
            },
            "memory",
        ),
        // Channel
        (
            DomainEvent::ChannelInboundMessage {
                event_name: "telegram:message".into(),
                channel: "telegram".into(),
                message: "hi".into(),
                sender: None,
                reply_target: None,
                thread_ts: None,
                raw_data: serde_json::Value::Null,
            },
            "channel",
        ),
        (
            DomainEvent::ChannelMessageReceived {
                channel: "c".into(),
                message_id: "m1".into(),
                sender: "s".into(),
                reply_target: "r".into(),
                content: "hi".into(),
                thread_ts: None,
                inbound_envelope: None,
                workspace_dir: std::path::PathBuf::from("/test"),
            },
            "channel",
        ),
        (
            DomainEvent::ChannelMessageProcessed {
                channel: "c".into(),
                message_id: "m1".into(),
                sender: "s".into(),
                reply_target: "r".into(),
                content: "hi".into(),
                thread_ts: None,
                response: "hello".into(),
                provider: "test-provider".into(),
                model: "test-model".into(),
                elapsed_ms: 0,
                success: true,
                workspace_dir: std::path::PathBuf::from("/test"),
            },
            "channel",
        ),
        (
            DomainEvent::ChannelReactionReceived {
                channel: "c".into(),
                sender: "s".into(),
                target_message_id: "m1".into(),
                emoji: "👍".into(),
            },
            "channel",
        ),
        (
            DomainEvent::ChannelReactionSent {
                channel: "c".into(),
                target_message_id: "m1".into(),
                emoji: "✅".into(),
                success: true,
            },
            "channel",
        ),
        (
            DomainEvent::ChannelConnected {
                channel: "c".into(),
            },
            "channel",
        ),
        (
            DomainEvent::ChannelDisconnected {
                channel: "c".into(),
                reason: "r".into(),
            },
            "channel",
        ),
        // Cron
        (
            DomainEvent::CronJobTriggered {
                job_id: "j".into(),
                job_name: "my-job".into(),
                job_type: "t".into(),
            },
            "cron",
        ),
        (
            DomainEvent::CronJobCompleted {
                job_id: "j".into(),
                success: true,
                output: "ok".into(),
            },
            "cron",
        ),
        (
            DomainEvent::CronDeliveryRequested {
                job_id: "j".into(),
                channel: "c".into(),
                target: "t".into(),
                output: "o".into(),
            },
            "cron",
        ),
        (
            DomainEvent::ProactiveMessageRequested {
                source: "cron:morning_briefing".into(),
                message: "Good morning!".into(),
                job_name: Some("morning_briefing".into()),
            },
            "cron",
        ),
        (
            DomainEvent::FlowScheduleTick {
                flow_id: "flow-1".into(),
            },
            "cron",
        ),
        // Workflow
        (
            DomainEvent::WorkflowLoaded {
                skill_id: "s".into(),
                runtime: "nodejs".into(),
            },
            "workflow",
        ),
        (
            DomainEvent::WorkflowStopped {
                skill_id: "s".into(),
            },
            "workflow",
        ),
        (
            DomainEvent::WorkflowStartFailed {
                skill_id: "s".into(),
                error: "e".into(),
            },
            "workflow",
        ),
        (
            DomainEvent::WorkflowExecuted {
                skill_id: "s".into(),
                tool_name: "t".into(),
                arguments: serde_json::Value::Null,
                result: None,
                success: true,
                elapsed_ms: 0,
            },
            "workflow",
        ),
        // Tool
        (
            DomainEvent::ToolExecutionStarted {
                tool_name: "t".into(),
                session_id: "s".into(),
            },
            "tool",
        ),
        (
            DomainEvent::ToolExecutionCompleted {
                tool_name: "t".into(),
                session_id: "s".into(),
                success: true,
                elapsed_ms: 0,
            },
            "tool",
        ),
        // Webhook
        (
            DomainEvent::WebhookIncomingRequest {
                request: crate::openhuman::skills::webhooks::WebhookRequest {
                    correlation_id: "c".into(),
                    tunnel_id: "t".into(),
                    tunnel_uuid: "u".into(),
                    tunnel_name: "n".into(),
                    method: "GET".into(),
                    path: "/".into(),
                    headers: Default::default(),
                    query: Default::default(),
                    body: String::new(),
                },
                raw_data: serde_json::Value::Null,
            },
            "webhook",
        ),
        (
            DomainEvent::WebhookReceived {
                tunnel_id: "t".into(),
                skill_id: "s".into(),
                method: "GET".into(),
                path: "/".into(),
                correlation_id: "c".into(),
            },
            "webhook",
        ),
        (
            DomainEvent::WebhookRegistered {
                tunnel_id: "t".into(),
                skill_id: "s".into(),
                tunnel_name: None,
            },
            "webhook",
        ),
        (
            DomainEvent::WebhookUnregistered {
                tunnel_id: "t".into(),
                skill_id: "s".into(),
            },
            "webhook",
        ),
        (
            DomainEvent::WebhookProcessed {
                tunnel_id: "t".into(),
                skill_id: "s".into(),
                method: "GET".into(),
                path: "/".into(),
                correlation_id: "c".into(),
                status_code: 200,
                elapsed_ms: 0,
                error: None,
            },
            "webhook",
        ),
        // Composio
        (
            DomainEvent::ComposioTriggerReceived {
                toolkit: "gmail".into(),
                trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
                metadata_id: "trig-1".into(),
                metadata_uuid: "uuid-1".into(),
                payload: serde_json::Value::Null,
            },
            "composio",
        ),
        (
            DomainEvent::ComposioConnectionCreated {
                toolkit: "gmail".into(),
                connection_id: "conn-1".into(),
                connect_url: "https://backend.composio.dev/connect/abc".into(),
            },
            "composio",
        ),
        (
            DomainEvent::ComposioActionExecuted {
                tool: "GMAIL_SEND_EMAIL".into(),
                success: true,
                error: None,
                cost_usd: 0.0,
                elapsed_ms: 123,
            },
            "composio",
        ),
        (
            DomainEvent::ComposioIntegrationsChanged {
                toolkits: vec!["gmail".into(), "notion".into()],
            },
            "composio",
        ),
        (
            DomainEvent::ComposioConfigChanged {
                mode: "direct".into(),
                api_key_set: true,
            },
            "composio",
        ),
        // Triage
        (
            DomainEvent::TriggerEvaluated {
                source: "composio".into(),
                external_id: "uuid-1".into(),
                display_label: "composio/gmail/GMAIL_NEW_GMAIL_MESSAGE".into(),
                decision: "drop".into(),
                used_local: false,
                latency_ms: 12,
            },
            "triage",
        ),
        (
            DomainEvent::TriggerEscalated {
                source: "composio".into(),
                external_id: "uuid-1".into(),
                display_label: "composio/gmail/GMAIL_NEW_GMAIL_MESSAGE".into(),
                target_agent: "orchestrator".into(),
            },
            "triage",
        ),
        (
            DomainEvent::TriggerEscalationFailed {
                source: "composio".into(),
                external_id: "uuid-1".into(),
                reason: "parser gave up after remote retry".into(),
            },
            "triage",
        ),
        // Tree Summarizer
        (
            DomainEvent::TreeSummarizerHourCompleted {
                namespace: "n".into(),
                node_id: "2024/03/15/14".into(),
                token_count: 500,
            },
            "tree_summarizer",
        ),
        (
            DomainEvent::TreeSummarizerPropagated {
                namespace: "n".into(),
                node_id: "2024/03/15".into(),
                level: "day".into(),
                token_count: 1000,
            },
            "tree_summarizer",
        ),
        (
            DomainEvent::TreeSummarizerRebuildCompleted {
                namespace: "n".into(),
                total_nodes: 10,
            },
            "tree_summarizer",
        ),
        // Notification
        (
            DomainEvent::NotificationIngested {
                id: "n1".into(),
                provider: "slack".into(),
                account_id: None,
            },
            "notification",
        ),
        (
            DomainEvent::NotificationTriaged {
                id: "n1".into(),
                provider: "slack".into(),
                action: "escalate".into(),
                importance_score: 0.9,
                latency_ms: 150,
                routed: true,
            },
            "notification",
        ),
        // System
        (
            DomainEvent::SystemStartup {
                component: "c".into(),
            },
            "system",
        ),
        (
            DomainEvent::SystemShutdown {
                component: "c".into(),
            },
            "system",
        ),
        (
            DomainEvent::SystemRestartRequested {
                source: "rpc".into(),
                reason: "test".into(),
            },
            "system",
        ),
        (
            DomainEvent::HealthChanged {
                component: "c".into(),
                healthy: true,
                message: None,
            },
            "system",
        ),
        (
            DomainEvent::HealthRestarted {
                component: "c".into(),
            },
            "system",
        ),
        // Memory tree
        (
            DomainEvent::DocumentCanonicalized {
                source_id: "gmail:abc".into(),
                source_kind: "email".into(),
                chunks_written: 3,
                chunk_ids: vec!["c1".into(), "c2".into(), "c3".into()],
                canonicalized_at: 1_700_000_000.0,
                body_preview: Some("Thanks,\nAlice".into()),
            },
            "memory",
        ),
        // Learning
        (
            DomainEvent::CacheRebuilt {
                added: 2,
                evicted: 1,
                kept: 5,
                total_size: 7,
                rebuilt_at: 1_700_000_000.0,
            },
            "learning",
        ),
        // Auth
        (
            DomainEvent::SessionExpired {
                source: "test".into(),
                reason: "401".into(),
            },
            "auth",
        ),
        (
            DomainEvent::ProviderApiKeyRejected {
                provider: "openrouter".into(),
                message: "openrouter rejected the API key (HTTP 401).".into(),
            },
            "auth",
        ),
        // MCP reconnect supervisor (#5931)
        (
            DomainEvent::McpServerProbeTimedOut {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                probe_timeout_secs: 8,
                consecutive_timeouts: 1,
                teardown_after: 3,
                workspace_dir: mcp_workspace(),
            },
            "mcp_client",
        ),
        (
            DomainEvent::McpServerTransportDropped {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                outcome: "broken".into(),
                detail: Some("connection reset".into()),
                elapsed_ms: Some(1961),
                consecutive_timeouts: 0,
                workspace_dir: mcp_workspace(),
            },
            "mcp_client",
        ),
        (
            DomainEvent::McpServerReconnected {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                tool_count: 25,
                after_failures: 0,
                workspace_dir: mcp_workspace(),
            },
            "mcp_client",
        ),
        (
            DomainEvent::McpServerReconnectFailed {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                error: "connection refused".into(),
                failures: 1,
                retry_in_secs: 5,
                workspace_dir: mcp_workspace(),
            },
            "mcp_client",
        ),
        (
            DomainEvent::McpServerParked {
                server_id: "srv-1".into(),
                qualified_name: "@scope/server".into(),
                error: "the `uvx` launcher is not installed".into(),
                workspace_dir: mcp_workspace(),
            },
            "mcp_client",
        ),
    ];

    for (event, expected_domain) in cases {
        assert_eq!(
            event.domain(),
            expected_domain,
            "Wrong domain for {:?}",
            std::mem::discriminant(&event)
        );
    }
}

/// Regression guard. An earlier revision of
/// [`DomainEvent::ApprovalRequested`] published a `session_id`
/// field that historically carried the verbatim JSON-RPC bearer.
/// Any downstream subscriber that Debug-printed the event (audit
/// pipeline, `tracing` instrumentation, panic backtrace) leaked
/// the credential. The field has been removed from the variant;
/// this test fails loudly if it ever comes back, by name, via
/// Debug — the bus does not derive `Serialize` so the audit-side
/// risk lives entirely in the Debug surface.
#[test]
fn approval_requested_does_not_surface_session_id() {
    let event = DomainEvent::ApprovalRequested {
        request_id: "req-1".to_string(),
        tool_name: "composio".to_string(),
        action_summary: "send slack message".to_string(),
        args_redacted: serde_json::json!({ "tool_slug": "SLACK_SEND" }),
        thread_id: Some("t-1".to_string()),
        client_id: Some("c-1".to_string()),
    };
    let dbg = format!("{event:?}");
    assert!(
        !dbg.contains("session_id"),
        "ApprovalRequested Debug must not surface session_id: {dbg}"
    );
}

#[test]
fn workflows_changed_domain_and_name() {
    let event = DomainEvent::WorkflowsChanged {
        reason: "install".into(),
    };
    assert_eq!(event.domain(), "workflow");
    assert_eq!(event.variant_name(), "WorkflowsChanged");
}

#[test]
fn memory_driver_bind_failed_domain_and_name() {
    let event = DomainEvent::MemoryDriverBindFailed {
        configured_driver: "supermemory".into(),
        bound_driver: "null".into(),
        reason: "external driver is untrusted".into(),
    };
    assert_eq!(event.domain(), "memory");
    assert_eq!(event.variant_name(), "MemoryDriverBindFailed");
}

/// The Event Log's "agent" column is the only per-row context the stream
/// carries, so every MCP row must name its server there (#5931): the
/// supervisor's verdicts by the registry name a user knows the server by, the
/// RPC-driven lifecycle by the install id.
#[test]
fn mcp_supervisor_events_name_themselves_and_hint_the_server() {
    let cases: Vec<(DomainEvent, &str)> = vec![
        (
            DomainEvent::McpServerProbeTimedOut {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                probe_timeout_secs: 8,
                consecutive_timeouts: 1,
                teardown_after: 3,
                workspace_dir: mcp_workspace(),
            },
            "McpServerProbeTimedOut",
        ),
        (
            DomainEvent::McpServerTransportDropped {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                outcome: "timed_out".into(),
                detail: None,
                elapsed_ms: Some(8_000),
                consecutive_timeouts: 3,
                workspace_dir: mcp_workspace(),
            },
            "McpServerTransportDropped",
        ),
        (
            DomainEvent::McpServerReconnected {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                tool_count: 25,
                after_failures: 1,
                workspace_dir: mcp_workspace(),
            },
            "McpServerReconnected",
        ),
        (
            DomainEvent::McpServerReconnectFailed {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                error: "connection refused".into(),
                failures: 1,
                retry_in_secs: 5,
                workspace_dir: mcp_workspace(),
            },
            "McpServerReconnectFailed",
        ),
        (
            DomainEvent::McpServerParked {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                error: "the `uvx` launcher is not installed".into(),
                workspace_dir: mcp_workspace(),
            },
            "McpServerParked",
        ),
    ];

    for (event, expected_name) in cases {
        assert_eq!(event.variant_name(), expected_name);
        assert_eq!(event.domain(), "mcp_client");
        assert_eq!(
            event.agent_hint(),
            Some("ac.inference.sh/mcp"),
            "{expected_name} should hint the registry name"
        );
    }
}

#[test]
fn mcp_lifecycle_events_hint_the_install_id() {
    let cases = vec![
        DomainEvent::McpServerInstalled {
            server_id: "srv-1".into(),
            qualified_name: "ac.inference.sh/mcp".into(),
        },
        DomainEvent::McpServerConnected {
            server_id: "srv-1".into(),
            tool_count: 3,
        },
        DomainEvent::McpServerDisconnected {
            server_id: "srv-1".into(),
            reason: Some("disabled".into()),
        },
    ];

    for event in cases {
        assert_eq!(
            event.agent_hint(),
            Some("srv-1"),
            "{} should hint the install id",
            event.variant_name()
        );
    }
}

/// The Event Log envelope carries the variant name, the agent hint and a
/// timestamp — no payload — so the supervisor variants attach the one line a
/// reader needs to tell a broken transport from a timed-out one (#5931).
#[test]
fn mcp_supervisor_events_summarise_themselves_for_the_event_log() {
    let cases: Vec<(DomainEvent, Vec<&str>)> = vec![
        (
            DomainEvent::McpServerProbeTimedOut {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                probe_timeout_secs: 8,
                consecutive_timeouts: 1,
                teardown_after: 3,
                workspace_dir: mcp_workspace(),
            },
            vec!["8s", "1", "3"],
        ),
        (
            DomainEvent::McpServerTransportDropped {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                outcome: "broken".into(),
                detail: Some("connection reset".into()),
                elapsed_ms: Some(1961),
                consecutive_timeouts: 0,
                workspace_dir: mcp_workspace(),
            },
            vec!["broken", "1961ms", "connection reset"],
        ),
        (
            DomainEvent::McpServerReconnected {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                tool_count: 25,
                after_failures: 2,
                workspace_dir: mcp_workspace(),
            },
            vec!["25 tools", "2 failed"],
        ),
        (
            DomainEvent::McpServerReconnectFailed {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                error: "connection refused".into(),
                failures: 1,
                retry_in_secs: 5,
                workspace_dir: mcp_workspace(),
            },
            vec!["attempt 1", "5s", "connection refused"],
        ),
        (
            DomainEvent::McpServerParked {
                server_id: "srv-1".into(),
                qualified_name: "ac.inference.sh/mcp".into(),
                error: "the `uvx` launcher is not installed".into(),
                workspace_dir: mcp_workspace(),
            },
            vec!["parked", "uvx"],
        ),
    ];

    for (event, expected) in cases {
        let detail = event
            .log_detail()
            .unwrap_or_else(|| panic!("{} should summarise itself", event.variant_name()));
        for fragment in expected {
            assert!(
                detail.contains(fragment),
                "{} detail {detail:?} is missing {fragment:?}",
                event.variant_name()
            );
        }
        assert!(
            !detail.contains("/tmp/openhuman-ws"),
            "{} must not print its workspace into a shared panel: {detail:?}",
            event.variant_name()
        );
    }
}

/// A transport drop with nothing measured still says what happened.
#[test]
fn a_missing_entry_drop_summarises_without_a_measurement() {
    let detail = DomainEvent::McpServerTransportDropped {
        server_id: "srv-1".into(),
        qualified_name: "ac.inference.sh/mcp".into(),
        outcome: "missing".into(),
        detail: None,
        elapsed_ms: None,
        consecutive_timeouts: 0,
        workspace_dir: mcp_workspace(),
    }
    .log_detail()
    .expect("a drop always summarises");
    assert_eq!(detail, "session ended: missing");
}

/// One row cannot flood the log, and truncation never splits a character.
#[test]
fn a_long_error_is_clipped_on_a_character_boundary() {
    let detail = DomainEvent::McpServerParked {
        server_id: "srv-1".into(),
        qualified_name: "ac.inference.sh/mcp".into(),
        error: "é".repeat(400),
        workspace_dir: mcp_workspace(),
    }
    .log_detail()
    .expect("a parked server always summarises");
    assert!(detail.ends_with('…'), "{detail:?}");
    assert_eq!(detail.chars().filter(|c| *c == 'é').count(), 160);
}

/// Every other variant is unchanged: no detail, so its row renders as before.
#[test]
fn events_outside_the_supervisor_have_no_event_log_detail() {
    assert!(DomainEvent::CronJobCompleted {
        job_id: "job-1".into(),
        success: true,
        output: "done".into(),
    }
    .log_detail()
    .is_none());
    assert!(DomainEvent::McpServerConnected {
        server_id: "srv-1".into(),
        tool_count: 25,
    }
    .log_detail()
    .is_none());
}

// ── workspace_dir ────────────────────────────────────────────────────────

/// A workspace that is not [`mcp_workspace`], so a test can tell "the
/// accessor returned *a* path" apart from "the accessor returned *this*
/// path".
fn other_workspace() -> std::path::PathBuf {
    std::path::PathBuf::from("/tmp/openhuman-other-ws")
}

/// Every variant that carries a workspace must be reachable through the one
/// accessor. The bug this guards is not a missing arm in the abstract: before
/// #5966 the notification bridge kept its own list, it named only the MCP
/// family, and the channel and artifact families — which carry the same field
/// — were silently ungated. A consumer that filters by workspace must be able
/// to ask one question and get every bound event, or it covers a subset and
/// nothing says which.
#[test]
fn every_workspace_bound_variant_is_reachable_through_one_accessor() {
    let ws = other_workspace();
    let cases: Vec<DomainEvent> = vec![
        DomainEvent::ChannelMessageReceived {
            channel: "c".into(),
            message_id: "m1".into(),
            sender: "s".into(),
            reply_target: "r".into(),
            content: "hi".into(),
            thread_ts: None,
            inbound_envelope: None,
            workspace_dir: ws.clone(),
        },
        DomainEvent::ChannelMessageProcessed {
            channel: "c".into(),
            message_id: "m1".into(),
            sender: "s".into(),
            reply_target: "r".into(),
            content: "hi".into(),
            thread_ts: None,
            response: "ok".into(),
            provider: "p".into(),
            model: "m".into(),
            elapsed_ms: 1,
            success: true,
            workspace_dir: ws.clone(),
        },
        DomainEvent::ArtifactReady {
            artifact_id: "a1".into(),
            kind: "document".into(),
            title: "t".into(),
            workspace_dir: ws.to_string_lossy().into_owned(),
            path: "a1/doc.docx".into(),
            size_bytes: 1,
            thread_id: None,
            client_id: None,
        },
        DomainEvent::ArtifactFailed {
            artifact_id: "a1".into(),
            kind: "document".into(),
            title: "t".into(),
            workspace_dir: ws.to_string_lossy().into_owned(),
            error: "boom".into(),
            thread_id: None,
            client_id: None,
        },
        DomainEvent::ArtifactPending {
            artifact_id: "a1".into(),
            kind: "document".into(),
            title: "t".into(),
            workspace_dir: ws.to_string_lossy().into_owned(),
            path: "a1/doc.docx".into(),
            thread_id: None,
            client_id: None,
        },
        DomainEvent::McpServerProbeTimedOut {
            server_id: "srv-1".into(),
            qualified_name: "ac.inference.sh/mcp".into(),
            probe_timeout_secs: 8,
            consecutive_timeouts: 1,
            teardown_after: 3,
            workspace_dir: ws.clone(),
        },
        DomainEvent::McpServerTransportDropped {
            server_id: "srv-1".into(),
            qualified_name: "ac.inference.sh/mcp".into(),
            outcome: "timed_out".into(),
            detail: None,
            elapsed_ms: Some(8_000),
            consecutive_timeouts: 3,
            workspace_dir: ws.clone(),
        },
        DomainEvent::McpServerReconnected {
            server_id: "srv-1".into(),
            qualified_name: "ac.inference.sh/mcp".into(),
            tool_count: 4,
            after_failures: 1,
            workspace_dir: ws.clone(),
        },
        DomainEvent::McpServerReconnectFailed {
            server_id: "srv-1".into(),
            qualified_name: "ac.inference.sh/mcp".into(),
            error: "refused".into(),
            failures: 1,
            retry_in_secs: 30,
            workspace_dir: ws.clone(),
        },
        DomainEvent::McpServerParked {
            server_id: "srv-1".into(),
            qualified_name: "ac.inference.sh/mcp".into(),
            error: "refused".into(),
            workspace_dir: ws.clone(),
        },
    ];

    for event in cases {
        assert_eq!(
            event.workspace_dir(),
            Some(ws.as_path()),
            "{} carries a workspace but the accessor did not return it",
            event.variant_name()
        );
    }
}

/// `None` here means "not bound to a workspace", which a consumer filtering
/// by workspace must let through rather than drop. Getting this backwards
/// would silence cron, webhook and sub-agent notifications entirely.
#[test]
fn events_without_a_workspace_report_none() {
    assert_eq!(
        DomainEvent::AgentPathsChanged.workspace_dir(),
        None,
        "a process-wide event must not claim a workspace"
    );
    assert_eq!(
        DomainEvent::CronJobCompleted {
            job_id: "job-1".into(),
            success: true,
            output: "done".into(),
        }
        .workspace_dir(),
        None
    );
    assert_eq!(
        DomainEvent::ChannelReactionReceived {
            channel: "c".into(),
            sender: "s".into(),
            target_message_id: "m1".into(),
            emoji: "👍".into(),
        }
        .workspace_dir(),
        None,
        "a channel event with no workspace field must not borrow one"
    );
}

/// The artifact family carries its workspace as a `String`, so it is the one
/// place an *empty* value is representable. Reading that back as the empty
/// path would bind the event to a workspace nothing matches, hiding it from
/// every scoped consumer — strictly worse than reporting it unbound, which at
/// least shows it everywhere.
#[test]
fn an_empty_artifact_workspace_reads_as_unbound_not_as_a_workspace() {
    assert_eq!(
        DomainEvent::ArtifactReady {
            artifact_id: "a1".into(),
            kind: "document".into(),
            title: "t".into(),
            workspace_dir: String::new(),
            path: "a1/doc.docx".into(),
            size_bytes: 1,
            thread_id: None,
            client_id: None,
        }
        .workspace_dir(),
        None
    );
}

/// The switch announcement names a workspace but is deliberately not *bound*
/// to one: it is what tells a consumer the active workspace changed, so
/// filtering it by the rule it announces would hide the announcement from
/// exactly the consumers that are still out of date.
#[test]
fn the_workspace_switch_announcement_is_not_itself_workspace_bound() {
    let event = DomainEvent::ActiveWorkspaceChanged {
        workspace_dir: other_workspace(),
        revision: 1,
    };
    assert_eq!(event.workspace_dir(), None);
    assert_eq!(event.variant_name(), "ActiveWorkspaceChanged");
    assert_eq!(DomainEvent::domain(&event), "system");
}
