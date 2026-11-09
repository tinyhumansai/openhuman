//! Channel-origin delivery: the reply is validated against the captured
//! session identity, sent through the live channel module, and appended to the
//! tenant's durable session transcript.

use super::tests::{config, job};
use super::*;
use crate::core::runtime::{CoreContext, DomainSet};
use std::sync::{Arc, Mutex};
use tinyagents_session::transcript::SessionRef;
use tinyflows_bus::schedule::{ChannelConversationIdentity, ChannelSessionIdentity};

fn ctx_for(config: &Config) -> DispatchCtx {
    DispatchCtx::new(CoreContext::for_test(
        DomainSet::none(),
        Some(config.workspace_dir.clone()),
    ))
}

fn identity(ctx: &DispatchCtx, config: &Config, account: &str) -> ChannelSessionIdentity {
    let tenant = ctx.tenant().unwrap();
    ChannelSessionIdentity {
        tenant_key: crate::core::runtime::tenant_key(&tenant, ""),
        agent_id: crate::web_chat::pick_target_agent_id(config),
        session_key: "channel-session-v22:cron-origin".into(),
        provider: "telegram".into(),
        account_id: Some(account.into()),
        conversation: ChannelConversationIdentity {
            kind: "dm".into(),
            id: "42".into(),
            scope_id: None,
            parent_id: None,
            thread_id: None,
            topic_id: None,
        },
    }
}

fn channel_origin(identity: Option<ChannelSessionIdentity>) -> JobOrigin {
    JobOrigin::Channel {
        channel: "telegram".into(),
        reply_target: "42".into(),
        history_key: "legacy".into(),
        sender: None,
        thread_id: None,
        channel_session: identity.map(Box::new),
    }
}

fn account_of(
    objects: &std::collections::HashMap<String, tinybus::module::ModuleObjectClient>,
) -> String {
    objects
        .keys()
        .find(|key| tinychannels_bus::provider_id(key) == "telegram")
        .expect("the linked runtime configures a telegram account")
        .clone()
}

/// A prior live turn, written the way the session host does, so the cron
/// append has a session to extend (a channel that never spoke has none).
fn seed_session(ctx: &DispatchCtx, config: &Config, identity: &ChannelSessionIdentity) {
    use tinyagents_session::transcript::{TranscriptMessage, TranscriptTurn};
    let tenant = ctx.tenant().unwrap();
    let locator = origin_channel_transcripts(ctx, config, identity, &identity.agent_id).unwrap();
    let session = SessionRef::scoped(
        crate::core::runtime::tenant_key(&tenant, &identity.session_key),
        &identity.agent_id,
    );
    let meta = super::tests::meta();
    let next = vec![
        TranscriptMessage::user("remind me"),
        TranscriptMessage::assistant("Scheduled."),
    ];
    locator
        .open_session(&session, meta.clone())
        .unwrap()
        .append_turn(TranscriptTurn {
            prev: &[],
            next: &next,
            meta: &meta,
            turn_usage: None,
            request_id: None,
            tools: None,
        })
        .unwrap();
}

