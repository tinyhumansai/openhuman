//! Tests for the channel approval-surface and turn-state subscribers.

use super::*;
use crate::channels::runtime::{ChannelHandle, TestChannel};
use std::path::Path;
#[cfg(feature = "e2e-test-support")]
use tempfile::tempdir;

fn recording(name: &str) -> (Arc<ChannelHandle>, Arc<TestChannel>) {
    let channel = Arc::new(TestChannel::new(name));
    (Arc::new(ChannelHandle::Test(Arc::clone(&channel))), channel)
}

fn approval_subscriber(
    channels: &[(&str, Arc<ChannelHandle>)],
) -> ChannelApprovalSurfaceSubscriber {
    let map: HashMap<String, Arc<ChannelHandle>> = channels
        .iter()
        .map(|(name, channel)| ((*name).to_owned(), Arc::clone(channel)))
        .collect();
    ChannelApprovalSurfaceSubscriber::new(Arc::new(map))
}

/// A turn-state subscriber routed through the module object the runtime lease
/// opened, as production routes through the configured module objects, with
/// an in-memory record of every mark and whether its dispatch succeeded.
#[cfg(feature = "e2e-test-support")]
fn turn_state_subscriber(
    workspace_dir: PathBuf,
    names: &[&str],
    lease: &crate::modules::channel_runtime::ChannelRuntimeLease,
) -> (ChannelTurnStateSubscriber, TurnStateMarks) {
    let objects: HashMap<String, ModuleObjectClient> = names
        .iter()
        .map(|name| ((*name).to_owned(), lease.object().clone()))
        .collect();
    let mut subscriber = ChannelTurnStateSubscriber::with_objects(workspace_dir, Arc::new(objects));
    let marks = Arc::new(Mutex::new(Vec::new()));
    subscriber.test_marks = Some(Arc::clone(&marks));
    (subscriber, marks)
}

#[cfg(feature = "e2e-test-support")]
struct TurnStateHostCallback;

#[cfg(feature = "e2e-test-support")]
#[tinybus::interface(name = "ai.tinyhumans.tinychannels.ChannelsHost")]
impl TurnStateHostCallback {
    async fn dispatch_controller_backend(
        &self,
        _request: tinychannels_bus::ChannelBackendRequest,
    ) -> tinybus::Result<tinychannels_bus::ChannelBackendResult> {
        Err(tinybus::Error::failed("unused turn-state callback"))
    }
}

#[cfg(feature = "e2e-test-support")]
async fn open_turn_state_runtime() -> crate::modules::channel_runtime::ChannelRuntimeLease {
    let client = crate::channels::module_fixture::client().await;
    crate::modules::channel_runtime::ChannelRuntimeLease::open(client, TurnStateHostCallback)
        .await
        .expect("retain the host callback for remote turn-state calls")
}

fn received(channel: &str, reply_target: &str, thread_ts: Option<&str>, ws: &Path) -> DomainEvent {
    DomainEvent::ChannelMessageReceived {
        channel: channel.into(),
        message_id: "m1".into(),
        sender: "alice".into(),
        reply_target: reply_target.into(),
        content: "hi".into(),
        thread_ts: thread_ts.map(str::to_string),
        inbound_envelope: None,
        workspace_dir: ws.to_path_buf(),
    }
}

#[cfg(feature = "e2e-test-support")]
fn processed(channel: &str, reply_target: &str, ws: &Path) -> DomainEvent {
    DomainEvent::ChannelMessageProcessed {
        channel: channel.into(),
        message_id: "m1".into(),
        sender: "alice".into(),
        reply_target: reply_target.into(),
        content: "hi".into(),
        thread_ts: None,
        response: "ok".into(),
        provider: "test-provider".into(),
        model: "test-model".into(),
        elapsed_ms: 10,
        success: true,
        workspace_dir: ws.to_path_buf(),
    }
}

fn approval(thread_id: Option<&str>, client_id: Option<&str>) -> DomainEvent {
    DomainEvent::ApprovalRequested {
        request_id: "req-1".into(),
        tool_name: "file_write".into(),
        action_summary: "Write notes/today.md (1.2 KiB)".into(),
        args_redacted: serde_json::json!({"path": "notes/today.md"}),
        thread_id: thread_id.map(str::to_string),
        client_id: client_id.map(str::to_string),
        tool_call_id: None,
        expires_at: None,
        agent_id: None,
    }
}

