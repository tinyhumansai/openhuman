use super::*;

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
                elapsed_ms: 0,
                success: true,
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
        // Skill
        (
            DomainEvent::SkillLoaded {
                skill_id: "s".into(),
                runtime: "quickjs".into(),
            },
            "skill",
        ),
        (
            DomainEvent::SkillStopped {
                skill_id: "s".into(),
            },
            "skill",
        ),
        (
            DomainEvent::SkillStartFailed {
                skill_id: "s".into(),
                error: "e".into(),
            },
            "skill",
        ),
        (
            DomainEvent::SkillExecuted {
                skill_id: "s".into(),
                tool_name: "t".into(),
                arguments: serde_json::Value::Null,
                result: None,
                success: true,
                elapsed_ms: 0,
            },
            "skill",
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
                request: crate::openhuman::webhooks::WebhookRequest {
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
