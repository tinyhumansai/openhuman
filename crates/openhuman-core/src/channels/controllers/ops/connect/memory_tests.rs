use super::*;
use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
use crate::threads::store::{
    ConversationMessage, ConversationMessagePatch, CreateConversationThread,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{
    ItemKind, LearningKind, ListRequest, MemoryEngine, MemoryMeta, MetaFilter, Role, SourceKind,
    StoreItem, Turn,
};

fn conversation(thread: &str, channel: Option<&str>) -> StoreItem {
    let mut meta = MemoryMeta::from_source(SourceKind::Conversation, Some(thread.to_string()));
    meta.thread_id = Some(thread.to_string());
    meta.channel = channel.map(str::to_string);
    StoreItem::Conversation {
        turns: vec![Turn::new(Role::User, format!("hello from {thread}"))],
        meta,
    }
}

fn account_context(workspace: &std::path::Path, profile: &str) -> DispatchCtx {
    let config = Config {
        workspace_dir: workspace.to_path_buf(),
        action_dir: workspace.to_path_buf(),
        config_path: workspace.join("config.toml"),
        ..Config::default()
    };
    DispatchCtx::new(
        CoreContext::for_test(DomainSet::none(), Some(workspace.to_path_buf())).derive_with(
            ContextOverlay::new(config, DomainSet::full(), Default::default()).profile(profile),
        ),
    )
}

async fn use_linked_channel_module(config: &mut Config) {
    config.modules.source = crate::config::schema::ModuleSourceMode::Linked;
    config.modules.allow_download = false;
    let _client = crate::channels::module_fixture::client().await;
}

async fn store_channel_thread(
    ctx: DispatchCtx,
    workspace: &std::path::Path,
    channel: &str,
    message_id: &str,
) -> String {
    #[cfg(feature = "storage-sqlite")]
    crate::storage::local::use_persistent_default_for_test(workspace);
    let params = crate::channels::providers::relay::RelayInboundParams::new(
        channel,
        format!("chat-{message_id}"),
        format!("sender-{message_id}"),
        message_id,
        "hello",
    );
    let identity = crate::modules::channel_controller::prepare_identity(
        &crate::channels::module_fixture::client().await,
        tinychannels_bus::module_inbound::PrepareChannelIdentityRequest {
            channel: channel.to_string(),
            sender: Some(format!("sender-{message_id}")),
            reply_target: Some(format!("chat-{message_id}")),
            thread_ts: None,
        },
    )
    .await
    .expect("TinyChannels module should prepare the canonical thread id");
    let thread_id = identity.thread_id;
    let now = chrono::Utc::now().to_rfc3339();
    crate::threads::store::blocking::ensure_thread(
        ctx.clone(),
        workspace.to_path_buf(),
        CreateConversationThread {
            id: thread_id.clone(),
            title: params.thread_title(),
            created_at: now.clone(),
            parent_thread_id: None,
            labels: Some(vec!["general".into()]),
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .unwrap();
    crate::threads::store::blocking::append_message(
        ctx,
        workspace.to_path_buf(),
        thread_id.clone(),
        ConversationMessage {
            id: format!("user:{message_id}"),
            content: "hello".into(),
            message_type: "text".into(),
            extra_metadata: json!({"scope": "channel", "channel": channel}),
            sender: "user".into(),
            created_at: now,
        },
    )
    .await
    .unwrap();
    thread_id
}

#[tokio::test]
async fn clears_stamped_and_legacy_channel_conversations_without_mutating_legacy_map() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    use_linked_channel_module(&mut config).await;
    let engine = Arc::new(ReferenceEngine::new());
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    let discord_thread = store_channel_thread(
        DispatchCtx::ambient(),
        &config.workspace_dir,
        "discord",
        "discord-current",
    )
    .await;
    store_channel_thread(
        DispatchCtx::ambient(),
        &config.workspace_dir,
        "telegram",
        "telegram-current",
    )
    .await;
    engine
        .store(conversation(&discord_thread, Some("discord")))
        .await
        .unwrap();
    engine
        .store(conversation("discord-legacy", None))
        .await
        .unwrap();
    engine
        .store(StoreItem::learning(
            "durable preference",
            LearningKind::Preference,
            0.9,
            MemoryMeta {
                thread_id: Some(discord_thread.clone()),
                channel: Some("discord".into()),
                ..MemoryMeta::default()
            },
        ))
        .await
        .unwrap();
    engine
        .store(StoreItem::document(
            "durable brain document",
            MemoryMeta {
                thread_id: Some(discord_thread.clone()),
                channel: Some("discord".into()),
                ..MemoryMeta::default()
            },
        ))
        .await
        .unwrap();
    engine
        .store(conversation("telegram-current", Some("telegram")))
        .await
        .unwrap();
    let legacy_file = config.workspace_dir.join("memory/channel_threads.json");
    std::fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
    let legacy_bytes = br#"{"discord":["discord-legacy"]}"#;
    std::fs::write(&legacy_file, legacy_bytes).unwrap();

    assert_eq!(clear_channel_memory(&config, "discord").await.unwrap(), 2);
    assert_eq!(
        engine.len(),
        3,
        "other-channel and non-conversation items remain"
    );
    let remaining = engine
        .list(ListRequest {
            filter: MetaFilter::default(),
            limit: 100,
            cursor: None,
        })
        .await
        .unwrap()
        .items;
    assert!(remaining
        .iter()
        .any(|item| { item.kind == ItemKind::Learning && item.text == "durable preference" }));
    assert!(remaining
        .iter()
        .any(|item| { item.kind == ItemKind::Document && item.text == "durable brain document" }));
    assert!(remaining.iter().any(|item| {
        item.kind == ItemKind::Conversation
            && item.meta.thread_id.as_deref() == Some("telegram-current")
    }));
    assert_eq!(std::fs::read(&legacy_file).unwrap(), legacy_bytes);
}

#[tokio::test]
async fn channel_cleanup_reads_legacy_threads_after_state_migration() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    use_linked_channel_module(&mut config).await;
    let engine = Arc::new(ReferenceEngine::new());
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    engine
        .store(conversation("discord-legacy", None))
        .await
        .unwrap();

    let legacy_file = config.workspace_dir.join("memory/channel_threads.json");
    std::fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
    std::fs::write(&legacy_file, br#"{"discord":["discord-legacy"]}"#).unwrap();
    let imported: Option<BTreeMap<String, BTreeSet<String>>> =
        crate::memory::state_store::load(&config.workspace_dir, "channel_threads").unwrap();
    assert!(imported.is_some());
    std::fs::remove_file(&legacy_file).unwrap();

    assert_eq!(clear_channel_memory(&config, "discord").await.unwrap(), 1);
    assert!(engine.is_empty());
}

#[tokio::test]
async fn stamped_channel_forget_isolated_to_the_configured_account() {
    let current_tmp = tempfile::TempDir::new().unwrap();
    let other_tmp = tempfile::TempDir::new().unwrap();
    let mut current = Config::default();
    current.workspace_dir = current_tmp.path().to_path_buf();
    current.memory.engine.clear();
    use_linked_channel_module(&mut current).await;
    let mut other = Config::default();
    other.workspace_dir = other_tmp.path().to_path_buf();
    other.memory.engine.clear();
    let current_ctx = account_context(&current.workspace_dir, "account-a");
    let other_ctx = account_context(&other.workspace_dir, "account-b");
    let current_thread = store_channel_thread(
        current_ctx.clone(),
        &current.workspace_dir,
        "discord",
        "current-account",
    )
    .await;
    let other_thread = store_channel_thread(
        other_ctx.clone(),
        &other.workspace_dir,
        "discord",
        "other-account",
    )
    .await;

    assert_eq!(
        CoreContext::scope(current_ctx.into_core().unwrap(), async {
            clear_channel_memory(&current, "discord").await.unwrap()
        })
        .await,
        0
    );
    assert_eq!(
        crate::memory::deletion::pending(&current.workspace_dir),
        vec![crate::memory::deletion::PendingDeletion::Thread {
            thread_id: current_thread.clone()
        }]
    );
    assert!(crate::memory::deletion::pending(&other.workspace_dir).is_empty());
    assert_ne!(current_thread, other_thread);
}

#[tokio::test]
async fn missing_or_corrupt_legacy_map_does_not_block_stamped_forget() {
    for corrupt in [false, true] {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = Config::default();
        config.workspace_dir = tmp.path().to_path_buf();
        use_linked_channel_module(&mut config).await;
        let engine = Arc::new(ReferenceEngine::new());
        crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
        let thread_id = store_channel_thread(
            DispatchCtx::ambient(),
            &config.workspace_dir,
            "discord",
            "discord-current",
        )
        .await;
        engine
            .store(conversation(&thread_id, Some("discord")))
            .await
            .unwrap();
        let legacy_file = config.workspace_dir.join("memory/channel_threads.json");
        let corrupt_bytes = b"{not-json";
        if corrupt {
            std::fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
            std::fs::write(&legacy_file, corrupt_bytes).unwrap();
        }

        assert_eq!(clear_channel_memory(&config, "discord").await.unwrap(), 1);
        assert!(engine.is_empty());
        if corrupt {
            assert_eq!(std::fs::read(&legacy_file).unwrap(), corrupt_bytes);
            assert!(!legacy_file.with_extension("json.corrupt").exists());
        } else {
            assert!(!legacy_file.exists());
        }
    }
}

#[tokio::test]
async fn memory_off_queues_channel_threads_from_the_canonical_thread_store() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    config.memory.engine.clear();
    #[cfg(feature = "storage-sqlite")]
    crate::storage::local::use_persistent_default_for_test(&config.workspace_dir);
    use_linked_channel_module(&mut config).await;
    let thread_id = store_channel_thread(
        DispatchCtx::ambient(),
        &config.workspace_dir,
        "discord",
        "discord-offline",
    )
    .await;

    assert_eq!(clear_channel_memory(&config, "discord").await.unwrap(), 0);
    assert_eq!(
        crate::memory::deletion::pending(&config.workspace_dir),
        vec![crate::memory::deletion::PendingDeletion::Thread { thread_id }]
    );
}

#[tokio::test]
async fn reserved_channel_thread_id_survives_metadata_edit_without_prefix_collision() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    config.memory.engine.clear();
    use_linked_channel_module(&mut config).await;
    let ctx = DispatchCtx::ambient();
    let target = store_channel_thread(
        ctx.clone(),
        &config.workspace_dir,
        "discord",
        "stripped-channel-meta",
    )
    .await;
    let collision = store_channel_thread(
        ctx.clone(),
        &config.workspace_dir,
        "discordish",
        "prefix-collision",
    )
    .await;
    for (thread_id, message_id) in [
        (&target, "user:stripped-channel-meta"),
        (&collision, "user:prefix-collision"),
    ] {
        crate::threads::store::blocking::update_message(
            ctx.clone(),
            config.workspace_dir.clone(),
            thread_id.clone(),
            message_id.into(),
            ConversationMessagePatch {
                extra_metadata: Some(json!({"scope": "web"})),
                merge_extra_metadata: None,
            },
        )
        .await
        .unwrap();
    }
    let messages = crate::threads::store::blocking::get_messages(
        ctx,
        config.workspace_dir.clone(),
        target.clone(),
    )
    .await
    .unwrap();
    assert!(messages[0].extra_metadata.get("channel").is_none());

    assert_eq!(clear_channel_memory(&config, "discord").await.unwrap(), 0);
    assert_eq!(
        crate::memory::deletion::pending(&config.workspace_dir),
        vec![crate::memory::deletion::PendingDeletion::Thread { thread_id: target }]
    );
}