fn assistant_messages(
    ctx: &DispatchCtx,
    config: &Config,
    identity: &ChannelSessionIdentity,
) -> Vec<String> {
    let tenant = ctx.tenant().unwrap();
    let locator = origin_channel_transcripts(ctx, config, identity, &identity.agent_id).unwrap();
    let session = SessionRef::scoped(
        crate::core::runtime::tenant_key(&tenant, &identity.session_key),
        &identity.agent_id,
    );
    let head = locator.head_generation(&session);
    locator
        .read_session_transcript(&head)
        .and_then(|t| t.read_session().ok().flatten())
        .map(|s| {
            s.messages
                .into_iter()
                .filter(|m| m.role == "assistant")
                .map(|m| m.content)
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn channel_origin_reply_reaches_the_provider_and_the_durable_session() {
    let linked = crate::channels::runtime::test_support::linked_telegram_runtime().await;
    let objects = linked.module_objects();
    let account = account_of(&objects);
    crate::cron::channel_bridge::register_channel_bridge(objects);
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    let ctx = ctx_for(&cfg);
    let ident = identity(&ctx, &cfg, &account);
    seed_session(&ctx, &cfg, &ident);
    let j = job(Some(channel_origin(Some(ident.clone()))));

    let status = deliver_to_origin(&ctx, &cfg, &j, "run-chan-1", "Drink water!").await;
    crate::cron::channel_bridge::clear_channel_bridge();

    assert_eq!(status.unwrap(), DeliveryStatus::Delivered);
    assert!(linked
        .provider_request_paths()
        .await
        .iter()
        .any(|p| p.ends_with("/sendMessage")));
    assert_eq!(
        assistant_messages(&ctx, &cfg, &ident),
        ["Scheduled.", "Drink water!"]
    );
}

#[tokio::test]
async fn channel_origin_without_a_session_identity_is_rejected_before_sending() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    let ctx = ctx_for(&cfg);
    let j = job(Some(channel_origin(None)));
    let err = deliver_to_origin(&ctx, &cfg, &j, "run-x", "hi")
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("no durable session identity"),
        "{err}"
    );
}

#[tokio::test]
async fn channel_origin_captured_under_another_tenant_or_agent_is_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    let ctx = ctx_for(&cfg);

    let mut foreign = identity(&ctx, &cfg, "telegram");
    foreign.tenant_key = "another-tenant".into();
    let err = deliver_to_origin(
        &ctx,
        &cfg,
        &job(Some(channel_origin(Some(foreign)))),
        "r",
        "hi",
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("invalid channel origin"), "{err}");
    assert!(err.to_string().contains("different tenant"), "{err}");

    let mut stale_agent = identity(&ctx, &cfg, "telegram");
    stale_agent.agent_id = "no-such-agent".into();
    let err = deliver_to_origin(
        &ctx,
        &cfg,
        &job(Some(channel_origin(Some(stale_agent)))),
        "r",
        "hi",
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("no longer matches"), "{err}");
}

#[tokio::test]
async fn channel_origin_fails_when_the_channel_runtime_is_not_running() {
    crate::cron::channel_bridge::clear_channel_bridge();
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    let ctx = ctx_for(&cfg);
    let ident = identity(&ctx, &cfg, "telegram");
    let err = deliver_to_origin(
        &ctx,
        &cfg,
        &job(Some(channel_origin(Some(ident.clone())))),
        "r",
        "hi",
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("channel runtime is not running"),
        "{err}"
    );
    assert!(assistant_messages(&ctx, &cfg, &ident).is_empty());
}

struct Capture(Mutex<Vec<crate::cron::host_delivery::CronReply>>);

#[async_trait::async_trait]
impl crate::cron::host_delivery::CronDelivery for Capture {
    async fn deliver(&self, reply: crate::cron::host_delivery::CronReply) -> Result<(), String> {
        self.0.lock().unwrap().push(reply);
        Ok(())
    }
}

#[tokio::test]
async fn host_routed_channel_origin_reply_is_also_recorded_in_the_session_transcript() {
    use crate::core::runtime::ContextOverlay;
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    let core = CoreContext::for_test(DomainSet::none(), Some(cfg.workspace_dir.clone()))
        .derive_with(
            ContextOverlay::new(cfg.clone(), DomainSet::none(), Default::default())
                .profile("cron-origin-profile"),
        );
    let ctx = DispatchCtx::new(core);
    let tenant = ctx.tenant().unwrap();
    assert!(tenant.profile.is_some());
    let ident = ChannelSessionIdentity {
        tenant_key: crate::core::runtime::tenant_key(&tenant, ""),
        agent_id: "orchestrator".into(),
        ..identity(&ctx, &cfg, "telegram")
    };
    let sink = Arc::new(Capture(Mutex::new(Vec::new())));
    let previous = crate::cron::host_delivery::install(sink.clone());

    seed_session(&ctx, &cfg, &ident);
    let j = job(Some(channel_origin(Some(ident.clone()))));
    let status = deliver_to_origin(&ctx, &cfg, &j, "run-host-1", "Host reply").await;

    crate::cron::host_delivery::clear();
    if let Some(previous) = previous {
        crate::cron::host_delivery::install(previous);
    }
    assert_eq!(status.unwrap(), DeliveryStatus::Delivered);
    let replies = sink.0.lock().unwrap().clone();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].text, "Host reply");
    assert_eq!(
        replies[0].profile_id.as_deref(),
        Some("cron-origin-profile")
    );
    assert_eq!(
        assistant_messages(&ctx, &cfg, &ident),
        ["Scheduled.", "Host reply"]
    );
}
