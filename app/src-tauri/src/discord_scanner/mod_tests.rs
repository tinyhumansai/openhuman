use super::*;

#[test]
fn gateway_guild_create_and_message_create_build_channel_snapshot() {
    let mut state = DiscordIngestState::default();
    state.apply_gateway_payload(
        r#"{
            "op":0,
            "t":"GUILD_CREATE",
            "d":{
                "id":"guild-1",
                "channels":[{"id":"chan-1","name":"general"}],
                "threads":[]
            }
        }"#,
    );

    let snapshots = state.apply_gateway_payload(
        r#"{
            "op":0,
            "t":"MESSAGE_CREATE",
            "d":{
                "id":"msg-1",
                "channel_id":"chan-1",
                "guild_id":"guild-1",
                "content":"hello discord",
                "timestamp":"2026-05-17T12:34:56.000Z",
                "author":{"id":"user-1","username":"alice","global_name":"Alice"},
                "member":{"nick":"Ali"}
            }
        }"#,
    );

    assert_eq!(snapshots.len(), 1);
    let snapshot = &snapshots[0];
    assert_eq!(snapshot.channel_id, "chan-1");
    assert_eq!(snapshot.channel_name, "general");
    assert_eq!(snapshot.guild_id.as_deref(), Some("guild-1"));
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(snapshot.messages[0].author, "Ali");
    assert_eq!(snapshot.messages[0].body, "hello discord");
    assert_eq!(
        snapshot.messages[0].source_ref,
        "https://discord.com/channels/guild-1/chan-1/msg-1"
    );
}

#[test]
fn channel_create_dm_recipients_become_channel_name() {
    let mut state = DiscordIngestState::default();
    let snapshots = state.apply_gateway_payload(
        r#"{
            "op":0,
            "t":"CHANNEL_CREATE",
            "d":{
                "id":"dm-1",
                "type":1,
                "recipients":[
                    {"id":"u1","username":"alice"},
                    {"id":"u2","global_name":"Bob Builder"}
                ]
            }
        }"#,
    );

    assert!(snapshots.is_empty());
    let channel = state.channels.get("dm-1").expect("dm channel cached");
    assert_eq!(channel.name.as_deref(), Some("alice, Bob Builder"));
}

#[test]
fn channel_update_emits_snapshot_when_messages_are_already_cached() {
    let mut state = DiscordIngestState::default();
    let _ = state.apply_gateway_payload(
        r#"{
            "op":0,
            "t":"MESSAGE_CREATE",
            "d":{
                "id":"msg-1",
                "channel_id":"chan-1",
                "guild_id":"guild-1",
                "content":"hello",
                "timestamp":"2026-05-17T12:34:56.000Z",
                "author":{"id":"user-1","username":"alice"}
            }
        }"#,
    );

    let snapshots = state.apply_gateway_payload(
        r#"{
            "op":0,
            "t":"CHANNEL_UPDATE",
            "d":{
                "id":"chan-1",
                "guild_id":"guild-1",
                "name":"renamed-general"
            }
        }"#,
    );

    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].channel_name, "renamed-general");
    assert_eq!(snapshots[0].messages.len(), 1);
}

#[test]
fn message_update_replaces_existing_message_body() {
    let mut state = DiscordIngestState::default();
    let _ = state.apply_gateway_payload(
        r#"{
            "op":0,
            "t":"MESSAGE_CREATE",
            "d":{
                "id":"msg-1",
                "channel_id":"chan-1",
                "content":"before",
                "timestamp":"2026-05-17T12:34:56.000Z",
                "author":{"id":"user-1","username":"alice"}
            }
        }"#,
    );

    let snapshots = state.apply_gateway_payload(
        r#"{
            "op":0,
            "t":"MESSAGE_UPDATE",
            "d":{
                "id":"msg-1",
                "channel_id":"chan-1",
                "content":"after",
                "timestamp":"2026-05-17T12:34:56.000Z",
                "author":{"id":"user-1","username":"alice"}
            }
        }"#,
    );

    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].messages.len(), 1);
    assert_eq!(snapshots[0].messages[0].body, "after");
}

#[test]
fn message_update_preserves_missing_fields_from_cached_message() {
    let mut state = DiscordIngestState::default();
    let _ = state.apply_gateway_payload(
        r#"{
            "op":0,
            "t":"MESSAGE_CREATE",
            "d":{
                "id":"msg-1",
                "channel_id":"chan-1",
                "guild_id":"guild-1",
                "content":"before",
                "timestamp":"2026-05-17T12:34:56.000Z",
                "author":{"id":"user-1","username":"alice"}
            }
        }"#,
    );

    let snapshots = state.apply_gateway_payload(
        r#"{
            "op":0,
            "t":"MESSAGE_UPDATE",
            "d":{
                "id":"msg-1",
                "channel_id":"chan-1",
                "guild_id":"guild-1",
                "edited_timestamp":"2026-05-17T12:35:56.000Z"
            }
        }"#,
    );

    assert_eq!(snapshots.len(), 1);
    let message = &snapshots[0].messages[0];
    assert_eq!(message.body, "before");
    assert_eq!(message.author, "alice");
    assert_eq!(message.author_id, "user-1");
    assert_eq!(
        message.timestamp_ms,
        parse_discord_timestamp_ms("2026-05-17T12:34:56.000Z").unwrap()
    );
}