#[tokio::test]
async fn offline_snapshot_scans_every_thread_and_message_page() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    config.memory.engine.clear();
    use_linked_channel_module(&mut config).await;
    let workspace = config.workspace_dir.clone();
    let ctx = DispatchCtx::ambient();
    let params = crate::channels::providers::relay::RelayInboundParams::new(
        "discord",
        "chat-old-channel-message",
        "sender-old-channel-message",
        "old-channel-message",
        "hello from discord",
    );
    let target_thread = "zz-legacy-channel-thread".to_owned();
    crate::threads::store::blocking::ensure_thread(
        ctx.clone(),
        workspace.clone(),
        CreateConversationThread {
            id: target_thread.clone(),
            title: params.thread_title(),
            created_at: "2026-01-01T00:00:00Z".into(),
            parent_thread_id: None,
            labels: Some(vec!["general".into()]),
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .unwrap();

    for index in 0..64 {
        crate::threads::store::blocking::append_message(
            ctx.clone(),
            workspace.clone(),
            target_thread.clone(),
            ConversationMessage {
                id: format!("web-before-{index}"),
                content: "unrelated web row before channel marker".into(),
                message_type: "text".into(),
                extra_metadata: json!({"scope": "web"}),
                sender: "user".into(),
                created_at: "2099-01-01T00:00:00Z".into(),
            },
        )
        .await
        .unwrap();
    }
    crate::threads::store::blocking::append_message(
        ctx.clone(),
        workspace.clone(),
        target_thread.clone(),
        ConversationMessage {
            id: "user:old-channel-message".into(),
            content: "hello from discord".into(),
            message_type: "text".into(),
            extra_metadata: json!({"scope": "channel", "channel": "discord"}),
            sender: "user".into(),
            created_at: "2099-01-01T00:00:00Z".into(),
        },
    )
    .await
    .unwrap();
    crate::threads::store::blocking::append_message(
        ctx.clone(),
        workspace.clone(),
        target_thread.clone(),
        ConversationMessage {
            id: "web-after-channel".into(),
            content: "later unrelated web row".into(),
            message_type: "text".into(),
            extra_metadata: json!({"scope": "web"}),
            sender: "user".into(),
            created_at: "2099-01-01T00:00:00Z".into(),
        },
    )
    .await
    .unwrap();
    for index in 0..65 {
        crate::threads::store::blocking::ensure_thread(
            ctx.clone(),
            workspace.clone(),
            CreateConversationThread {
                id: format!("web-thread-{index}"),
                title: format!("web {index}"),
                created_at: "2099-01-01T00:00:00Z".into(),
                parent_thread_id: None,
                labels: None,
                personality_id: None,
                working_dir: None,
            },
        )
        .await
        .unwrap();
    }

    let threads = crate::threads::store::blocking::list_threads(ctx.clone(), workspace.clone())
        .await
        .unwrap();
    assert_eq!(threads.len(), 66);
    assert!(threads[..64]
        .iter()
        .all(|thread| thread.id != target_thread));
    assert!(
        threads
            .iter()
            .position(|thread| thread.id == target_thread)
            .unwrap()
            >= 64
    );

    let messages = crate::threads::store::blocking::get_messages(
        ctx.clone(),
        workspace.clone(),
        target_thread.clone(),
    )
    .await
    .unwrap();
    assert_eq!(messages.len(), 66);
    assert!(messages[..64]
        .iter()
        .all(|message| message.extra_metadata.get("channel").is_none()));
    assert_eq!(messages[64].extra_metadata["channel"], "discord");
    assert!(messages[65].extra_metadata.get("channel").is_none());
    assert!(
        !workspace.join("memory/channel_threads.json").exists(),
        "the historical fallback fixture must not depend on the legacy map"
    );

    assert_eq!(clear_channel_memory(&config, "discord").await.unwrap(), 0);
    assert_eq!(
        crate::memory::deletion::pending(&config.workspace_dir),
        vec![crate::memory::deletion::PendingDeletion::Thread {
            thread_id: target_thread
        }]
    );
}

#[tokio::test]
async fn filter_failure_is_visible_and_queued_snapshot_does_not_catch_reconnected_threads() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    use_linked_channel_module(&mut config).await;
    let old_thread = store_channel_thread(
        DispatchCtx::ambient(),
        &config.workspace_dir,
        "discord",
        "before-disconnect",
    )
    .await;
    crate::memory::test_fixtures::RefusingEngine::with(tinymemory_api::Error::Engine(
        "forget failed".into(),
    ))
    .bind(&config);

    let error = clear_channel_memory(&config, "discord")
        .await
        .expect_err("a failed filter forget stays visible to disconnect");
    assert!(error.to_string().contains("clear_channel_memory"));
    assert_eq!(
        crate::memory::deletion::pending(&config.workspace_dir),
        vec![crate::memory::deletion::PendingDeletion::Thread {
            thread_id: old_thread.clone()
        }]
    );

    let new_thread = store_channel_thread(
        DispatchCtx::ambient(),
        &config.workspace_dir,
        "discord",
        "after-reconnect",
    )
    .await;
    let engine = Arc::new(ReferenceEngine::new());
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    engine
        .store(conversation(&old_thread, Some("discord")))
        .await
        .unwrap();
    engine
        .store(conversation(&new_thread, Some("discord")))
        .await
        .unwrap();

    assert_eq!(crate::memory::deletion::drain(&config).await, 1);
    assert_eq!(engine.len(), 1);
    let remaining = crate::memory::test_fixtures::stored(
        &engine,
        tinymemory_api::MetaFilter::kinds([tinymemory_api::ItemKind::Conversation]),
    )
    .await;
    assert_eq!(
        remaining[0].meta.thread_id.as_deref(),
        Some(new_thread.as_str())
    );
}

#[tokio::test]
async fn memory_off_clears_nothing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    config.memory.engine = String::new();
    use_linked_channel_module(&mut config).await;
    assert_eq!(clear_channel_memory(&config, "discord").await.unwrap(), 0);
}