#[tokio::test]
async fn approval_prompts_reach_every_chat_channel() {
    let ws = PathBuf::from("/tmp");
    for (channel, thread_ts, key) in [
        ("telegram", Some("7"), "telegram_alice_chat-1"),
        ("discord", None, "discord_alice_chat-1"),
        ("slack", Some("1700.1"), "slack_alice_chat-1_thread:1700.1"),
    ] {
        let (handle, ch) = recording(channel);
        let sub = approval_subscriber(&[(channel, handle)]);
        assert_eq!(sub.name(), "channels::approval_surface");
        assert_eq!(sub.domains(), Some(&["channel", "approval"][..]));
        sub.handle(&received(channel, "chat-1", thread_ts, &ws))
            .await;
        assert!(sub.replies.lock().unwrap().contains_key(key), "{channel}");
        sub.handle(&approval(Some(key), Some(channel))).await;
        let sent = ch.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1, "{channel}");
        assert_eq!(sent[0].recipient, "chat-1");
        assert_eq!(sent[0].thread_ts.as_deref(), thread_ts);
        assert!(sent[0].content.contains("file_write"));
    }
}

#[tokio::test]
async fn approvals_for_web_email_or_unrouted_requests_are_ignored() {
    let ws = PathBuf::from("/tmp");
    let (email_handle, email) = recording("email");
    let (discord_handle, discord) = recording("discord");
    let sub = approval_subscriber(&[("email", email_handle), ("discord", discord_handle)]);
    sub.handle(&received("email", "a@b.c", None, &ws)).await;
    sub.handle(&approval(Some("email_alice_a@b.c"), Some("email")))
        .await;
    sub.handle(&approval(Some("web-thread"), Some("web-client-1")))
        .await;
    sub.handle(&approval(Some("x"), None)).await;
    sub.handle(&approval(None, Some("discord"))).await;
    // No recorded context for this conversation → nothing sent.
    sub.handle(&approval(Some("discord_bob_c"), Some("discord")))
        .await;
    sub.handle(&DomainEvent::SystemStartup {
        component: "test".into(),
    })
    .await;
    assert!(email.sent.lock().unwrap().is_empty());
    assert!(discord.sent.lock().unwrap().is_empty());
}

// Busy flags live in the module's own storage scope and are written by the
// module, so this runs against the linked-module fixture
// (`channels::module_fixture`).
#[tokio::test]
#[cfg(feature = "e2e-test-support")]
async fn turn_state_marks_busy_for_every_remote_control_channel() {
    let _storage = crate::channels::module_fixture::install_memory_storage();
    let lease = open_turn_state_runtime().await;
    let dir = tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    let (sub, marks) = turn_state_subscriber(ws.clone(), &["telegram", "discord"], &lease);
    assert_eq!(sub.name(), "channels::turn_state");
    assert_eq!(sub.domains(), Some(&["channel"][..]));
    for channel in ["telegram", "discord"] {
        sub.handle(&received(channel, "chat-99", None, &ws)).await;
        assert_eq!(
            marks.lock().unwrap().last(),
            Some(&(channel.to_owned(), "chat-99".to_owned(), true, true))
        );

        sub.handle(&processed(channel, "chat-99", &ws)).await;
        assert_eq!(
            marks.lock().unwrap().last(),
            Some(&(channel.to_owned(), "chat-99".to_owned(), false, true))
        );
    }
    lease.close().await.unwrap();
}

#[tokio::test]
#[cfg(feature = "e2e-test-support")]
async fn turn_state_ignores_non_remote_channels_and_stale_workspaces() {
    let _storage = crate::channels::module_fixture::install_memory_storage();
    let lease = open_turn_state_runtime().await;
    let dir = tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    let (sub, marks) = turn_state_subscriber(ws.clone(), &["telegram"], &lease);
    let other = tempdir().unwrap().path().to_path_buf();
    sub.handle(&received("email", "a@b.c", None, &ws)).await;
    assert!(marks.lock().unwrap().is_empty());

    sub.handle(&received("telegram", "chat-1", None, &ws)).await;
    assert_eq!(
        marks.lock().unwrap().as_slice(),
        &[("telegram".into(), "chat-1".into(), true, true)]
    );

    sub.handle(&processed("telegram", "chat-1", &other)).await;
    assert_eq!(
        marks.lock().unwrap().len(),
        1,
        "a stale workspace is ignored"
    );

    sub.handle(&processed("telegram", "chat-1", &ws)).await;
    assert_eq!(
        marks.lock().unwrap().as_slice(),
        &[
            ("telegram".into(), "chat-1".into(), true, true),
            ("telegram".into(), "chat-1".into(), false, true),
        ]
    );
    lease.close().await.unwrap();
}