#[test]
fn message_update_with_embed_only_keeps_existing_body_text() {
    let mut state = DiscordIngestState::default();
    let _ = state.apply_gateway_payload(
        r#"{
            "op":0,
            "t":"MESSAGE_CREATE",
            "d":{
                "id":"msg-1",
                "channel_id":"chan-1",
                "content":"before",
                "timestamp":"2026-05-17T12:34:56.000Z",
                "author":{"id":"user-1","username":"alice"}
            }
        }"#,
    );

    let snapshots = state.apply_gateway_payload(
        r#"{
            "op":0,
            "t":"MESSAGE_UPDATE",
            "d":{
                "id":"msg-1",
                "channel_id":"chan-1",
                "embeds":[{"title":"preview card"}]
            }
        }"#,
    );

    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].messages[0].body, "before");
}

#[test]
fn discord_channel_doc_key_scopes_same_channel_name_by_guild() {
    assert_eq!(
        discord_channel_doc_key("guild-1", "chan-1"),
        "guild-1:chan-1"
    );
    assert_eq!(
        discord_channel_doc_key("guild-2", "chan-1"),
        "guild-2:chan-1"
    );
    assert_eq!(discord_channel_doc_key("", "chan-1"), "@me:chan-1");
}

fn insert_pending_tasks(
    registry: &ScannerRegistry,
    account_id: &str,
    count: usize,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut tasks = Vec::with_capacity(count);
    let mut abort_handles = Vec::with_capacity(count);
    for _ in 0..count {
        let task = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        abort_handles.push(task.abort_handle());
        tasks.push(task);
    }
    registry
        .started
        .lock()
        .insert(account_id.to_string(), abort_handles);
    tasks
}

async fn assert_cancelled(task: tokio::task::JoinHandle<()>) {
    let err = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("aborted scanner task should finish")
        .expect_err("scanner task should be cancelled");
    assert!(err.is_cancelled());
}

async fn assert_all_cancelled(tasks: Vec<tokio::task::JoinHandle<()>>) {
    for task in tasks {
        assert_cancelled(task).await;
    }
}

#[tokio::test]
async fn registry_forget_aborts_all_handles_for_account_only() {
    let registry = ScannerRegistry::default();
    let account_tasks = insert_pending_tasks(&registry, "acct-1", 2);
    let survivor_tasks = insert_pending_tasks(&registry, "acct-2", 1);

    registry.forget("acct-1");

    {
        let guard = registry.started.lock();
        assert_eq!(guard.len(), 1);
        assert!(guard.contains_key("acct-2"));
    }
    assert_all_cancelled(account_tasks).await;
    assert!(
        !survivor_tasks[0].is_finished(),
        "forget(acct-1) must not abort acct-2"
    );

    assert_eq!(registry.forget_all(), 1);
    assert_all_cancelled(survivor_tasks).await;
}

#[tokio::test]
async fn registry_forget_missing_account_is_noop() {
    let registry = ScannerRegistry::default();
    let mut tasks = insert_pending_tasks(&registry, "acct-1", 1);

    registry.forget("missing");

    {
        let guard = registry.started.lock();
        assert_eq!(guard.len(), 1);
        assert!(guard.contains_key("acct-1"));
    }
    assert!(
        !tasks[0].is_finished(),
        "forget(missing) must not abort existing scanners"
    );

    registry.forget("acct-1");
    assert_cancelled(tasks.pop().expect("task")).await;
}

#[tokio::test]
async fn registry_forget_all_aborts_all_tasks_and_reports_handle_count() {
    let registry = ScannerRegistry::default();
    let task_a = insert_pending_tasks(&registry, "acct-1", 2);
    let task_b = insert_pending_tasks(&registry, "acct-2", 3);

    assert_eq!(registry.forget_all(), 5);

    assert!(registry.started.lock().is_empty());
    assert_all_cancelled(task_a).await;
    assert_all_cancelled(task_b).await;
}

#[tokio::test]
async fn registry_forget_all_is_repeatable_noop_after_drain() {
    let registry = ScannerRegistry::default();
    assert_eq!(registry.forget_all(), 0);

    let tasks = insert_pending_tasks(&registry, "acct-1", 1);
    assert_eq!(registry.forget_all(), 1);
    assert_eq!(registry.forget_all(), 0);

    assert!(registry.started.lock().is_empty());
    assert_all_cancelled(tasks).await;
}
